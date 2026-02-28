#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/lib.sh"
load_state

EC2_VPC_ID="$(resolve_vpc_id)"
save_state "EC2_VPC_ID" "${EC2_VPC_ID}"

if [[ -n "${EC2_SECURITY_GROUP_ID:-}" ]]; then
  echo "Using existing security group: ${EC2_SECURITY_GROUP_ID}"
  save_state "EC2_SECURITY_GROUP_ID" "${EC2_SECURITY_GROUP_ID}"
  exit 0
fi

SG_NAME="${RELAY_NAME}-sg"
echo "Creating security group ${SG_NAME} in ${EC2_VPC_ID}..."
SG_ID="$(aws_cmd ec2 create-security-group \
  --group-name "${SG_NAME}" \
  --description "Security group for ${RELAY_NAME}" \
  --vpc-id "${EC2_VPC_ID}" \
  --query 'GroupId' \
  --output text)"

echo "Authorizing inbound SSH (22/tcp) and relay (${RELAY_PORT}/tcp)..."
aws_cmd ec2 authorize-security-group-ingress \
  --group-id "${SG_ID}" \
  --ip-permissions \
  "IpProtocol=tcp,FromPort=22,ToPort=22,IpRanges=[{CidrIp=0.0.0.0/0}]" \
  "IpProtocol=tcp,FromPort=${RELAY_PORT},ToPort=${RELAY_PORT},IpRanges=[{CidrIp=0.0.0.0/0}]"

save_state "EC2_SECURITY_GROUP_ID" "${SG_ID}"
echo "Created security group: ${SG_ID}"
