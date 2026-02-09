#!/bin/bash
# Run backend in foreground with RUST_LOG=debug in a one-off container (same env/volumes as main backend)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}/.." && docker compose run --rm backend sh -c 'RUST_LOG=debug RUST_BACKTRACE=1 /app/dev-dock-manager-api'
