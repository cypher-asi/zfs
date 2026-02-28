#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/lib.sh"
load_state

require_vars SSH_PRIVATE_KEY

: "${EC2_INSTANCE_ID:?Missing EC2_INSTANCE_ID in .state. Run 03-launch-instance.sh first.}"
: "${EC2_PUBLIC_IP:?Missing EC2_PUBLIC_IP in .state. Run 03-launch-instance.sh first.}"

KEY_PATH="${SSH_PRIVATE_KEY/#\~/${HOME}}"
REMOTE="${EC2_SSH_USER}@${EC2_PUBLIC_IP}"

echo "Waiting for SSH on ${EC2_PUBLIC_IP}..."
for _ in {1..40}; do
  if ssh -i "${KEY_PATH}" -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5 "${REMOTE}" "echo ok" >/dev/null 2>&1; then
    break
  fi
  sleep 3
done

echo "Installing service..."
ssh -i "${KEY_PATH}" -o StrictHostKeyChecking=accept-new "${REMOTE}" "sudo bash -s" <<EOF
set -euo pipefail
test -f "${RELAY_BUILD_DIR}/target/release/grid-relayd"
sudo useradd --system --no-create-home --shell /usr/sbin/nologin grid 2>/dev/null || true
sudo install -m 0755 "${RELAY_BUILD_DIR}/target/release/grid-relayd" /usr/local/bin/grid-relayd
sudo tee /etc/systemd/system/grid-relayd.service >/dev/null <<UNIT
[Unit]
Description=GRID relay service
After=network-online.target
Wants=network-online.target

[Service]
User=grid
Group=grid
Environment=GRID_RELAY_LISTEN=/ip4/0.0.0.0/tcp/${RELAY_PORT}
Environment=GRID_RELAY_LOG=${RELAY_LOG}
Environment=GRID_RELAY_MAX_RESERVATIONS=${RELAY_MAX_RESERVATIONS}
Environment=GRID_RELAY_MAX_CIRCUITS=${RELAY_MAX_CIRCUITS}
ExecStart=/usr/local/bin/grid-relayd
Restart=always
RestartSec=3
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
UNIT
sudo systemctl daemon-reload
sudo systemctl enable --now grid-relayd
sudo systemctl --no-pager --full status grid-relayd
EOF

echo "Relay service deployed to ${EC2_PUBLIC_IP}"
