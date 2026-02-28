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

echo "Building grid-relayd on remote host from ${GRID_REPO_URL} (${GRID_REPO_REF})..."
ssh -i "${KEY_PATH}" -o StrictHostKeyChecking=accept-new "${REMOTE}" "bash -s" <<EOF
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive
sudo apt-get update -y
sudo apt-get install -y build-essential cmake pkg-config clang libclang-dev git curl

if ! command -v cargo >/dev/null 2>&1; then
  curl https://sh.rustup.rs -sSf | sh -s -- -y
fi

if [[ -f "\${HOME}/.cargo/env" ]]; then
  # shellcheck disable=SC1091
  source "\${HOME}/.cargo/env"
fi

sudo mkdir -p "${RELAY_BUILD_DIR}"
sudo chown "\${USER}":"\${USER}" "${RELAY_BUILD_DIR}"

if [[ -d "${RELAY_BUILD_DIR}/.git" ]]; then
  git -C "${RELAY_BUILD_DIR}" fetch --depth 1 origin "${GRID_REPO_REF}"
  git -C "${RELAY_BUILD_DIR}" checkout -f FETCH_HEAD
else
  git clone --depth 1 --branch "${GRID_REPO_REF}" "${GRID_REPO_URL}" "${RELAY_BUILD_DIR}"
fi

cargo build --manifest-path "${RELAY_BUILD_DIR}/Cargo.toml" --release -p grid-relayd
test -f "${RELAY_BUILD_DIR}/target/release/grid-relayd"
EOF

echo "Remote build completed: ${RELAY_BUILD_DIR}/target/release/grid-relayd"
