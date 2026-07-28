#!/usr/bin/env bash
set -euo pipefail

repository_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
deployment_dir="${repository_dir}/deploy/event-publication"
services_root="$(cd "${repository_dir}/.." && pwd)"
aeds_repository="${services_root}/arroba-aeds"
export ARROBA_BUILD_REVISION="${ARROBA_BUILD_REVISION:-$(git -C "${repository_dir}" rev-parse HEAD)-dirty}"
"${deployment_dir}/prepare-secrets.sh"
set -a
# shellcheck disable=SC1091
source "${deployment_dir}/.env"
set +a

compose=(
  docker compose
  --project-directory "${deployment_dir}"
  -f "${deployment_dir}/compose.yaml"
)
export COMPOSE_PARALLEL_LIMIT=1
"${compose[@]}" up --build -d --wait aeds

start_aegs() {
  "${compose[@]}" build "$1"
  "${compose[@]}" up --no-build --no-deps -d --wait "$1"
}

stop_aegs() {
  "${compose[@]}" stop "$1"
  "${compose[@]}" rm --force "$1"
}

kernel_token="$(tr -d '\r\n' < "${deployment_dir}/secrets/local-kernel-token")"
dummy_management_token="$(tr -d '\r\n' < "${deployment_dir}/secrets/dummy-aegs-management-token")"
start_aegs dummy-aegs
authorization="$(
  curl --fail --silent \
    -H "authorization: Bearer ${dummy_management_token}" \
    -H "content-type: application/json" \
    --data '{"generator_id":"dev.arroba.dummy"}' \
    "http://127.0.0.1:${ARROBA_DUMMY_AEGS_PORT}/v1/authorizations"
)"
if ! grep -q '"connection_id":"local-dummy"' <<< "${authorization}"; then
  printf 'Dummy AEGS authorization did not return its opaque connection\n' >&2
  exit 1
fi
resources="$(
  curl --fail --silent \
    -H "authorization: Bearer ${dummy_management_token}" \
    -H "content-type: application/json" \
    --data '{"generator_id":"dev.arroba.dummy","connection_id":"local-dummy","limit":20}' \
    "http://127.0.0.1:${ARROBA_DUMMY_AEGS_PORT}/v1/resources/query"
)"
if ! grep -q '"connection_scope":"default"' <<< "${resources}"; then
  printf 'Dummy AEGS resource enumeration did not return its provider scope\n' >&2
  exit 1
fi

ARROBA_DRILL_KERNEL_TOKEN="${kernel_token}" \
ARROBA_DRILL_AEDS_URL="ws://127.0.0.1:${ARROBA_AEDS_KERNEL_PORT}" \
ARROBA_DRILL_AEGS_URL="http://127.0.0.1:${ARROBA_DUMMY_AEGS_PORT}/v1/emit" \
  cargo run --quiet --manifest-path "${aeds_repository}/Cargo.toml" \
    --example deployment_drill
stop_aegs dummy-aegs

provider_specs=(
  "github-aegs|github|GITHUB|ARROBA_GITHUB_AEGS_PORT|github"
  "jira-aegs|jira-cloud|JIRA|ARROBA_JIRA_AEGS_PORT|jira"
  "linear-aegs|linear|LINEAR|ARROBA_LINEAR_AEGS_PORT|linear"
  "gitlab-aegs|gitlab|GITLAB|ARROBA_GITLAB_AEGS_PORT|gitlab"
  "sentry-aegs|sentry|SENTRY|ARROBA_SENTRY_AEGS_PORT|sentry"
  "slack-aegs|slack|SLACK|ARROBA_SLACK_AEGS_PORT|slack"
)
for provider_spec in "${provider_specs[@]}"; do
  IFS="|" read -r service fixture prefix port_variable secret_stem <<< "${provider_spec}"
  port="${!port_variable}"
  start_aegs "${service}"
  env \
    ARROBA_DRILL_PROVIDERS="${fixture}" \
    ARROBA_DRILL_KERNEL_TOKEN="${kernel_token}" \
    ARROBA_DRILL_AEDS_URL="ws://127.0.0.1:${ARROBA_AEDS_KERNEL_PORT}" \
    "ARROBA_DRILL_${prefix}_AEGS_URL=http://127.0.0.1:${port}" \
    "ARROBA_DRILL_${prefix}_MANAGEMENT_TOKEN=$(tr -d '\r\n' < "${deployment_dir}/secrets/${secret_stem}-aegs-management-token")" \
    "ARROBA_DRILL_${prefix}_WEBHOOK_SECRET=$(tr -d '\r\n' < "${deployment_dir}/secrets/${secret_stem}-aegs-webhook-secret")" \
    cargo run --quiet --manifest-path "${aeds_repository}/Cargo.toml" \
      --example first_wave_provider_drill
  stop_aegs "${service}"
done

curl --fail --silent "http://127.0.0.1:${ARROBA_AEDS_PRODUCER_PORT}/metrics"
"${compose[@]}" restart aeds
ready=false
for attempt in {1..30}; do
  if curl --fail --silent "http://127.0.0.1:${ARROBA_AEDS_PRODUCER_PORT}/readyz"; then
    ready=true
    break
  fi
  sleep 1
done
if [[ "${ready}" != "true" ]]; then
  printf 'AEDS did not become ready after restart\n' >&2
  exit 1
fi
metrics="$(curl --fail --silent "http://127.0.0.1:${ARROBA_AEDS_PRODUCER_PORT}/metrics")"
if ! grep -q '^arroba_aeds_active_routes 1$' <<< "${metrics}"; then
  printf 'AEDS route state did not survive restart\n%s\n' "${metrics}" >&2
  exit 1
fi
printf '%s\n' "${metrics}"

backup_path="$(
  ARROBA_AEDS_COMPOSE_FILE="${deployment_dir}/compose.yaml" \
  ARROBA_AEDS_COMPOSE_PROJECT_DIRECTORY="${deployment_dir}" \
  ARROBA_AEDS_COMPOSE_PROJECT="arroba-event-publication" \
  ARROBA_AEDS_BACKUP_DIR="${aeds_repository}/backups" \
    "${aeds_repository}/deploy/backup.sh"
)"
ARROBA_AEDS_COMPOSE_FILE="${deployment_dir}/compose.yaml" \
ARROBA_AEDS_COMPOSE_PROJECT_DIRECTORY="${deployment_dir}" \
ARROBA_AEDS_COMPOSE_PROJECT="arroba-event-publication" \
ARROBA_AEDS_BACKUP_DIR="${aeds_repository}/backups" \
  "${aeds_repository}/deploy/restore.sh" --yes "$(basename "${backup_path}")"
restored_metrics="$(curl --fail --silent "http://127.0.0.1:${ARROBA_AEDS_PRODUCER_PORT}/metrics")"
if ! grep -q '^arroba_aeds_active_routes 1$' <<< "${restored_metrics}"; then
  printf 'AEDS route state did not survive backup/restore\n%s\n' "${restored_metrics}" >&2
  exit 1
fi
printf '%s\n' "${restored_metrics}"
