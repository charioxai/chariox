#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"; . ./env.sh
docker ps --filter "label=com.docker.compose.project=$COMPOSE_PROJECT_NAME" --format '{{.Names}} {{.Status}}'
