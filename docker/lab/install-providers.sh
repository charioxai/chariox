#!/usr/bin/env bash
set -euo pipefail

worker="${1:-arroba-worker-a}"

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required to install providers in a lab worker" >&2
  exit 127
fi

if ! docker inspect "$worker" >/dev/null 2>&1; then
  echo "container $worker was not found; start the lab first" >&2
  echo "  docker compose -f docker/lab/docker-compose.yml up -d relay worker-a worker-b" >&2
  exit 1
fi

docker_exec=(docker exec)
if [[ -t 0 ]]; then
  docker_exec+=(--interactive --tty)
fi
docker_exec+=("$worker" sh -lc)

"${docker_exec[@]}" '
  set -eu
  npm install -g @openai/codex opencode-ai
  echo "installed provider CLIs:"
  command -v codex && codex --version || true
  command -v opencode && opencode --version || true
'

cat <<MSG
Provider CLIs are installed in $worker.

Next manual steps inside the worker:
  docker exec -it $worker zsh
  codex login
  opencode auth login

If a provider prints a browser URL, open it in the host browser.
If it uses a localhost callback, use the mapped lab port ranges or provider-specific fixed-port options.
MSG
