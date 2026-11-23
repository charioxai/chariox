#!/usr/bin/env bash
set -euo pipefail

lab="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$lab/../../scripts/lib/drill-docker-owner.sh"
drill_docker_environment detached docker-lab
exec docker compose -f "$lab/docker-compose.yml" up -d "$@" relay worker-a worker-b
