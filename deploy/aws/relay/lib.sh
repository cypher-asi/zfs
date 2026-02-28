#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STATE_FILE="${SCRIPT_DIR}/.state"
ENV_FILE="${SCRIPT_DIR}/.env"

if [[ -f "${ENV_FILE}" ]]; then
  # shellcheck disable=SC1090
  source "${ENV_FILE}"
fi

: "${EC2_INSTANCE_TYPE:=t3.small}"
: "${EC2_SSH_USER:=ubuntu}"
: "${RELAY_NAME:=grid-relayd}"
: "${RELAY_PORT:=3691}"
: "${RELAY_LOG:=info}"
: "${RELAY_MAX_RESERVATIONS:=256}"
: "${RELAY_MAX_CIRCUITS:=512}"
: "${GRID_REPO_URL:=https://github.com/cypher-asi/the-grid}"
: "${GRID_REPO_REF:=master}"
: "${RELAY_BUILD_DIR:=/opt/the-grid}"

AWS_ARGS=()
if [[ -n "${AWS_REGION:-}" ]]; then
  AWS_ARGS+=(--region "${AWS_REGION}")
fi
if [[ -n "${AWS_PROFILE:-}" ]]; then
  AWS_ARGS+=(--profile "${AWS_PROFILE}")
fi

aws_cmd() {
  aws "${AWS_ARGS[@]}" "$@"
}

save_state() {
  local key="$1"
  local value="$2"
  touch "${STATE_FILE}"
  if [[ -f "${STATE_FILE}" ]] && grep -q "^${key}=" "${STATE_FILE}"; then
    sed -i.bak "s#^${key}=.*#${key}=${value}#" "${STATE_FILE}" && rm -f "${STATE_FILE}.bak"
  else
    echo "${key}=${value}" >> "${STATE_FILE}"
  fi
}

load_state() {
  if [[ -f "${STATE_FILE}" ]]; then
    # shellcheck disable=SC1090
    source "${STATE_FILE}"
  fi
}

require_cmd() {
  local cmd="$1"
  if ! command -v "${cmd}" >/dev/null 2>&1; then
    echo "Missing required command: ${cmd}" >&2
    exit 1
  fi
}

require_vars() {
  local var_name
  for var_name in "$@"; do
    if [[ -z "${!var_name:-}" ]]; then
      echo "Missing required setting: ${var_name} (set it in ${ENV_FILE})" >&2
      exit 1
    fi
  done
}

is_empty_aws_value() {
  local value="${1:-}"
  [[ -z "${value}" || "${value}" == "None" || "${value}" == "null" ]]
}

resolve_vpc_id() {
  if [[ -n "${EC2_VPC_ID:-}" ]]; then
    echo "${EC2_VPC_ID}"
    return 0
  fi
  local detected
  detected="$(aws_cmd ec2 describe-vpcs \
    --filters Name=isDefault,Values=true \
    --query 'Vpcs[0].VpcId' \
    --output text)"
  if is_empty_aws_value "${detected}"; then
    echo "Unable to auto-detect default VPC. Set EC2_VPC_ID in ${ENV_FILE}." >&2
    exit 1
  fi
  echo "${detected}"
}

resolve_subnet_id() {
  local vpc_id="$1"
  if [[ -n "${EC2_SUBNET_ID:-}" ]]; then
    echo "${EC2_SUBNET_ID}"
    return 0
  fi

  local detected
  detected="$(aws_cmd ec2 describe-subnets \
    --filters Name=vpc-id,Values="${vpc_id}" Name=default-for-az,Values=true \
    --query 'Subnets[0].SubnetId' \
    --output text)"
  if is_empty_aws_value "${detected}"; then
    detected="$(aws_cmd ec2 describe-subnets \
      --filters Name=vpc-id,Values="${vpc_id}" \
      --query 'Subnets[0].SubnetId' \
      --output text)"
  fi
  if is_empty_aws_value "${detected}"; then
    echo "Unable to auto-detect subnet in VPC ${vpc_id}. Set EC2_SUBNET_ID in ${ENV_FILE}." >&2
    exit 1
  fi
  echo "${detected}"
}

resolve_ami_id() {
  if [[ -n "${EC2_AMI_ID:-}" ]]; then
    echo "${EC2_AMI_ID}"
    return 0
  fi

  local ami
  ami="$(aws_cmd ssm get-parameter \
    --name /aws/service/canonical/ubuntu/server/24.04/stable/current/amd64/hvm/ebs-gp3/ami-id \
    --query 'Parameter.Value' \
    --output text 2>/dev/null || true)"
  if is_empty_aws_value "${ami}"; then
    ami="$(aws_cmd ssm get-parameter \
      --name /aws/service/canonical/ubuntu/server/22.04/stable/current/amd64/hvm/ebs-gp3/ami-id \
      --query 'Parameter.Value' \
      --output text 2>/dev/null || true)"
  fi
  if is_empty_aws_value "${ami}"; then
    ami="$(aws_cmd ec2 describe-images \
      --owners 099720109477 \
      --filters \
      Name=name,Values=ubuntu/images/hvm-ssd-gp3/ubuntu-noble-24.04-amd64-server-* \
      Name=state,Values=available \
      Name=architecture,Values=x86_64 \
      Name=virtualization-type,Values=hvm \
      --query 'reverse(sort_by(Images,&CreationDate))[0].ImageId' \
      --output text 2>/dev/null || true)"
  fi
  if is_empty_aws_value "${ami}"; then
    ami="$(aws_cmd ec2 describe-images \
      --owners 099720109477 \
      --filters \
      Name=name,Values=ubuntu/images/hvm-ssd-gp3/ubuntu-jammy-22.04-amd64-server-* \
      Name=state,Values=available \
      Name=architecture,Values=x86_64 \
      Name=virtualization-type,Values=hvm \
      --query 'reverse(sort_by(Images,&CreationDate))[0].ImageId' \
      --output text 2>/dev/null || true)"
  fi
  if is_empty_aws_value "${ami}"; then
    echo "Unable to auto-detect Ubuntu AMI from SSM or describe-images. Set EC2_AMI_ID in ${ENV_FILE}." >&2
    exit 1
  fi
  echo "${ami}"
}

ensure_keypair() {
  if [[ -n "${EC2_KEY_NAME:-}" && -n "${SSH_PRIVATE_KEY:-}" ]]; then
    return 0
  fi

  local key_dir="${SCRIPT_DIR}/.keys"
  mkdir -p "${key_dir}"

  local generated_name="${RELAY_NAME}-$(date +%s)"
  local key_path="${key_dir}/${generated_name}.pem"

  echo "Creating temporary EC2 key pair: ${generated_name}"
  aws_cmd ec2 create-key-pair \
    --key-name "${generated_name}" \
    --query 'KeyMaterial' \
    --output text > "${key_path}"
  chmod 600 "${key_path}"

  EC2_KEY_NAME="${generated_name}"
  SSH_PRIVATE_KEY="${key_path}"
  save_state "EC2_KEY_NAME" "${EC2_KEY_NAME}"
  save_state "SSH_PRIVATE_KEY" "${SSH_PRIVATE_KEY}"
}

repo_root() {
  git -C "${SCRIPT_DIR}" rev-parse --show-toplevel
}
