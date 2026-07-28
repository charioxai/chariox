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

docker compose --project-directory "${deployment_dir}" -f "${deployment_dir}/compose.yaml" up --build -d --wait

kernel_token="$(tr -d '\r\n' < "${deployment_dir}/secrets/local-kernel-token")"
dummy_management_token="$(tr -d '\r\n' < "${deployment_dir}/secrets/dummy-aegs-management-token")"
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

ARROBA_DRILL_KERNEL_TOKEN="${kernel_token}" \
ARROBA_DRILL_AEDS_URL="ws://127.0.0.1:${ARROBA_AEDS_KERNEL_PORT}" \
ARROBA_DRILL_GITHUB_AEGS_URL="http://127.0.0.1:${ARROBA_GITHUB_AEGS_PORT}" \
ARROBA_DRILL_GITHUB_MANAGEMENT_TOKEN="$(tr -d '\r\n' < "${deployment_dir}/secrets/github-aegs-management-token")" \
ARROBA_DRILL_GITHUB_WEBHOOK_SECRET="$(tr -d '\r\n' < "${deployment_dir}/secrets/github-aegs-webhook-secret")" \
ARROBA_DRILL_JIRA_AEGS_URL="http://127.0.0.1:${ARROBA_JIRA_AEGS_PORT}" \
ARROBA_DRILL_JIRA_MANAGEMENT_TOKEN="$(tr -d '\r\n' < "${deployment_dir}/secrets/jira-aegs-management-token")" \
ARROBA_DRILL_JIRA_WEBHOOK_SECRET="$(tr -d '\r\n' < "${deployment_dir}/secrets/jira-aegs-webhook-secret")" \
ARROBA_DRILL_LINEAR_AEGS_URL="http://127.0.0.1:${ARROBA_LINEAR_AEGS_PORT}" \
ARROBA_DRILL_LINEAR_MANAGEMENT_TOKEN="$(tr -d '\r\n' < "${deployment_dir}/secrets/linear-aegs-management-token")" \
ARROBA_DRILL_LINEAR_WEBHOOK_SECRET="$(tr -d '\r\n' < "${deployment_dir}/secrets/linear-aegs-webhook-secret")" \
ARROBA_DRILL_GITLAB_AEGS_URL="http://127.0.0.1:${ARROBA_GITLAB_AEGS_PORT}" \
ARROBA_DRILL_GITLAB_MANAGEMENT_TOKEN="$(tr -d '\r\n' < "${deployment_dir}/secrets/gitlab-aegs-management-token")" \
ARROBA_DRILL_GITLAB_WEBHOOK_SECRET="$(tr -d '\r\n' < "${deployment_dir}/secrets/gitlab-aegs-webhook-secret")" \
ARROBA_DRILL_SENTRY_AEGS_URL="http://127.0.0.1:${ARROBA_SENTRY_AEGS_PORT}" \
ARROBA_DRILL_SENTRY_MANAGEMENT_TOKEN="$(tr -d '\r\n' < "${deployment_dir}/secrets/sentry-aegs-management-token")" \
ARROBA_DRILL_SENTRY_WEBHOOK_SECRET="$(tr -d '\r\n' < "${deployment_dir}/secrets/sentry-aegs-webhook-secret")" \
ARROBA_DRILL_SLACK_AEGS_URL="http://127.0.0.1:${ARROBA_SLACK_AEGS_PORT}" \
ARROBA_DRILL_SLACK_MANAGEMENT_TOKEN="$(tr -d '\r\n' < "${deployment_dir}/secrets/slack-aegs-management-token")" \
ARROBA_DRILL_SLACK_WEBHOOK_SECRET="$(tr -d '\r\n' < "${deployment_dir}/secrets/slack-aegs-webhook-secret")" \
  cargo run --quiet --manifest-path "${aeds_repository}/Cargo.toml" \
    --example first_wave_provider_drill

curl --fail --silent "http://127.0.0.1:${ARROBA_AEDS_PRODUCER_PORT}/metrics"
docker compose --project-directory "${deployment_dir}" -f "${deployment_dir}/compose.yaml" restart aeds
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
