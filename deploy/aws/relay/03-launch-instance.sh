#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/lib.sh"
load_state

EC2_AMI_ID="$(resolve_ami_id)"
EC2_VPC_ID="${EC2_VPC_ID:-$(resolve_vpc_id)}"
EC2_SUBNET_ID="$(resolve_subnet_id "${EC2_VPC_ID}")"
ensure_keypair

save_state "EC2_AMI_ID" "${EC2_AMI_ID}"
save_state "EC2_VPC_ID" "${EC2_VPC_ID}"
save_state "EC2_SUBNET_ID" "${EC2_SUBNET_ID}"

if [[ -z "${EC2_SECURITY_GROUP_ID:-}" ]]; then
  echo "EC2_SECURITY_GROUP_ID is missing. Run 02-create-security-group.sh first." >&2
  exit 1
fi

if [[ -n "${EC2_INSTANCE_ID:-}" ]]; then
  echo "Instance already recorded in state: ${EC2_INSTANCE_ID}"
else
  echo "Launching EC2 instance..."
  EC2_INSTANCE_ID="$(aws_cmd ec2 run-instances \
    --image-id "${EC2_AMI_ID}" \
    --instance-type "${EC2_INSTANCE_TYPE}" \
    --key-name "${EC2_KEY_NAME}" \
    --subnet-id "${EC2_SUBNET_ID}" \
    --security-group-ids "${EC2_SECURITY_GROUP_ID}" \
    --tag-specifications "ResourceType=instance,Tags=[{Key=Name,Value=${RELAY_NAME}}]" \
    --query 'Instances[0].InstanceId' \
    --output text)"
  save_state "EC2_INSTANCE_ID" "${EC2_INSTANCE_ID}"
fi

echo "Waiting for instance to be running: ${EC2_INSTANCE_ID}"
aws_cmd ec2 wait instance-running --instance-ids "${EC2_INSTANCE_ID}"

EC2_PUBLIC_IP="$(aws_cmd ec2 describe-instances \
  --instance-ids "${EC2_INSTANCE_ID}" \
  --query 'Reservations[0].Instances[0].PublicIpAddress' \
  --output text)"

save_state "EC2_PUBLIC_IP" "${EC2_PUBLIC_IP}"
echo "Instance running with public IP: ${EC2_PUBLIC_IP}"
