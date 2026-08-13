#!/usr/bin/env bash
set -euo pipefail

core_only=false
case "${1:-}" in
  "") ;;
  --core-only) core_only=true ;;
  *)
    printf 'usage: %s [--core-only]\n' "$0" >&2
    exit 2
    ;;
esac
if (( $# > 1 )); then
  printf 'usage: %s [--core-only]\n' "$0" >&2
  exit 2
fi

repository_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
deployment_dir="${repository_dir}/deploy/event-publication"
services_root="$(cd "${repository_dir}/.." && pwd)"
aeds_repository="${services_root}/chariox-aeds"
export CHARIOX_BUILD_REVISION="${CHARIOX_BUILD_REVISION:-$(git -C "${repository_dir}" rev-parse HEAD)-dirty}"
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

wait_for_aeds_ready() {
  for attempt in {1..30}; do
    if curl --fail --silent "http://127.0.0.1:${CHARIOX_AEDS_PRODUCER_PORT}/readyz"; then
      return 0
    fi
    sleep 1
  done
  printf 'AEDS did not become reachable through its host port\n' >&2
  return 1
}

kernel_token="$(tr -d '\r\n' < "${deployment_dir}/secrets/local-kernel-token")"
dummy_management_token="$(tr -d '\r\n' < "${deployment_dir}/secrets/dummy-aegs-management-token")"
start_aegs dummy-aegs
authorization="$(
  curl --fail --silent \
    -H "authorization: Bearer ${dummy_management_token}" \
    -H "content-type: application/json" \
    --data '{"generator_id":"dev.chariox.dummy"}' \
    "http://127.0.0.1:${CHARIOX_DUMMY_AEGS_PORT}/v1/authorizations"
)"
if ! grep -q '"connection_id":"local-dummy"' <<< "${authorization}"; then
  printf 'Dummy AEGS authorization did not return its opaque connection\n' >&2
  exit 1
fi
resources="$(
  curl --fail --silent \
    -H "authorization: Bearer ${dummy_management_token}" \
    -H "content-type: application/json" \
    --data '{"generator_id":"dev.chariox.dummy","connection_id":"local-dummy","limit":20}' \
    "http://127.0.0.1:${CHARIOX_DUMMY_AEGS_PORT}/v1/resources/query"
)"
if ! grep -q '"connection_scope":"default"' <<< "${resources}"; then
  printf 'Dummy AEGS resource enumeration did not return its provider scope\n' >&2
  exit 1
fi

CHARIOX_DRILL_KERNEL_TOKEN="${kernel_token}" \
CHARIOX_DRILL_AEDS_URL="ws://127.0.0.1:${CHARIOX_AEDS_KERNEL_PORT}" \
CHARIOX_DRILL_AEGS_URL="http://127.0.0.1:${CHARIOX_DUMMY_AEGS_PORT}/v1/emit" \
  cargo run --quiet --manifest-path "${aeds_repository}/Cargo.toml" \
    --example deployment_drill
stop_aegs dummy-aegs

provider_specs=(
  "github-aegs|github|GITHUB|CHARIOX_GITHUB_AEGS_PORT|github"
  "jira-aegs|jira-cloud|JIRA|CHARIOX_JIRA_AEGS_PORT|jira"
  "linear-aegs|linear|LINEAR|CHARIOX_LINEAR_AEGS_PORT|linear"
  "gitlab-aegs|gitlab|GITLAB|CHARIOX_GITLAB_AEGS_PORT|gitlab"
  "sentry-aegs|sentry|SENTRY|CHARIOX_SENTRY_AEGS_PORT|sentry"
  "slack-aegs|slack|SLACK|CHARIOX_SLACK_AEGS_PORT|slack"
)
if [[ "${core_only}" != "true" ]]; then
  for provider_spec in "${provider_specs[@]}"; do
    IFS="|" read -r service fixture prefix port_variable secret_stem <<< "${provider_spec}"
    port="${!port_variable}"
    start_aegs "${service}"
    env \
      CHARIOX_DRILL_PROVIDERS="${fixture}" \
      CHARIOX_DRILL_KERNEL_TOKEN="${kernel_token}" \
      CHARIOX_DRILL_AEDS_URL="ws://127.0.0.1:${CHARIOX_AEDS_KERNEL_PORT}" \
      "CHARIOX_DRILL_${prefix}_AEGS_URL=http://127.0.0.1:${port}" \
      "CHARIOX_DRILL_${prefix}_MANAGEMENT_TOKEN=$(tr -d '\r\n' < "${deployment_dir}/secrets/${secret_stem}-aegs-management-token")" \
      "CHARIOX_DRILL_${prefix}_WEBHOOK_SECRET=$(tr -d '\r\n' < "${deployment_dir}/secrets/${secret_stem}-aegs-webhook-secret")" \
      cargo run --quiet --manifest-path "${aeds_repository}/Cargo.toml" \
        --example first_wave_provider_drill
    stop_aegs "${service}"
  done
fi

curl --fail --silent "http://127.0.0.1:${CHARIOX_AEDS_PRODUCER_PORT}/metrics"
"${compose[@]}" restart aeds
wait_for_aeds_ready
metrics="$(curl --fail --silent "http://127.0.0.1:${CHARIOX_AEDS_PRODUCER_PORT}/metrics")"
if ! grep -q '^chariox_aeds_active_routes 1$' <<< "${metrics}"; then
  printf 'AEDS route state did not survive restart\n%s\n' "${metrics}" >&2
  exit 1
fi
printf '%s\n' "${metrics}"

backup_path="$(
  CHARIOX_AEDS_COMPOSE_FILE="${deployment_dir}/compose.yaml" \
  CHARIOX_AEDS_COMPOSE_PROJECT_DIRECTORY="${deployment_dir}" \
  CHARIOX_AEDS_COMPOSE_PROJECT="chariox-event-publication" \
  CHARIOX_AEDS_BACKUP_DIR="${aeds_repository}/backups" \
    "${aeds_repository}/deploy/backup.sh"
)"
CHARIOX_AEDS_COMPOSE_FILE="${deployment_dir}/compose.yaml" \
CHARIOX_AEDS_COMPOSE_PROJECT_DIRECTORY="${deployment_dir}" \
CHARIOX_AEDS_COMPOSE_PROJECT="chariox-event-publication" \
CHARIOX_AEDS_BACKUP_DIR="${aeds_repository}/backups" \
  "${aeds_repository}/deploy/restore.sh" --yes "$(basename "${backup_path}")"
wait_for_aeds_ready
restored_metrics="$(curl --fail --silent "http://127.0.0.1:${CHARIOX_AEDS_PRODUCER_PORT}/metrics")"
if ! grep -q '^chariox_aeds_active_routes 1$' <<< "${restored_metrics}"; then
  printf 'AEDS route state did not survive backup/restore\n%s\n' "${restored_metrics}" >&2
  exit 1
fi
printf '%s\n' "${restored_metrics}"
