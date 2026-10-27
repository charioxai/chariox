#!/usr/bin/env bash

operation_lock_owned=false
operation_lock_dir="${CHARIOX_AEGS_OPERATION_LOCK_DIR:-/run/lock/chariox-aegs-dummy-backup-restore.lock}"
operation_lock_owner="${operation_lock_dir}/owner"

operation_lock_acquire() {
  local action="$1" owner_partial parent
  [[ "${operation_lock_dir}" == /* && ! -L "${operation_lock_dir}" ]] \
    || { printf '%s\n' "Operation lock path must be absolute and not a symlink" >&2; return 1; }
  parent="$(dirname "${operation_lock_dir}")"
  [[ -d "${parent}" && ! -L "${parent}" ]] \
    || { printf '%s\n' "Operation lock parent must be an existing non-symlink directory" >&2; return 1; }
  if ! mkdir -- "${operation_lock_dir}" 2>/dev/null; then
    printf '%s\n' "Another AEGS backup or restore operation holds ${operation_lock_dir}" >&2
    return 1
  fi
  operation_lock_owned=true
  chmod 0700 "${operation_lock_dir}"
  owner_partial="${operation_lock_owner}.partial.$$"
  printf 'pid=%s\naction=%s\nstarted=%s\n' \
    "$$" "${action}" "$(date -u +%Y%m%dT%H%M%SZ)" >"${owner_partial}"
  chmod 0600 "${owner_partial}"
  mv -- "${owner_partial}" "${operation_lock_owner}"
}

operation_lock_release() {
  local key value owner_pid=""
  [[ "${operation_lock_owned}" == true ]] || return 0
  if [[ -f "${operation_lock_owner}" && ! -L "${operation_lock_owner}" ]]; then
    while IFS='=' read -r key value; do
      [[ "${key}" != pid ]] || owner_pid="${value}"
    done <"${operation_lock_owner}"
    [[ "${owner_pid}" == "$$" ]] \
      || { printf '%s\n' "Operation lock ownership changed; lock was not removed" >&2; return 1; }
    rm -f -- "${operation_lock_owner}"
  fi
  rm -f -- "${operation_lock_owner}.partial.$$"
  rmdir -- "${operation_lock_dir}"
  operation_lock_owned=false
}

runtime_is_stopped() {
  local running
  if ! running="$(docker ps \
    --filter "label=com.docker.compose.project=${project_name}" \
    --filter label=com.docker.compose.service=dummy-aegs --quiet)"; then
    return 2
  fi
  [[ -z "${running}" ]]
}

ensure_stopped() {
  local runtime_status
  "${compose[@]}" stop dummy-aegs >/dev/null 2>&1 || true
  if runtime_is_stopped; then return 0; else runtime_status=$?; fi
  "${compose[@]}" kill dummy-aegs >/dev/null 2>&1 || true
  if runtime_is_stopped; then return 0; else runtime_status=$?; fi
  return "${runtime_status}"
}
