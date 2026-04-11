#!/usr/bin/env bash
set -euo pipefail

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required for the Arroba Docker lab smoke test" >&2
  exit 127
fi

compose=(docker compose -f docker/lab/docker-compose.yml)

"${compose[@]}" up -d --build relay worker-a worker-b

for container in arroba-relay arroba-worker-a arroba-worker-b; do
  running="$(docker inspect -f '{{.State.Running}}' "$container")"
  if [[ "$running" != "true" ]]; then
    echo "$container is not running" >&2
    docker logs --tail 120 "$container" >&2 || true
    exit 1
  fi
done

docker exec arroba-worker-a sh -lc '
  command -v arroba >/dev/null &&
  command -v arroba-daemon >/dev/null &&
  command -v arroba-relay >/dev/null &&
  command -v bun >/dev/null &&
  test "$ARROBA_MACHINE_ALIAS" = "arroba-worker-a"
'

docker exec arroba-worker-b sh -lc '
  command -v arroba >/dev/null &&
  command -v arroba-daemon >/dev/null &&
  command -v arroba-relay >/dev/null &&
  command -v bun >/dev/null &&
  test "$ARROBA_MACHINE_ALIAS" = "arroba-worker-b"
'

if ! docker logs --tail 120 arroba-relay 2>&1 | grep -q "arroba relay listening"; then
  echo "relay did not print its listening banner" >&2
  docker logs --tail 120 arroba-relay >&2 || true
  exit 1
fi

for container in arroba-worker-a arroba-worker-b; do
  if ! docker logs --tail 160 "$container" 2>&1 | grep -q "ready on machine"; then
    echo "$container did not print daemon readiness" >&2
    docker logs --tail 160 "$container" >&2 || true
    exit 1
  fi
done

cat <<'MSG'
Arroba Docker lab smoke passed.

Host CLI follow-up:
  /relay use ws://127.0.0.1:43150 local-lab
  /machine list

Provider follow-up:
  docker exec -it arroba-worker-a zsh
  docker exec -it arroba-worker-b zsh
  # install and log into provider CLIs inside each worker
MSG
