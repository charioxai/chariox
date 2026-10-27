#!/usr/bin/env bash
set -euo pipefail
umask 077

deployment_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
project_name="${CHARIOX_AEGS_COMPOSE_PROJECT_NAME:-chariox-aegs-dummy-staging}"
env_file="${CHARIOX_AEGS_ENV_FILE:-/etc/chariox/aegs-dummy.env}"
backup_dir="${CHARIOX_AEGS_BACKUP_DIR:-${deployment_dir}/backups}"
role_marker="${CHARIOX_AEGS_HOST_ROLE_MARKER:-/etc/chariox/event-publication/host-role}"
compose=(docker compose --env-file "${env_file}" --project-name "${project_name}" \
  --project-directory "${deployment_dir}" -f "${deployment_dir}/compose.yaml")
# shellcheck source=deploy/event-publication/remote-aegs-dummy/operation-common.sh
source "${deployment_dir}/operation-common.sh"
restart_required=false
backup_path=""
backup_complete=false
reservation_path=""
runtime_touched=false
interrupted_signal=""

fail() { printf '%s\n' "$1" >&2; exit 1; }
path_mode() { stat -c '%a' "$1" 2>/dev/null || stat -f '%Lp' "$1"; }
path_uid() { stat -c '%u' "$1" 2>/dev/null || stat -f '%u' "$1"; }
checksum_create() {
  if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi
}
prepare_backup_dir() {
  [[ "${backup_dir}" == /* ]] || fail "Backup directory must be absolute"
  [[ ! -L "${backup_dir}" ]] || fail "Backup directory must not be a symlink"
  local leaf parent
  leaf="$(basename "${backup_dir}")"
  parent="$(dirname "${backup_dir}")"
  [[ "${leaf}" != "." && "${leaf}" != ".." && "${leaf}" != "/" ]] \
    || fail "Backup directory must be a dedicated child directory"
  [[ -d "${parent}" && ! -L "${parent}" ]] \
    || fail "Backup directory parent must be an existing non-symlink directory"
  parent="$(cd "${parent}" && pwd -P)"
  backup_dir="${parent%/}/${leaf}"
  case "${backup_dir}" in
    /|/bin|/boot|/dev|/etc|/home|/lib|/lib64|/opt|/proc|/root|/run|/sbin|/srv|/sys|/tmp|/usr|/var|"${deployment_dir}")
      fail "Backup directory must be a dedicated child directory"
      ;;
  esac
  if [[ ! -e "${backup_dir}" ]]; then
    mkdir -- "${backup_dir}"
    chmod 0700 "${backup_dir}"
  fi
  [[ -d "${backup_dir}" && ! -L "${backup_dir}" ]] \
    || fail "Backup directory must be a non-symlink directory"
  backup_dir="$(cd "${backup_dir}" && pwd -P)"
  [[ "$(path_uid "${backup_dir}")" == "${EUID}" ]] \
    || fail "Backup directory must be owned by the invoking user"
  [[ "$(path_mode "${backup_dir}")" == "700" ]] \
    || fail "Backup directory must already be mode 0700"
}
restart_aegs() { "${compose[@]}" start --wait dummy-aegs >/dev/null; }
handle_signal() {
  interrupted_signal="$1"
  exit "$2"
}
on_exit() {
  local status=$? runtime_status
  trap - EXIT HUP INT TERM
  if [[ -n "${interrupted_signal}" && "${runtime_touched}" == true ]]; then
    if ensure_stopped; then runtime_status=0; else runtime_status=$?; fi
    if [[ "${runtime_status}" -eq 2 ]]; then
      printf '%s\n' "${interrupted_signal} interrupted backup; Docker is unavailable and runtime state is unknown" >&2
    elif [[ "${runtime_status}" -eq 0 ]]; then
      printf '%s\n' "${interrupted_signal} interrupted backup; workload was verified stopped" >&2
    else
      printf '%s\n' "${interrupted_signal} interrupted backup; workload could not be verified stopped" >&2
    fi
    status=1
  elif [[ "${restart_required}" == true ]]; then
    if ! restart_aegs; then
      printf '%s\n' "AEGS restart failed; workload remains stopped or unhealthy" >&2
      status=1
    fi
  fi
  if [[ "${status}" -ne 0 && "${backup_complete}" == false && -n "${backup_path}" ]]; then
    rm -f -- "${backup_path}" "${backup_path}-wal" "${backup_path}.sha256" \
      "${backup_path}.partial" "${backup_path}-wal.partial" \
      "${backup_path}.sha256.partial"
  fi
  if ! operation_lock_release; then status=1; fi
  exit "${status}"
}
trap on_exit EXIT
trap 'handle_signal HUP 129' HUP
trap 'handle_signal INT 130' INT
trap 'handle_signal TERM 143' TERM

[[ -f "${role_marker}" && ! -L "${role_marker}" && "$(<"${role_marker}")" == "aegs" ]] \
  || fail "AEGS host role marker is missing or unsafe"
operation_lock_acquire backup || fail "Could not acquire the AEGS backup/restore operation lock"
prepare_backup_dir

volumes=()
while IFS= read -r volume; do
  [[ -n "${volume}" ]] && volumes+=("${volume}")
done < <(docker volume ls --filter "label=com.docker.compose.project=${project_name}" \
  --filter label=com.docker.compose.volume=aegs-data --quiet)
[[ "${#volumes[@]}" -eq 1 ]] \
  || fail "Expected exactly one owned AEGS volume, found ${#volumes[@]}"

image="$("${compose[@]}" images --quiet dummy-aegs)"
[[ -n "${image}" && "${image}" != *$'\n'* ]] \
  || fail "Expected exactly one available AEGS image; start the deployment first"
backup_timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
for _attempt in {0..99}; do
  backup_name="dummy-${backup_timestamp}-$$-$(printf '%05d' "${_attempt}").db"
  backup_path="${backup_dir}/${backup_name}"
  reservation_path="${backup_path}.reserve"
  stem_paths=(
    "${backup_path}" "${backup_path}.partial"
    "${backup_path}-wal" "${backup_path}-wal.partial"
    "${backup_path}.sha256" "${backup_path}.sha256.partial"
    "${reservation_path}"
  )
  stem_available=true
  for stem_path in "${stem_paths[@]}"; do
    if [[ -e "${stem_path}" || -L "${stem_path}" ]]; then
      stem_available=false
      break
    fi
  done
  [[ "${stem_available}" == true ]] || continue
  if ! mkdir -- "${reservation_path}" 2>/dev/null; then continue; fi
  collision=false
  for stem_path in "${stem_paths[@]:0:6}"; do
    if [[ -e "${stem_path}" || -L "${stem_path}" ]]; then collision=true; break; fi
  done
  if [[ "${collision}" == true ]]; then
    backup_path=""
    reservation_path=""
    continue
  fi
  if (set -o noclobber; : >"${backup_path}") 2>/dev/null; then break; fi
  backup_path=""
  reservation_path=""
done
[[ -n "${backup_path}" ]] || fail "Could not reserve a unique backup filename"

restart_required=true
runtime_touched=true
if ensure_stopped; then
  runtime_status=0
else
  runtime_status=$?
fi
if [[ "${runtime_status}" -eq 2 ]]; then
  fail "Docker is unavailable and backup runtime state is unknown"
elif [[ "${runtime_status}" -ne 0 ]]; then
  fail "Dummy AEGS could not be verified stopped; backup copy was not started"
fi
docker run --rm --user 0 --volume "${volumes[0]}:/source:ro" \
  --volume "${backup_dir}:/backup" --entrypoint sh "${image}" -ceu '
    test -s /source/aegs.db
    test -f "/backup/$1" && test ! -s "/backup/$1"
    test ! -e "/backup/$1-wal" && test ! -L "/backup/$1-wal"
    cp /source/aegs.db "/backup/$1.partial"
    chmod 600 "/backup/$1.partial"
    mv "/backup/$1.partial" "/backup/$1"
    if test -s /source/aegs.db-wal; then
      cp /source/aegs.db-wal "/backup/$1-wal.partial"
      chmod 600 "/backup/$1-wal.partial"
      mv "/backup/$1-wal.partial" "/backup/$1-wal"
    fi
  ' -- "${backup_name}"

(
  cd "${backup_dir}"
  files=("${backup_name}")
  [[ -s "${backup_name}-wal" ]] && files+=("${backup_name}-wal")
  checksum_create "${files[@]}" >"${backup_name}.sha256.partial"
  chmod 0600 "${backup_name}.sha256.partial"
  mv "${backup_name}.sha256.partial" "${backup_name}.sha256"
)
backup_complete=true
rmdir -- "${reservation_path}"
reservation_path=""
restart_aegs
restart_required=false
operation_lock_release
trap - EXIT HUP INT TERM
printf '%s\n' "${backup_path}"
