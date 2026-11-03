#!/bin/bash
# Create an admin for the Rust API (everyone else is created from the Users page).
# Usage: dev-create-superuser.sh [username] [password] [--email EMAIL]
# Defaults: username=admin, password=1234

USERNAME="${1:-admin}"
PASSWORD="${2:-1234}"
[[ $# -ge 2 ]] && shift 2
EXTRA_ARGS=("$@")

docker exec -it d-gui-manager-backend /app/dev-dock-manager-api create-user "$USERNAME" "$PASSWORD" --staff "${EXTRA_ARGS[@]}"
