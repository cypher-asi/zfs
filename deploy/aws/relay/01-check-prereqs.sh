#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/lib.sh"

echo "Checking local prerequisites..."
require_cmd aws
require_cmd ssh

if [[ ! -f "${ENV_FILE}" ]]; then
  echo "Note: ${ENV_FILE} not found. Continuing with auto-discovery defaults." >&2
fi

if [[ -n "${SSH_PRIVATE_KEY:-}" ]] && [[ ! -f "${SSH_PRIVATE_KEY/#\~/${HOME}}" ]]; then
  echo "SSH key not found at ${SSH_PRIVATE_KEY}" >&2
  exit 1
fi

echo "Validating AWS caller identity..."
aws_cmd sts get-caller-identity >/dev/null

echo "All prerequisites look good."
