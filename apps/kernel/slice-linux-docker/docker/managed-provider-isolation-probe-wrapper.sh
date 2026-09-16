#!/usr/bin/env bash
set -Eeuo pipefail

workspace="${CHARIOX_MANAGED_ISOLATION_PROBE_WORKSPACE:?probe workspace is required}"
result="${CHARIOX_MANAGED_ISOLATION_PROBE_RESULT:?probe result is required}"
real_provider="${CHARIOX_MANAGED_ISOLATION_REAL_PROVIDER:?real provider executable is required}"
account="${CODEX_HOME:-}"

fail() {
  printf 'managed_provider_isolation=failure\nreason=%s\n' "$1" >"$result" 2>/dev/null || true
  chmod 600 "$result" 2>/dev/null || true
  printf '%s\n' "$1" >&2
  exit 1
}

[[ "${CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE:-}" == "1" ]] \
  || fail "managed provider isolation marker is unavailable"
[[ "$HOME" == "/home/chariox" ]] \
  || fail "managed provider sandbox HOME is incorrect"
[[ -d "$workspace" && -w "$workspace" ]] \
  || fail "probe workspace is unavailable or not writable"
[[ -n "$account" && -d "$account" && -r "$account" && -w "$account" ]] \
  || fail "probe provider account is unavailable or not writable"
[[ -x "$real_provider" ]] \
  || fail "real provider executable is unavailable"

for denied in \
  /var/lib/chariox \
  /home/slice/.chariox \
  /run/chariox-slice-broker.sock \
  /proc/1/root/var/lib/chariox
do
  [[ ! -e "$denied" ]] || fail "a denied host path is visible in the provider sandbox"
done

for secret_name in \
  CHARIOX_RELAY_TOKEN \
  CHARIOX_KERNEL_LOCAL_AUTH_TOKEN \
  CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE \
  CHARIOX_SLICE_DOCKER_BROKER_SOCKET \
  CHARIOX_SLICE_DOCKER_BROKER_FD \
  CHARIOX_HOME \
  CHARIOX_CAPABILITY_ISOLATION_ROOT \
  CHARIOX_MANAGED_PROVIDER_HOME \
  CHARIOX_MANAGED_VAULT_PATH \
  CHARIOX_DISPOSABLE_WORKER_BOOTSTRAP_PATH
do
  [[ -z "${!secret_name:-}" ]] || fail "a denied control credential is visible in the provider sandbox"
done

account_probe="$account/.chariox-isolation-account-$$"
workspace_probe="$workspace/.chariox-isolation-workspace-$$"
cross_mount_probe="$workspace/.chariox-isolation-cross-mount-$$"
outside_root="/tmp/chariox-managed-isolation-outside-$$"
outside_repository="$outside_root/repository"
cloned_repository="$outside_root/cloned"
cleanup() {
  rm -f "$account_probe" "$workspace_probe" "$cross_mount_probe"
  rm -rf "$outside_root"
}
trap cleanup EXIT
printf 'account\n' >"$account_probe"
printf 'workspace\n' >"$workspace_probe"
if ln "$account_probe" "$cross_mount_probe" 2>/dev/null; then
  printf 'provider account and workspace unexpectedly share one writable mount\n' >&2
  exit 1
fi

# This deliberately lives outside the transferred publication mount. The
# managed boundary must retain ordinary filesystem permissions here, so a
# provider can create a repository and clone into another new repository
# without project registration or another root allowlist.
mkdir -p "$outside_repository"
git -C "$outside_repository" init --quiet
printf 'outside managed workspace\n' >"$outside_repository/README.md"
git -C "$outside_repository" add README.md
git -C "$outside_repository" -c user.name=probe -c user.email=probe@example.invalid commit --quiet -m probe
git clone --quiet "$outside_repository" "$cloned_repository"
git -C "$cloned_repository" status --porcelain >/dev/null

printf 'managed_provider_isolation=ok\nreal_provider=%s\nworkspace=%s\naccount=%s\noutside_repository=%s\noutside_clone=%s\n' \
  "$real_provider" "$workspace" "$account" "$outside_repository" "$cloned_repository" >"$result"
chmod 600 "$result"
cleanup
trap - EXIT
exec "$real_provider" "$@"
