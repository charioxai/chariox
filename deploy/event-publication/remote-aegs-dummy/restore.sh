#!/usr/bin/env bash
set -euo pipefail

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
restart_allowed=true
runtime_touched=false
interrupted_signal=""

fail() { printf '%s\n' "$1" >&2; exit 1; }
path_mode() { stat -c '%a' "$1" 2>/dev/null || stat -f '%Lp' "$1"; }
path_uid() { stat -c '%u' "$1" 2>/dev/null || stat -f '%u' "$1"; }
checksum_check() {
  if command -v sha256sum >/dev/null; then sha256sum --check "$1"; else shasum -a 256 --check "$1"; fi
}
validate_backup_dir() {
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
  [[ -d "${backup_dir}" && ! -L "${backup_dir}" ]] \
    || fail "Backup directory must be an existing non-symlink directory"
  case "${backup_dir}" in
    /|/bin|/boot|/dev|/etc|/home|/lib|/lib64|/opt|/proc|/root|/run|/sbin|/srv|/sys|/tmp|/usr|/var|"${deployment_dir}")
      fail "Backup directory must be a dedicated child directory"
      ;;
  esac
  [[ "$(path_uid "${backup_dir}")" == "${EUID}" ]] \
    || fail "Backup directory must be owned by the invoking user"
  [[ "$(path_mode "${backup_dir}")" == "700" ]] \
    || fail "Backup directory must be mode 0700"
}
validate_backup_file() {
  local path="$1"
  [[ -f "${path}" && ! -L "${path}" ]] || fail "Backup artifact is missing or unsafe: ${path}"
  [[ -s "${path}" ]] || fail "Backup artifact must not be empty: ${path}"
  [[ "$(path_uid "${path}")" == "${EUID}" ]] \
    || fail "Backup artifact must be owned by the invoking user: ${path}"
  [[ "$(path_mode "${path}")" == "600" ]] \
    || fail "Backup artifact must be mode 0600: ${path}"
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
      printf '%s\n' "${interrupted_signal} interrupted restore; Docker is unavailable and runtime state is unknown; checkpoints were retained" >&2
    elif [[ "${runtime_status}" -eq 0 ]]; then
      printf '%s\n' "${interrupted_signal} interrupted restore; workload was verified stopped and checkpoints were retained" >&2
    else
      printf '%s\n' "${interrupted_signal} interrupted restore; workload could not be verified stopped and checkpoints were retained" >&2
    fi
    status=1
  elif [[ "${restart_required}" == true ]]; then
    if [[ "${restart_allowed}" == true ]]; then
      if ! restart_aegs; then
        if ensure_stopped; then runtime_status=0; else runtime_status=$?; fi
        if [[ "${runtime_status}" -eq 2 ]]; then
          printf '%s\n' "AEGS restart failed; Docker is unavailable and runtime state is unknown" >&2
        else
          printf '%s\n' "AEGS restart failed; workload was stopped for manual recovery" >&2
        fi
        status=1
      fi
    else
      printf '%s\n' "AEGS restore state is ambiguous; workload intentionally left stopped" >&2
      status=1
    fi
  fi
  if ! operation_lock_release; then status=1; fi
  exit "${status}"
}
trap on_exit EXIT
trap 'handle_signal HUP 129' HUP
trap 'handle_signal INT 130' INT
trap 'handle_signal TERM 143' TERM

[[ "${1:-}" == "--yes" && -n "${2:-}" && "${2:-}" != -* && -z "${3:-}" ]] \
  || fail "Usage: $0 --yes <backup filename>"
backup_name="${2}"
[[ "${backup_name}" =~ ^dummy-[0-9]{8}T[0-9]{6}Z-[0-9]+-[0-9]{5}\.db$ ]] \
  || fail "Backup must be a generated filename from ${backup_dir}"
[[ -f "${role_marker}" && ! -L "${role_marker}" && "$(<"${role_marker}")" == "aegs" ]] \
  || fail "AEGS host role marker is missing or unsafe"
validate_backup_dir
operation_lock_acquire restore || fail "Could not acquire the AEGS backup/restore operation lock"
backup_path="${backup_dir}/${backup_name}"
manifest_path="${backup_path}.sha256"
validate_backup_file "${backup_path}"
validate_backup_file "${manifest_path}"
wal_present=false
if [[ -e "${backup_path}-wal" || -L "${backup_path}-wal" ]]; then
  validate_backup_file "${backup_path}-wal"
  wal_present=true
fi

manifest_files=()
while read -r digest filename extra; do
  [[ -z "${extra:-}" && "${digest}" =~ ^[[:xdigit:]]{64}$ && -n "${filename:-}" ]] \
    || fail "Checksum manifest has an invalid entry"
  manifest_files+=("${filename}")
done <"${manifest_path}"
expected_files=("${backup_name}")
[[ "${wal_present}" == true ]] && expected_files+=("${backup_name}-wal")
[[ "${manifest_files[*]}" == "${expected_files[*]}" ]] \
  || fail "Checksum manifest does not exactly cover the backup file set"
(cd "${backup_dir}" && checksum_check "${backup_name}.sha256")

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
restore_timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
restart_required=true
runtime_touched=true
if ensure_stopped; then
  runtime_status=0
else
  runtime_status=$?
fi
if [[ "${runtime_status}" -eq 2 ]]; then
  restart_required=false
  fail "Docker is unavailable and restore runtime state is unknown; SQLite was not modified"
elif [[ "${runtime_status}" -ne 0 ]]; then
  restart_required=false
  fail "Dummy AEGS could not be verified stopped; SQLite was not modified"
fi
restore_status=78
restore_token=""
for restore_attempt in {0..99}; do
  candidate_token="${restore_timestamp}-$$-$(printf '%05d' "${restore_attempt}")"
  set +e
  docker run --rm --user 0 --volume "${volumes[0]}:/target" \
    --volume "${backup_dir}:/backup:ro" --entrypoint sh "${image}" -ceu '
    backup_name=$1
    token=$2
    namespace="/target/.aegs.restore-$token"
    stage_db="$namespace/stage.db"
    stage_wal="$namespace/stage.db-wal"
    previous_db="$namespace/previous.db"
    previous_wal="$namespace/previous.db-wal"
    previous_shm="$namespace/previous.db-shm"
    state="$namespace/state"
    state_partial="$namespace/state.partial"
    namespace_owned=0
    swap_started=0
    restore_complete=0
    had_db=0
    had_wal=0
    had_shm=0

    finish() {
      status=$?
      trap - EXIT HUP INT TERM
      if test "$status" -eq 0; then exit 0; fi
      if test "$namespace_owned" -eq 0; then exit "$status"; fi
      rollback_ok=1
      if test "$swap_started" -eq 1 && test "$restore_complete" -eq 0; then
        restore_previous() {
          expected=$1
          previous=$2
          live=$3
          if test "$expected" -eq 0; then
            rm -f "$live" || rollback_ok=0
          elif test -e "$previous"; then
            rm -f "$live" || rollback_ok=0
            mv "$previous" "$live" || rollback_ok=0
          elif ! test -e "$live"; then
            rollback_ok=0
          fi
        }
        restore_previous "$had_db" "$previous_db" /target/aegs.db
        restore_previous "$had_wal" "$previous_wal" /target/aegs.db-wal
        restore_previous "$had_shm" "$previous_shm" /target/aegs.db-shm
      fi
      rm -f "$stage_db" "$stage_wal" "$state_partial" || rollback_ok=0
      if test "$rollback_ok" -eq 1; then
        rm -f "$state" || rollback_ok=0
        rmdir "$namespace" || rollback_ok=0
      fi
      if test "$rollback_ok" -eq 1; then exit 75; else exit 76; fi
    }
    trap finish EXIT
    trap "exit 74" HUP INT TERM

    if ! mkdir "$namespace" 2>/dev/null; then exit 78; fi
    namespace_owned=1
    chmod 700 "$namespace"
    install -o 10001 -g 10001 -m 600 "/backup/$backup_name" "$stage_db"
    if test -s "/backup/$backup_name-wal"; then
      install -o 10001 -g 10001 -m 600 "/backup/$backup_name-wal" "$stage_wal"
    fi

    if test -e /target/aegs.db; then had_db=1; fi
    if test -e /target/aegs.db-wal; then had_wal=1; fi
    if test -e /target/aegs.db-shm; then had_shm=1; fi
    printf "%s %s %s\n" "$had_db" "$had_wal" "$had_shm" >"$state_partial"
    chmod 600 "$state_partial"
    mv "$state_partial" "$state"
    swap_started=1
    if test -e /target/aegs.db; then mv /target/aegs.db "$previous_db"; fi
    if test -e /target/aegs.db-wal; then mv /target/aegs.db-wal "$previous_wal"; fi
    if test -e /target/aegs.db-shm; then mv /target/aegs.db-shm "$previous_shm"; fi
    mv "$stage_db" /target/aegs.db
    if test -e "$stage_wal"; then mv "$stage_wal" /target/aegs.db-wal; fi
    restore_complete=1
  ' -- "${backup_name}" "${candidate_token}"
  restore_status=$?
  set -e
  if [[ "${restore_status}" -eq 78 ]]; then continue; fi
  restore_token="${candidate_token}"
  break
done
if [[ "${restore_status}" -eq 0 ]]; then
  restart_allowed=false
elif [[ "${restore_status}" -eq 75 ]]; then
  restart_allowed=true
  fail "Restore failed and the prior database was rolled back"
elif [[ "${restore_status}" -eq 78 ]]; then
  restart_allowed=true
  fail "Could not reserve a unique restore namespace"
else
  restart_allowed=false
  fail "Restore failed without a verified rollback (container status ${restore_status})"
fi

checkpoint_action() {
  local action="$1"
  docker run --rm --user 0 --volume "${volumes[0]}:/target" \
    --entrypoint sh "${image}" -ceu '
      action=$1
      token=$2
      namespace="/target/.aegs.restore-$token"
      previous_db="$namespace/previous.db"
      previous_wal="$namespace/previous.db-wal"
      previous_shm="$namespace/previous.db-shm"
      state="$namespace/state"
      stage_db="$namespace/stage.db"
      stage_wal="$namespace/stage.db-wal"
      trap "exit 79" HUP INT TERM
      test -d "$namespace" && test ! -L "$namespace"
      test -f "$state" && test ! -L "$state"
      IFS=" " read -r had_db had_wal had_shm extra <"$state"
      test -z "${extra:-}"
      case "$had_db:$had_wal:$had_shm" in
        0:0:0|0:0:1|0:1:0|0:1:1|1:0:0|1:0:1|1:1:0|1:1:1) ;;
        *) exit 77 ;;
      esac
      if test "$action" = commit; then
        rm -f "$stage_db" "$stage_wal" "$previous_db" "$previous_wal" \
          "$previous_shm"
        rm -f "$state"
        rmdir "$namespace"
        exit 0
      fi
      test "$action" = rollback
      verify_previous() {
        expected=$1
        previous=$2
        if test "$expected" -eq 1; then test -e "$previous"; else test ! -e "$previous"; fi
      }
      verify_previous "$had_db" "$previous_db"
      verify_previous "$had_wal" "$previous_wal"
      verify_previous "$had_shm" "$previous_shm"
      restore_previous() {
        expected=$1
        previous=$2
        live=$3
        rm -f "$live"
        if test "$expected" -eq 1; then mv "$previous" "$live"; fi
      }
      restore_previous "$had_db" "$previous_db" /target/aegs.db
      restore_previous "$had_wal" "$previous_wal" /target/aegs.db-wal
      restore_previous "$had_shm" "$previous_shm" /target/aegs.db-shm
      rm -f "$stage_db" "$stage_wal" "$state"
      rmdir "$namespace"
    ' -- "${action}" "${restore_token}"
}

if restart_aegs; then
  restart_required=false
  checkpoint_action commit \
    || fail "Restored workload is healthy, but its prior-state checkpoints need manual cleanup"
  operation_lock_release
  trap - EXIT HUP INT TERM
  printf 'Restored %s\n' "${backup_path}"
  exit 0
fi

if ensure_stopped; then
  runtime_status=0
else
  runtime_status=$?
fi
if [[ "${runtime_status}" -eq 2 ]]; then
  restart_required=false
  fail "Restored workload failed health; Docker is unavailable, runtime state is unknown, and checkpoints were retained"
elif [[ "${runtime_status}" -ne 0 ]]; then
  restart_required=false
  fail "Restored workload failed health and could not be verified stopped; checkpoints were retained"
fi
if ! checkpoint_action rollback; then
  if ensure_stopped; then
    runtime_status=0
  else
    runtime_status=$?
  fi
  restart_required=false
  if [[ "${runtime_status}" -eq 2 ]]; then
    fail "Prior-state rollback failed; Docker is unavailable, runtime state is unknown, and checkpoints were retained"
  elif [[ "${runtime_status}" -eq 0 ]]; then
    fail "Restored workload failed health and prior-state rollback could not be verified; workload was stopped"
  else
    fail "Restored workload failed health and neither rollback nor stopped state could be verified"
  fi
fi
restart_allowed=true
if restart_aegs; then
  restart_required=false
  fail "Restored workload failed health; the prior database state is healthy again"
fi
if ensure_stopped; then
  runtime_status=0
else
  runtime_status=$?
fi
restart_required=false
if [[ "${runtime_status}" -eq 2 ]]; then
  fail "Prior database state was restored but failed health; Docker is unavailable and runtime state is unknown"
elif [[ "${runtime_status}" -eq 0 ]]; then
  fail "Prior database state was restored but did not pass health; workload was stopped"
else
  fail "Prior database state was restored but did not pass health and could not be verified stopped"
fi
