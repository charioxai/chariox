#!/bin/bash
# Remove only the amunblock-fa project (containers, network, volumes) and its generated state.
set -euo pipefail
cd "$(dirname "$0")"; . ./env.sh
FA_DB_PASSWORD=x docker compose -f compose.yml down -v --remove-orphans
rm -rf "$FA_STATE_DIR"
