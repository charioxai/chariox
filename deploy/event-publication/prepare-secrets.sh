#!/usr/bin/env bash
set -euo pipefail

deployment_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
secrets_dir="${deployment_dir}/secrets"
mkdir -p "${secrets_dir}"
chmod 700 "${secrets_dir}"

resolve_port() {
  local name="$1"
  local fallback="$2"
  local explicit="${!name:-}"
  local persisted=""
  if [[ -f "${deployment_dir}/.env" ]]; then
    persisted="$(sed -n "s/^${name}=//p" "${deployment_dir}/.env" | tail -1)"
  fi
  local value="${explicit:-${persisted:-${fallback}}}"
  if [[ ! "${value}" =~ ^[0-9]+$ ]] || (( value < 1 || value > 65535 )); then
    printf '%s must be a TCP port between 1 and 65535\n' "${name}" >&2
    exit 2
  fi
  printf '%s\n' "${value}"
}

aeds_kernel_port="$(resolve_port ARROBA_AEDS_KERNEL_PORT 43130)"
aeds_producer_port="$(resolve_port ARROBA_AEDS_PRODUCER_PORT 43131)"
dummy_aegs_port="$(resolve_port ARROBA_DUMMY_AEGS_PORT 43132)"
github_aegs_port="$(resolve_port ARROBA_GITHUB_AEGS_PORT 43133)"
jira_aegs_port="$(resolve_port ARROBA_JIRA_AEGS_PORT 43134)"
linear_aegs_port="$(resolve_port ARROBA_LINEAR_AEGS_PORT 43135)"
gitlab_aegs_port="$(resolve_port ARROBA_GITLAB_AEGS_PORT 43136)"
sentry_aegs_port="$(resolve_port ARROBA_SENTRY_AEGS_PORT 43137)"
slack_aegs_port="$(resolve_port ARROBA_SLACK_AEGS_PORT 43138)"
if [[ "$(printf '%s\n' \
  "${aeds_kernel_port}" "${aeds_producer_port}" "${dummy_aegs_port}" "${github_aegs_port}" \
  "${jira_aegs_port}" "${linear_aegs_port}" "${gitlab_aegs_port}" \
  "${sentry_aegs_port}" "${slack_aegs_port}" \
  | sort -u | wc -l | tr -d ' ')" != "9" ]]; then
  printf 'Event-service host ports must be distinct\n' >&2
  exit 2
fi
printf 'ARROBA_AEDS_KERNEL_PORT=%s\nARROBA_AEDS_PRODUCER_PORT=%s\nARROBA_DUMMY_AEGS_PORT=%s\nARROBA_GITHUB_AEGS_PORT=%s\nARROBA_JIRA_AEGS_PORT=%s\nARROBA_LINEAR_AEGS_PORT=%s\nARROBA_GITLAB_AEGS_PORT=%s\nARROBA_SENTRY_AEGS_PORT=%s\nARROBA_SLACK_AEGS_PORT=%s\n' \
  "${aeds_kernel_port}" \
  "${aeds_producer_port}" \
  "${dummy_aegs_port}" \
  "${github_aegs_port}" \
  "${jira_aegs_port}" \
  "${linear_aegs_port}" \
  "${gitlab_aegs_port}" \
  "${sentry_aegs_port}" \
  "${slack_aegs_port}" \
  > "${deployment_dir}/.env"
chmod 600 "${deployment_dir}/.env"

ensure_secret() {
  local path="$1"
  if [[ ! -s "${path}" ]]; then
    openssl rand -hex 32 > "${path}"
  fi
  chmod 600 "${path}"
}

kernel_token_file="${secrets_dir}/local-kernel-token"
producer_token_file="${secrets_dir}/dummy-aegs-producer-token"
management_token_file="${secrets_dir}/dummy-aegs-management-token"
github_producer_token_file="${secrets_dir}/github-aegs-producer-token"
github_management_token_file="${secrets_dir}/github-aegs-management-token"
github_webhook_secret_file="${secrets_dir}/github-aegs-webhook-secret"
jira_producer_token_file="${secrets_dir}/jira-aegs-producer-token"
jira_management_token_file="${secrets_dir}/jira-aegs-management-token"
jira_webhook_secret_file="${secrets_dir}/jira-aegs-webhook-secret"
linear_producer_token_file="${secrets_dir}/linear-aegs-producer-token"
linear_management_token_file="${secrets_dir}/linear-aegs-management-token"
linear_webhook_secret_file="${secrets_dir}/linear-aegs-webhook-secret"
gitlab_producer_token_file="${secrets_dir}/gitlab-aegs-producer-token"
gitlab_management_token_file="${secrets_dir}/gitlab-aegs-management-token"
gitlab_webhook_secret_file="${secrets_dir}/gitlab-aegs-webhook-secret"
sentry_producer_token_file="${secrets_dir}/sentry-aegs-producer-token"
sentry_management_token_file="${secrets_dir}/sentry-aegs-management-token"
sentry_webhook_secret_file="${secrets_dir}/sentry-aegs-webhook-secret"
slack_producer_token_file="${secrets_dir}/slack-aegs-producer-token"
slack_management_token_file="${secrets_dir}/slack-aegs-management-token"
slack_webhook_secret_file="${secrets_dir}/slack-aegs-webhook-secret"
for secret_file in \
  "${kernel_token_file}" \
  "${producer_token_file}" \
  "${management_token_file}" \
  "${github_producer_token_file}" \
  "${github_management_token_file}" \
  "${github_webhook_secret_file}" \
  "${jira_producer_token_file}" \
  "${jira_management_token_file}" \
  "${jira_webhook_secret_file}" \
  "${linear_producer_token_file}" \
  "${linear_management_token_file}" \
  "${linear_webhook_secret_file}" \
  "${gitlab_producer_token_file}" \
  "${gitlab_management_token_file}" \
  "${gitlab_webhook_secret_file}" \
  "${sentry_producer_token_file}" \
  "${sentry_management_token_file}" \
  "${sentry_webhook_secret_file}" \
  "${slack_producer_token_file}" \
  "${slack_management_token_file}" \
  "${slack_webhook_secret_file}"
do
  ensure_secret "${secret_file}"
done

kernel_token="$(tr -d '\r\n' < "${kernel_token_file}")"
producer_token="$(tr -d '\r\n' < "${producer_token_file}")"
management_token="$(tr -d '\r\n' < "${management_token_file}")"
github_producer_token="$(tr -d '\r\n' < "${github_producer_token_file}")"
github_management_token="$(tr -d '\r\n' < "${github_management_token_file}")"
jira_producer_token="$(tr -d '\r\n' < "${jira_producer_token_file}")"
jira_management_token="$(tr -d '\r\n' < "${jira_management_token_file}")"
linear_producer_token="$(tr -d '\r\n' < "${linear_producer_token_file}")"
linear_management_token="$(tr -d '\r\n' < "${linear_management_token_file}")"
gitlab_producer_token="$(tr -d '\r\n' < "${gitlab_producer_token_file}")"
gitlab_management_token="$(tr -d '\r\n' < "${gitlab_management_token_file}")"
sentry_producer_token="$(tr -d '\r\n' < "${sentry_producer_token_file}")"
sentry_management_token="$(tr -d '\r\n' < "${sentry_management_token_file}")"
slack_producer_token="$(tr -d '\r\n' < "${slack_producer_token_file}")"
slack_management_token="$(tr -d '\r\n' < "${slack_management_token_file}")"
printf '{"local-container-kernel":"%s"}\n' "${kernel_token}" \
  > "${secrets_dir}/aeds-kernel-tokens.json"
printf '{"dev.arroba.dummy":{"token":"%s","event_types":["dummy.test"]},"dev.arroba.github":{"token":"%s","event_types":["pull_request.opened","pull_request.synchronize","pull_request.review_requested","issues.opened","workflow_run.completed"]},"dev.arroba.jira-cloud":{"token":"%s","event_types":["issue.created","issue.assigned","issue.updated","issue.transitioned"]},"dev.arroba.linear":{"token":"%s","event_types":["issue.created","issue.assigned","issue.updated","project.updated"]},"dev.arroba.gitlab":{"token":"%s","event_types":["merge_request.opened","merge_request.updated","issue.created","pipeline.completed"]},"dev.arroba.sentry":{"token":"%s","event_types":["issue.created","issue.resolved"]},"dev.arroba.slack":{"token":"%s","event_types":["app.mentioned","message.channels","reaction.added"]}}\n' \
  "${producer_token}" "${github_producer_token}" "${jira_producer_token}" \
  "${linear_producer_token}" "${gitlab_producer_token}" "${sentry_producer_token}" \
  "${slack_producer_token}" \
  > "${secrets_dir}/aeds-producer-capabilities.json"
printf '{"dev.arroba.dummy":{"url":"http://127.0.0.1:%s","token":"%s"},"dev.arroba.github":{"url":"http://127.0.0.1:%s","token":"%s"},"dev.arroba.jira-cloud":{"url":"http://127.0.0.1:%s","token":"%s"},"dev.arroba.linear":{"url":"http://127.0.0.1:%s","token":"%s"},"dev.arroba.gitlab":{"url":"http://127.0.0.1:%s","token":"%s"},"dev.arroba.sentry":{"url":"http://127.0.0.1:%s","token":"%s"},"dev.arroba.slack":{"url":"http://127.0.0.1:%s","token":"%s"}}\n' \
  "${dummy_aegs_port}" "${management_token}" \
  "${github_aegs_port}" "${github_management_token}" \
  "${jira_aegs_port}" "${jira_management_token}" \
  "${linear_aegs_port}" "${linear_management_token}" \
  "${gitlab_aegs_port}" "${gitlab_management_token}" \
  "${sentry_aegs_port}" "${sentry_management_token}" \
  "${slack_aegs_port}" "${slack_management_token}" \
  > "${secrets_dir}/kernel-aegs-management-targets.json"
chmod 600 \
  "${secrets_dir}/aeds-kernel-tokens.json" \
  "${secrets_dir}/aeds-producer-capabilities.json" \
  "${secrets_dir}/kernel-aegs-management-targets.json"

printf 'Prepared local event-service secrets in %s\n' "${secrets_dir}"
