#!/usr/bin/env bash
set -Eeuo pipefail

workspace="${CHARIOX_MANAGED_ISOLATION_PROBE_WORKSPACE:?probe workspace is required}"
result="${CHARIOX_MANAGED_ISOLATION_PROBE_RESULT:?probe result is required}"
real_provider="${CHARIOX_MANAGED_ISOLATION_REAL_PROVIDER:?real provider executable is required}"
unselected="${CHARIOX_MANAGED_ISOLATION_PROBE_UNSELECTED_REPOSITORY:?unselected repository is required}"
account="${CODEX_HOME:-}"

[[ "${CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE:-}" == "1" ]]
[[ "$HOME" == "/home/chariox" ]]
[[ -d "$workspace" && -w "$workspace" ]]
[[ -n "$account" && -d "$account" && -r "$account" && -w "$account" ]]
[[ -x "$real_provider" ]]

for denied in \
  /var/lib/chariox \
  /var/lib/chariox-slice-share \
  /run/chariox-slice-broker.sock \
  /proc/1/root/var/lib/chariox \
  "$unselected"
do
  [[ ! -e "$denied" ]]
done

for secret_name in \
  CHARIOX_RELAY_TOKEN \
  CHARIOX_KERNEL_LOCAL_AUTH_TOKEN \
  CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE \
  CHARIOX_SLICE_DOCKER_BROKER_SOCKET \
  CHARIOX_SLICE_DOCKER_BROKER_FD
do
  [[ -z "${!secret_name:-}" ]]
done

account_probe="$account/.chariox-isolation-account-$$"
workspace_probe="$workspace/.chariox-isolation-workspace-$$"
cross_mount_probe="$workspace/.chariox-isolation-cross-mount-$$"
cleanup() {
  rm -f "$account_probe" "$workspace_probe" "$cross_mount_probe"
}
trap cleanup EXIT
printf 'account\n' >"$account_probe"
printf 'workspace\n' >"$workspace_probe"
if ln "$account_probe" "$cross_mount_probe" 2>/dev/null; then
  printf 'provider account and workspace unexpectedly share one writable mount\n' >&2
  exit 1
fi

printf 'managed_provider_isolation=ok\nreal_provider=%s\nworkspace=%s\naccount=%s\n' \
  "$real_provider" "$workspace" "$account" >"$result"
chmod 600 "$result"
cleanup
trap - EXIT
exec "$real_provider" "$@"
