#!/usr/bin/env bash
set -euo pipefail

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required for the Chariox Docker lab smoke test" >&2
  exit 127
fi

compose=(docker compose -f docker/lab/docker-compose.yml)

"${compose[@]}" up -d --build relay worker-a worker-b

for container in chariox-relay chariox-worker-a chariox-worker-b; do
  running="$(docker inspect -f '{{.State.Running}}' "$container")"
  if [[ "$running" != "true" ]]; then
    echo "$container is not running" >&2
    docker logs --tail 120 "$container" >&2 || true
    exit 1
  fi
done

docker exec chariox-worker-a sh -lc '
  command -v chariox >/dev/null &&
  command -v chariox-kernel >/dev/null &&
  command -v chariox-relay >/dev/null &&
  command -v bun >/dev/null &&
  test "$CHARIOX_MACHINE_ALIAS" = "chariox-worker-a"
'

docker exec chariox-worker-b sh -lc '
  command -v chariox >/dev/null &&
  command -v chariox-kernel >/dev/null &&
  command -v chariox-relay >/dev/null &&
  command -v bun >/dev/null &&
  test "$CHARIOX_MACHINE_ALIAS" = "chariox-worker-b"
'

if ! docker logs --tail 120 chariox-relay 2>&1 | grep -q "chariox relay listening"; then
  echo "relay did not print its listening banner" >&2
  docker logs --tail 120 chariox-relay >&2 || true
  exit 1
fi

for container in chariox-worker-a chariox-worker-b; do
  if ! docker exec "$container" sh -lc 'pgrep -x chariox-kernel >/dev/null'; then
    echo "$container is running but chariox-kernel is not alive" >&2
    docker logs --tail 160 "$container" >&2 || true
    exit 1
  fi

  if ! docker exec "$container" sh -lc 'grep -R "ready on machine" "$HOME/.local/state/chariox/logs" >/dev/null 2>&1'; then
    echo "$container did not record daemon readiness" >&2
    docker exec "$container" sh -lc 'cat "$HOME"/.local/state/chariox/logs/*.ndjson 2>/dev/null | tail -n 160' >&2 || true
    exit 1
  fi
done

cat <<'MSG'
Chariox Docker lab smoke passed.

Host CLI follow-up:
  /relay use ws://127.0.0.1:43150 local-lab
  /machine list

Provider follow-up:
  docker exec -it chariox-worker-a zsh
  docker exec -it chariox-worker-b zsh
  # install and log into provider CLIs inside each worker
MSG
