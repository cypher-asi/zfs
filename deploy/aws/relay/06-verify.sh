#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/lib.sh"
load_state

require_vars SSH_PRIVATE_KEY

: "${EC2_PUBLIC_IP:?Missing EC2_PUBLIC_IP in .state. Run 03-launch-instance.sh first.}"

KEY_PATH="${SSH_PRIVATE_KEY/#\~/${HOME}}"
REMOTE="${EC2_SSH_USER}@${EC2_PUBLIC_IP}"

echo "Checking service and listening socket..."
ssh -i "${KEY_PATH}" -o StrictHostKeyChecking=accept-new "${REMOTE}" "sudo systemctl is-active grid-relayd"
ssh -i "${KEY_PATH}" -o StrictHostKeyChecking=accept-new "${REMOTE}" "sudo ss -ltnp | grep :${RELAY_PORT} || true"

echo "Fetching recent logs..."
ssh -i "${KEY_PATH}" -o StrictHostKeyChecking=accept-new "${REMOTE}" \
  "sudo journalctl -u grid-relayd -n 80 --no-pager"

echo
echo "Relay endpoint:"
echo "/ip4/${EC2_PUBLIC_IP}/tcp/${RELAY_PORT}"
echo
echo "To include peer ID, read local_peer_id from logs and append /p2p/<peer-id>."
