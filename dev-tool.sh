#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPTS_DIR="${SCRIPT_DIR}/scripts"

usage() {
  cat << 'EOF'
Dev Dock Manager - Development tool script

Usage: ./dev-tool.sh [options] <subcommand> [subcommand args...]

Options:
  -h, --help    Show this help

Subcommands:
  backend-debug     Run backend in foreground with RUST_LOG=debug (one-off container)
  bash              Open a bash shell in the backend container (d-gui-manager-backend)
  create-superuser  Create a user for login (default: admin / 1234; pass --email, --staff as extra args)
  logs              Follow backend container logs

Examples:
  ./dev-tool.sh --help
  ./dev-tool.sh bash
  ./dev-tool.sh backend-debug
  ./dev-tool.sh create-superuser
  ./dev-tool.sh create-superuser admin mypass --staff
  ./dev-tool.sh logs
EOF
}

run_script() {
  local name="$1"
  local script="${SCRIPTS_DIR}/dev-${name}.sh"
  if [[ ! -x "$script" ]]; then
    echo "Error: script not found or not executable: $script" >&2
    exit 1
  fi
  exec "$script" "${@:2}"
}

case "${1:-}" in
  -h|--help)
    usage
    exit 0
    ;;
  backend-debug)
    run_script "backend-debug" "${@:2}"
    ;;
  bash)
    run_script "bash" "${@:2}"
    ;;
  create-superuser)
    echo "Create backend user (press Enter to use default)."
    read -r -p "Username [admin]: " input_username
    read -r -s -p "Password [1234]: " input_password
    echo
    run_script "create-superuser" "${input_username:-admin}" "${input_password:-1234}" "${@:2}"
    ;;
  logs)
    run_script "logs" "${@:2}"
    ;;
  "")
    echo "Please specify a subcommand. Use ./dev-tool.sh --help for usage."
    exit 1
    ;;
  *)
    echo "Unknown subcommand: $1"
    echo "Use ./dev-tool.sh --help to see available commands."
    exit 1
    ;;
esac
