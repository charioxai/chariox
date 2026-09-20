#!/usr/bin/env bash
set -Eeuo pipefail

# Run the already-installed shared-host kernel launch path under the same
# service user and systemd hardening as chariox-managed-bootstrap. This
# deliberately starts only transient, uniquely named units; it never restarts,
# stops, or rebinds a live Chariox unit. Path 1 is one disposable VM per
# worker, so this managed-isolation probe is not applicable there and exits
# before checking or requiring Bubblewrap.

usage() {
  cat >&2 <<'EOF'
usage: probe-provider-launch-ab.sh <rollback-kernel> <candidate-kernel>

Both arguments must be explicit installed chariox-kernel binaries. The probe
runs each binary once, sequentially, as chariox and leaves evidence under a
new /var/lib/chariox-provider-launch-ab.* directory.

Environment:
  CHARIOX_PROVIDER_LAUNCH_PROBE_CONTEXT  complete updated probe context root (required)
  CHARIOX_PROVIDER_LAUNCH_PROBE_REAL_PROVIDER  provider executable (codex)
  CHARIOX_MANAGED_PROVIDER_TOPOLOGY      path1 (default) or shared_host
  CHARIOX_PROVIDER_LAUNCH_PROBE_NATIVE_TUI   1 (default) or 0
  CHARIOX_PROVIDER_LAUNCH_PROBE_TIMEOUT_MS   5000..120000 (default 45000)
  CHARIOX_PROVIDER_LAUNCH_PROBE_KEEP        1 (default) or 0 to remove evidence

The source context is copied verbatim, per case, into the scratch provider home
with its websocket dependency before the User=chariox unit starts. In the
explicit shared_host topology, managed isolation rebinds that home to
/home/chariox, while /var/lib/chariox is intentionally inaccessible to the
transient units.
EOF
}

if [[ $# -ne 2 ]]; then
  usage
  exit 2
fi

case "${CHARIOX_MANAGED_PROVIDER_TOPOLOGY:-path1}" in
  path1)
    printf '%s\n' 'provider launch A/B probe skipped: Path 1 uses the ordinary VM kernel/provider boundary' >&2
    exit 0
    ;;
  shared_host|legacy_shared_host)
    ;;
  *)
    printf '%s\n' 'CHARIOX_MANAGED_PROVIDER_TOPOLOGY must be path1 or shared_host' >&2
    exit 2
    ;;
esac

[[ "$(id -u)" == "0" ]] || {
  printf '%s\n' 'provider launch A/B probe must run as root (run it directly; it does not invoke sudo)' >&2
  exit 1
}

for required_command in id getent install cp mktemp od readlink stat systemctl systemd-run node sed tr tail awk grep timeout; do
  command -v "$required_command" >/dev/null 2>&1 || {
    printf 'provider launch A/B probe requires %s\n' "$required_command" >&2
    exit 1
  }
done

if ! command -v ss >/dev/null 2>&1; then
  printf '%s\n' 'provider launch A/B probe requires ss for collision-free loopback ports' >&2
  exit 1
fi

[[ -n "$(getent passwd chariox || true)" ]] || {
  printf '%s\n' 'managed service user chariox is unavailable' >&2
  exit 1
}
[[ -n "$(getent group chariox-slice || true)" ]] || {
  printf '%s\n' 'managed supplementary group chariox-slice is unavailable' >&2
  exit 1
}

SYSTEMCTL="$(command -v systemctl)"
SYSTEMD_RUN="$(command -v systemd-run)"
NODE="$(command -v node)"
SS="$(command -v ss)"
JOURNALCTL="$(command -v journalctl || true)"

canonical_installed_executable() {
  local input_path="$1"
  local label="$2"
  [[ "$input_path" == /* ]] || {
    printf '%s must be an absolute path: %s\n' "$label" "$input_path" >&2
    return 1
  }
  [[ -f "$input_path" && -x "$input_path" ]] || {
    printf '%s is not an executable file: %s\n' "$label" "$input_path" >&2
    return 1
  }
  local resolved owner mode
  resolved="$(readlink -f -- "$input_path")" || {
    printf '%s cannot be canonicalized: %s\n' "$label" "$input_path" >&2
    return 1
  }
  [[ -f "$resolved" && -x "$resolved" ]] || {
    printf '%s resolved to a non-executable file: %s\n' "$label" "$resolved" >&2
    return 1
  }
  owner="$(stat -c '%u' -- "$resolved")"
  mode="$(stat -c '%a' -- "$resolved")"
  [[ "$owner" == "0" ]] || {
    printf '%s must be root-owned: %s\n' "$label" "$resolved" >&2
    return 1
  }
  if (( (0$mode & 0022) != 0 )); then
    printf '%s must not be writable by group or other: %s\n' "$label" "$resolved" >&2
    return 1
  fi
  printf '%s\n' "$resolved"
}

rollback_kernel="$(canonical_installed_executable "$1" rollback kernel)" || exit 1
candidate_kernel="$(canonical_installed_executable "$2" candidate kernel)" || exit 1

probe_context_input="${CHARIOX_PROVIDER_LAUNCH_PROBE_CONTEXT:-}"
[[ -n "$probe_context_input" ]] || {
  printf '%s\n' 'set CHARIOX_PROVIDER_LAUNCH_PROBE_CONTEXT to the complete updated probe context' >&2
  exit 1
}
[[ "$probe_context_input" == /* ]] || {
  printf '%s\n' 'CHARIOX_PROVIDER_LAUNCH_PROBE_CONTEXT must be absolute' >&2
  exit 1
}
probe_context_source="$(readlink -f -- "$probe_context_input")" || {
  printf 'cannot canonicalize probe context: %s\n' "$probe_context_input" >&2
  exit 1
}
[[ -d "$probe_context_source" ]] || {
  printf 'probe context is not a directory: %s\n' "$probe_context_source" >&2
  exit 1
}

probe_script_source="$probe_context_source/apps/kernel/slice-linux-docker/docker/managed-provider-isolation-probe.mjs"
probe_wrapper_source="$probe_context_source/apps/kernel/slice-linux-docker/docker/managed-provider-isolation-probe-wrapper.sh"
probe_package_source_input="${CHARIOX_PROBE_PACKAGE_JSON:-$probe_context_source/apps/kernel/slice-linux-docker/toolchain/package.json}"
[[ "$probe_package_source_input" == /* ]] || {
  printf '%s\n' 'CHARIOX_PROBE_PACKAGE_JSON must be absolute' >&2
  exit 1
}
probe_package_source="$(readlink -f -- "$probe_package_source_input")" || {
  printf 'cannot canonicalize probe package: %s\n' "$probe_package_source_input" >&2
  exit 1
}
for probe_file in "$probe_script_source" "$probe_wrapper_source" "$probe_package_source"; do
  [[ -f "$probe_file" ]] || {
    printf 'installed managed provider probe file is missing: %s\n' "$probe_file" >&2
    exit 1
  }
done
[[ -x "$probe_wrapper_source" ]] || {
  printf 'managed provider probe wrapper is not executable: %s\n' "$probe_wrapper_source" >&2
  exit 1
}
probe_ws_source="${probe_package_source%/*}/node_modules/ws"
[[ -f "$probe_ws_source/package.json" ]] || {
  printf 'probe package has no adjacent node_modules/ws dependency: %s\n' "$probe_package_source" >&2
  printf '%s\n' 'set CHARIOX_PROBE_PACKAGE_JSON to a package with the exact ws dependency installed' >&2
  exit 1
}

real_provider_input="${CHARIOX_PROVIDER_LAUNCH_PROBE_REAL_PROVIDER:-}"
if [[ -z "$real_provider_input" ]]; then
  real_provider_input="$(command -v codex || true)"
fi
[[ -n "$real_provider_input" ]] || {
  printf '%s\n' 'Codex was not found; set CHARIOX_PROVIDER_LAUNCH_PROBE_REAL_PROVIDER to its installed executable' >&2
  exit 1
}
real_provider="$(canonical_installed_executable "$real_provider_input" real provider)" || exit 1
[[ "$real_provider" != "$probe_wrapper_source" ]] || {
  printf '%s\n' 'real provider must be Codex, not the managed probe wrapper' >&2
  exit 1
}

probe_timeout_ms="${CHARIOX_PROVIDER_LAUNCH_PROBE_TIMEOUT_MS:-45000}"
case "$probe_timeout_ms" in
  ''|*[!0-9]*) printf '%s\n' 'CHARIOX_PROVIDER_LAUNCH_PROBE_TIMEOUT_MS must be an integer' >&2; exit 1 ;;
esac
(( probe_timeout_ms >= 5000 && probe_timeout_ms <= 120000 )) || {
  printf '%s\n' 'CHARIOX_PROVIDER_LAUNCH_PROBE_TIMEOUT_MS must be between 5000 and 120000' >&2
  exit 1
}

native_tui="${CHARIOX_PROVIDER_LAUNCH_PROBE_NATIVE_TUI:-1}"
case "$native_tui" in
  0|1) ;;
  *) printf '%s\n' 'CHARIOX_PROVIDER_LAUNCH_PROBE_NATIVE_TUI must be 0 or 1' >&2; exit 1 ;;
esac

keep_evidence="${CHARIOX_PROVIDER_LAUNCH_PROBE_KEEP:-1}"
case "$keep_evidence" in
  0|1) ;;
  *) printf '%s\n' 'CHARIOX_PROVIDER_LAUNCH_PROBE_KEEP must be 0 or 1' >&2; exit 1 ;;
esac

probe_root="$(mktemp -d /var/lib/chariox-provider-launch-ab.XXXXXX)"
case "$probe_root" in
  /var/lib/chariox-provider-launch-ab.*) ;;
  *) printf 'unexpected probe root from mktemp: %s\n' "$probe_root" >&2; exit 1 ;;
esac
# The service user needs execute-only access through this parent to reach its
# private 0700 case directory. Root still owns the parent and all evidence.
chmod 0711 -- "$probe_root"

active_unit=""
cleanup() {
  if [[ -n "$active_unit" ]]; then
    "$SYSTEMCTL" stop "$active_unit" >/dev/null 2>&1 || true
    active_unit=""
  fi
  if [[ "$keep_evidence" == "0" && -d "$probe_root" ]]; then
    case "$probe_root" in
      /var/lib/chariox-provider-launch-ab.*) rm -rf -- "$probe_root" ;;
    esac
  fi
}
trap cleanup EXIT

port_is_in_use() {
  local port="$1"
  "$SS" -H -ltn 2>/dev/null | awk -v suffix=":$port" '$4 ~ (suffix "$") { found = 1 } END { exit found ? 0 : 1 }'
}

choose_ports() {
  local attempt base port free
  for ((attempt = 0; attempt < 100; attempt += 1)); do
    base=$((24000 + (RANDOM % 12000)))
    kernel_port=$base
    mcp_port=$((base + 1))
    codex_port_start=$((base + 10))
    codex_port_end=$((base + 19))
    free=1
    for port in "$kernel_port" "$mcp_port"; do
      if port_is_in_use "$port"; then free=0; break; fi
    done
    if [[ "$free" == "1" ]]; then
      for ((port = codex_port_start; port <= codex_port_end; port += 1)); do
        if port_is_in_use "$port"; then free=0; break; fi
      done
    fi
    if [[ "$free" == "1" ]]; then return 0; fi
  done
  printf '%s\n' 'could not reserve a collision-free loopback port set' >&2
  return 1
}

prepare_case() {
  case_label="$1"
  case_root="$probe_root/$case_label"
  chariox_home="$case_root/chariox-home"
  provider_home="$case_root/provider-home"
  workspace="$case_root/workspace"
  capability_root="$case_root/capabilities"
  service_root="$case_root/slice-share"
  publication_root="$service_root/slices"
  broker_control_root="$service_root/.broker-private/control"
  broker_socket="$broker_control_root/control.sock"
  bootstrap_root="$case_root/bootstrap"
  bootstrap_path="$bootstrap_root/managed-bootstrap.json"
  vault_root="$chariox_home/.chariox/vault"
  vault_path="$vault_root/vault.json"
  auth_file="$case_root/kernel-local-auth.token"
  result_path="$workspace/.chariox-managed-isolation-probe.result"
  launch_capture="$case_root/launch-capture.json"
  probe_log="$case_root/probe.log"
  kernel_log="$case_root/kernel.log"
  start_log="$case_root/systemd-run.log"
  unit_properties="$case_root/unit.properties"
  runtime_status="$case_root/runtime.status"
  supervisor_env="$case_root/supervisor-env.txt"
  unit="chariox-provider-launch-ab-${case_label}-$$-${RANDOM}.service"

  install -d -o chariox -g chariox -m 0700 \
    "$case_root" \
    "$chariox_home" \
    "$chariox_home/.chariox" \
    "$chariox_home/.chariox/daemon" \
    "$vault_root" \
    "$provider_home" \
    "$workspace" \
    "$capability_root" \
    "$service_root" \
    "$publication_root" \
    "$broker_control_root" \
    "$bootstrap_root"

  probe_context_stage="$provider_home/.probe-context"
  install -d -o chariox -g chariox -m 0755 \
    "$probe_context_stage/apps/kernel/slice-linux-docker/docker" \
    "$probe_context_stage/apps/kernel/slice-linux-docker/toolchain/node_modules"
  install -o chariox -g chariox -m 0644 "$probe_script_source" \
    "$probe_context_stage/apps/kernel/slice-linux-docker/docker/managed-provider-isolation-probe.mjs"
  install -o chariox -g chariox -m 0755 "$probe_wrapper_source" \
    "$probe_context_stage/apps/kernel/slice-linux-docker/docker/managed-provider-isolation-probe-wrapper.sh"
  install -o chariox -g chariox -m 0644 "$probe_package_source" \
    "$probe_context_stage/apps/kernel/slice-linux-docker/toolchain/package.json"
  cp -RP -- "$probe_ws_source" \
    "$probe_context_stage/apps/kernel/slice-linux-docker/toolchain/node_modules/ws"
  chown -R chariox:chariox -- "$probe_context_stage"
  chmod -R a+rX -- "$probe_context_stage"
  probe_context="$probe_context_stage"
  probe_script="$probe_context/apps/kernel/slice-linux-docker/docker/managed-provider-isolation-probe.mjs"
  probe_wrapper="$probe_context/apps/kernel/slice-linux-docker/docker/managed-provider-isolation-probe-wrapper.sh"
  probe_package="$probe_context/apps/kernel/slice-linux-docker/toolchain/package.json"

  cat >"$chariox_home/config.toml" <<EOF
[state]
path = "$chariox_home/.chariox/daemon/kernel-state.db"

[credential_vault]
backend = "process_memory"
path = "$vault_path"
EOF
  chown chariox:chariox "$chariox_home/config.toml"
  chmod 0600 "$chariox_home/config.toml"

  token="$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
  printf '%s\n' "$token" >"$auth_file"
  chown chariox:chariox "$auth_file"
  chmod 0600 "$auth_file"

  choose_ports

  cat >"$supervisor_env" <<EOF
HOME=$chariox_home
CHARIOX_HOME=$chariox_home
CHARIOX_CAPABILITY_ISOLATION_ROOT=$capability_root
CHARIOX_MANAGED_PROVIDER_ISOLATION=1
CHARIOX_MANAGED_PROVIDER_HOME=$provider_home
CHARIOX_MANAGED_SLICE_SERVICE_ROOT=$service_root
CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT=$publication_root
CHARIOX_MANAGED_VAULT_PATH=$vault_path
CHARIOX_MANAGED_BOOTSTRAP_PATH=$bootstrap_path
CHARIOX_MANAGED_RELEASE_MANIFEST=/usr/lib/chariox/release-manifest.json
CHARIOX_MANAGED_RELEASE_SIGNATURE=/usr/lib/chariox/release-manifest.sig
CHARIOX_MANAGED_RELEASE_PUBLIC_KEY=/usr/lib/chariox/release-public-key
CHARIOX_MANAGED_KERNEL_BINARY=/usr/local/bin/chariox-kernel
CHARIOX_SLICE_DOCKER_BROKER_SOCKET=$broker_socket
CHARIOX_KERNEL_HOST=127.0.0.1
CHARIOX_KERNEL_PORT=$kernel_port
CHARIOX_MCP_HOST=127.0.0.1
CHARIOX_MCP_PORT=$mcp_port
CHARIOX_CODEX_PORT_RANGE=$codex_port_start-$codex_port_end
CHARIOX_CODEX_BIND_HOST=127.0.0.1
CHARIOX_CODEX_BIN=$probe_wrapper
CHARIOX_MANAGED_ISOLATION_REAL_PROVIDER=$real_provider
CHARIOX_MANAGED_ISOLATION_PROBE_WORKSPACE=$workspace
CHARIOX_MANAGED_ISOLATION_PROBE_RESULT=$result_path
CHARIOX_MANAGED_ISOLATION_ASSERT_MODE=$isolation_assert_mode
CHARIOX_MANAGED_ISOLATION_REQUIRE_NESTED_USERNS_DENIED=$nested_userns_required
CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT=1
CHARIOX_ACCEPT_REMOTE_LEASES=0
CHARIOX_OS_NAME=Linux supervisor A/B probe
PATH=/usr/local/bin:/usr/bin:/bin
EOF
  chown chariox:chariox "$supervisor_env"
  chmod 0600 "$supervisor_env"
}

capture_unit_evidence() {
  "$SYSTEMCTL" show "$unit" \
    -p Id \
    -p MainPID \
    -p ActiveState \
    -p SubState \
    -p User \
    -p Group \
    -p SupplementaryGroups \
    -p WorkingDirectory \
    -p NoNewPrivileges \
    -p PrivateTmp \
    -p ProtectSystem \
    -p ProtectHome \
    -p ProtectKernelTunables \
    -p ProtectKernelModules \
    -p ProtectControlGroups \
    -p RestrictSUIDSGID \
    -p RestrictAddressFamilies \
    -p ReadWritePaths \
    -p KillMode \
    -p UMask \
    >"$unit_properties" 2>/dev/null || true
  chmod 0600 "$unit_properties" 2>/dev/null || true

  local main_pid
  main_pid="$("$SYSTEMCTL" show "$unit" -p MainPID --value 2>/dev/null || true)"
  {
    printf 'main_pid=%s\n' "$main_pid"
    if [[ "$main_pid" =~ ^[1-9][0-9]*$ && -r "/proc/$main_pid/status" ]]; then
      grep -E '^(Uid|Gid|NoNewPrivs|Seccomp|Seccomp_filters):' "/proc/$main_pid/status" || true
      if [[ -r "/proc/$main_pid/cmdline" ]]; then
        printf 'cmdline='
        tr '\0' ' ' <"/proc/$main_pid/cmdline" || true
        printf '\n'
      fi
    fi
  } >"$runtime_status"
  chmod 0600 "$runtime_status"
}

start_case_kernel() {
  local kernel="$1"
  local kernel_start_command_status=0
  local state="unknown"
  local attempt
  local -a properties=(
    "--property=User=chariox"
    "--property=Group=chariox"
    "--property=SupplementaryGroups=chariox-slice"
    "--property=WorkingDirectory=$chariox_home"
    "--property=NoNewPrivileges=true"
    "--property=PrivateTmp=true"
    "--property=ProtectSystem=strict"
    "--property=ProtectHome=true"
    "--property=ProtectKernelTunables=false"
    "--property=ProtectKernelModules=true"
    "--property=ProtectControlGroups=true"
    "--property=RestrictSUIDSGID=true"
    "--property=RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6"
    "--property=ReadWritePaths=$case_root"
    "--property=InaccessiblePaths=/var/lib/chariox /var/lib/chariox-slice-share"
    "--property=KillMode=control-group"
    "--property=TimeoutStartSec=30s"
    "--property=TimeoutStopSec=15s"
    "--property=UMask=0007"
    "--property=StandardOutput=journal"
    "--property=StandardError=journal"
  )
  local -a environment=(
    "--setenv=HOME=$chariox_home"
    "--setenv=CHARIOX_HOME=$chariox_home"
    "--setenv=CHARIOX_CAPABILITY_ISOLATION_ROOT=$capability_root"
    "--setenv=CHARIOX_MANAGED_PROVIDER_ISOLATION=1"
    "--setenv=CHARIOX_MANAGED_PROVIDER_HOME=$provider_home"
    "--setenv=CHARIOX_MANAGED_SLICE_SERVICE_ROOT=$service_root"
    "--setenv=CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT=$publication_root"
    "--setenv=CHARIOX_MANAGED_VAULT_PATH=$vault_path"
    "--setenv=CHARIOX_MANAGED_BOOTSTRAP_PATH=$bootstrap_path"
    "--setenv=CHARIOX_MANAGED_RELEASE_MANIFEST=/usr/lib/chariox/release-manifest.json"
    "--setenv=CHARIOX_MANAGED_RELEASE_SIGNATURE=/usr/lib/chariox/release-manifest.sig"
    "--setenv=CHARIOX_MANAGED_RELEASE_PUBLIC_KEY=/usr/lib/chariox/release-public-key"
    "--setenv=CHARIOX_MANAGED_KERNEL_BINARY=/usr/local/bin/chariox-kernel"
    "--setenv=CHARIOX_SLICE_DOCKER_BROKER_SOCKET=$broker_socket"
    "--setenv=CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE=$auth_file"
    "--setenv=CHARIOX_KERNEL_HOST=127.0.0.1"
    "--setenv=CHARIOX_KERNEL_PORT=$kernel_port"
    "--setenv=CHARIOX_MCP_HOST=127.0.0.1"
    "--setenv=CHARIOX_MCP_PORT=$mcp_port"
    "--setenv=CHARIOX_CODEX_PORT_RANGE=$codex_port_start-$codex_port_end"
    "--setenv=CHARIOX_CODEX_BIND_HOST=127.0.0.1"
    "--setenv=CHARIOX_CODEX_BIN=$probe_wrapper"
    "--setenv=CHARIOX_MANAGED_ISOLATION_REAL_PROVIDER=$real_provider"
    "--setenv=CHARIOX_MANAGED_ISOLATION_PROBE_WORKSPACE=$workspace"
    "--setenv=CHARIOX_MANAGED_ISOLATION_PROBE_RESULT=$result_path"
    "--setenv=CHARIOX_MANAGED_ISOLATION_ASSERT_MODE=$isolation_assert_mode"
    "--setenv=CHARIOX_MANAGED_ISOLATION_REQUIRE_NESTED_USERNS_DENIED=$nested_userns_required"
    "--setenv=CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT=1"
    "--setenv=CHARIOX_ACCEPT_REMOTE_LEASES=0"
    "--setenv=CHARIOX_OS_NAME=Linux supervisor A/B probe"
    "--setenv=PATH=/usr/local/bin:/usr/bin:/bin"
  )

  active_unit="$unit"
  if /usr/bin/env -i PATH=/usr/bin:/bin SYSTEMD_COLORS=0 \
    "$SYSTEMD_RUN" --unit="$unit" --collect --service-type=exec \
    "${properties[@]}" "${environment[@]}" -- "$kernel" \
    >"$start_log" 2>&1; then
    kernel_start_command_status=0
  else
    kernel_start_command_status=$?
  fi

  if [[ "$kernel_start_command_status" != "0" ]]; then
    capture_unit_evidence
    printf 'systemd-run failed before kernel readiness (exit=%s)\n' \
      "$kernel_start_command_status" >&2
    return "$kernel_start_command_status"
  fi

  for ((attempt = 0; attempt < 300; attempt += 1)); do
    state="$("$SYSTEMCTL" show "$unit" -p ActiveState --value 2>/dev/null || true)"
    if [[ "$state" == "active" && ! -e "$auth_file" ]]; then
      capture_unit_evidence
      return 0
    fi
    case "$state" in
      failed|inactive|deactivating) break ;;
    esac
    sleep 0.1
  done

  capture_unit_evidence
  printf 'kernel start did not reach active/auth-consumed state (systemd-run=%s state=%s)\n' \
    "$kernel_start_command_status" "$state" >&2
  return 1
}

sanitize_probe_stream() {
  LC_ALL=C sed -E \
    -e 's/(api[_-]?key|token|secret|password|credential(s)?|cookie|prompt|authorization)[[:space:]]*[:=][^[:space:];]*/\1=[redacted]/Ig' \
    -e 's/(Bearer|Basic)[[:space:]][^[:space:]]*/\1 [redacted]/Ig' \
    | LC_ALL=C tr -c '[:print:]\t\r\n' ' '
}

run_client_probe() {
  local client_home="$case_root/client-home"
  install -d -o root -g root -m 0700 "$client_home"
  local probe_status=0
  if /usr/bin/env -i \
    PATH=/usr/local/bin:/usr/bin:/bin \
    HOME="$client_home" \
    CHARIOX_KERNEL_URL="ws://127.0.0.1:$kernel_port" \
    CHARIOX_KERNEL_LOCAL_AUTH_TOKEN="$token" \
    CHARIOX_MANAGED_ISOLATION_PROBE_WORKSPACE="$workspace" \
    CHARIOX_MANAGED_ISOLATION_PROBE_WORKTREE="$workspace" \
    CHARIOX_MANAGED_ISOLATION_PROBE_RESULT="$result_path" \
    CHARIOX_MANAGED_ISOLATION_PROBE_CAPTURE="$launch_capture" \
    CHARIOX_MANAGED_ISOLATION_PROBE_NATIVE_TUI="$native_tui" \
    CHARIOX_MANAGED_ISOLATION_REQUIRE_NESTED_USERNS_DENIED="$nested_userns_required" \
    CHARIOX_MANAGED_ISOLATION_PROBE_ACCOUNT=default \
    CHARIOX_MANAGED_ISOLATION_PROBE_MODEL=gpt-5.4 \
    CHARIOX_PROBE_PACKAGE_JSON="$probe_package" \
    CHARIOX_PROBE_TIMEOUT_MS="$probe_timeout_ms" \
    "$NODE" "$probe_script" 2>&1 | sanitize_probe_stream >"$probe_log"; then
    probe_status="${PIPESTATUS[0]}"
  else
    probe_status="${PIPESTATUS[0]}"
  fi
  chmod 0600 "$probe_log" 2>/dev/null || true
  return "$probe_status"
}

stop_case_kernel() {
  if [[ -z "$active_unit" ]]; then return 0; fi
  local unit_to_stop="$active_unit"
  if ! timeout 20 "$SYSTEMCTL" stop "$unit_to_stop" >/dev/null 2>&1; then
    timeout 5 "$SYSTEMCTL" kill --kill-who=all "$unit_to_stop" >/dev/null 2>&1 || true
    timeout 10 "$SYSTEMCTL" stop "$unit_to_stop" >/dev/null 2>&1 || true
  fi
  active_unit=""
}

capture_kernel_journal() {
  if [[ -n "$JOURNALCTL" ]]; then
    "$JOURNALCTL" -u "$unit" --no-pager -o cat 2>/dev/null \
      | sanitize_probe_stream >"$kernel_log" || true
  else
    : >"$kernel_log"
  fi
  chmod 0600 "$kernel_log" 2>/dev/null || true
}

print_case_evidence() {
  printf '\n[%s] kernel=%s\n' "$case_label" "$kernel_under_test"
  printf 'probe_exit=%s kernel_start_exit=%s ports=%s,%s codex=%s-%s\n' \
    "$probe_status" "$kernel_start_status" "$kernel_port" "$mcp_port" \
    "$codex_port_start" "$codex_port_end"
  printf 'evidence_dir=%s\n' "$case_root"
  printf 'probe_context_source=%s\nprobe_context_staged=%s\n' \
    "$probe_context_source" "$probe_context_stage"
  printf 'unit=%s\n' "$unit"
  printf 'launch_capture=%s\n' "$launch_capture"
  if [[ -s "$launch_capture" ]]; then
    sed -n '1,260p' "$launch_capture"
  else
    printf '%s\n' 'launch capture was not written'
  fi
  printf 'probe_result=%s\n' "$result_path"
  if [[ -s "$result_path" ]]; then
    sanitize_probe_stream <"$result_path" | sed -n '1,80p'
  fi
  printf 'sanitized_probe_log=%s\n' "$probe_log"
  if [[ -s "$probe_log" ]]; then tail -n 160 "$probe_log"; fi
  printf 'unit_properties=%s\nruntime_status=%s\nkernel_log=%s\n' \
    "$unit_properties" "$runtime_status" "$kernel_log"
  if [[ -s "$runtime_status" ]]; then cat "$runtime_status"; fi
  if [[ -s "$kernel_log" ]]; then tail -n 100 "$kernel_log"; fi
}

run_case() {
  case_label="$1"
  kernel_under_test="$2"
  nested_userns_required="$3"
  isolation_assert_mode="$4"
  case "$isolation_assert_mode" in
    baseline|strict) ;;
    *) printf 'invalid isolation assertion mode: %s\n' "$isolation_assert_mode" >&2; return 2 ;;
  esac
  prepare_case "$case_label"

  kernel_start_status=0
  if start_case_kernel "$kernel_under_test"; then
    kernel_start_status=0
  else
    kernel_start_status=$?
  fi

  probe_status=0
  if [[ "$kernel_start_status" == "0" ]]; then
    if run_client_probe; then
      probe_status=0
    else
      probe_status=$?
    fi
  else
    probe_status=125
    printf 'skipping client probe because kernel start failed (exit=%s)\n' \
      "$kernel_start_status" >&2
    printf 'client probe skipped: kernel start failed (exit=%s)\n' \
      "$kernel_start_status" >"$probe_log"
    chmod 0600 "$probe_log"
  fi
  stop_case_kernel
  capture_kernel_journal
  print_case_evidence

  case_status="$probe_status"
  if [[ "$case_status" == "0" && "$kernel_start_status" != "0" ]]; then
    case_status="$kernel_start_status"
  fi
  if [[ ! -s "$launch_capture" ]]; then
    case_status=1
  fi
  last_case_root="$case_root"
  return "$case_status"
}

old_status=0
candidate_status=0
old_case_root=""
candidate_case_root=""

if run_case old "$rollback_kernel" 0 baseline; then
  old_status=0
else
  old_status=$?
fi
old_case_root="$last_case_root"

if run_case candidate "$candidate_kernel" 1 strict; then
  candidate_status=0
else
  candidate_status=$?
fi
candidate_case_root="$last_case_root"

if [[ -s "$old_case_root/launch-capture.json" && -s "$candidate_case_root/launch-capture.json" ]]; then
  diff -u "$old_case_root/launch-capture.json" "$candidate_case_root/launch-capture.json" \
    >"$probe_root/launch-capture.diff" || true
  chmod 0600 "$probe_root/launch-capture.diff"
  printf '\nlaunch_capture_diff=%s\n' "$probe_root/launch-capture.diff"
fi

printf '\nA/B summary: rollback_probe_exit=%s candidate_probe_exit=%s\n' \
  "$old_status" "$candidate_status"
printf 'probe_context_source=%s\nprobe_context_staged=%s\n' \
  "$probe_context_source" "$probe_context_stage"
printf 'evidence_root=%s\n' "$probe_root"
if [[ "$keep_evidence" == "1" ]]; then
  printf '%s\n' 'evidence is retained; remove only this printed directory when review is complete'
fi

if [[ "$old_status" != "0" || "$candidate_status" != "0" ]]; then
  exit 1
fi
