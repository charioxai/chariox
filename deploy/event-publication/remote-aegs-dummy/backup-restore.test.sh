#!/usr/bin/env bash
set -euo pipefail

deployment_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
temporary_dir="$(mktemp -d)"
trap 'rm -rf "${temporary_dir}"' EXIT

fail() { printf '%s\n' "$1" >&2; exit 1; }
assert_contains() { grep -Fq -- "$2" "$1" || fail "Expected $1 to contain: $2"; }
assert_not_contains() {
  if grep -Fq -- "$2" "$1"; then fail "Expected $1 not to contain: $2"; fi
}
checksum_create() {
  if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi
}
checksum_check() {
  if command -v sha256sum >/dev/null; then sha256sum --check "$1"; else shasum -a 256 --check "$1"; fi
}
file_mode() { stat -c '%a' "$1" 2>/dev/null || stat -f '%Lp' "$1"; }
file_digest() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi
}
assert_file_contents() {
  [[ "$(<"$1")" == "$2" ]] || fail "Unexpected contents in $1"
}
assert_no_restore_checkpoints() {
  if find "$1" -maxdepth 1 -name '.aegs.restore-*' -print -quit | grep -q .; then
    fail "Restore retained prior-state checkpoints in $1"
  fi
}
wait_for_path() {
  local path="$1" process_id="$2"
  for _wait_attempt in {1..500}; do
    [[ ! -e "${path}" ]] || return 0
    kill -0 "${process_id}" 2>/dev/null \
      || fail "Lock holder exited before reaching its blocked Docker call"
    sleep 0.01
  done
  fail "Timed out waiting for lock holder"
}

fake_bin="${temporary_dir}/bin"
fake_source="${temporary_dir}/source"
fake_target="${temporary_dir}/target"
backup_dir="${temporary_dir}/backups"
role_marker="${temporary_dir}/host-role"
log="${temporary_dir}/docker.log"
runtime_state_file="${temporary_dir}/runtime-state"
stop_count_file="${temporary_dir}/stop-count"
operation_lock_dir="${temporary_dir}/operation.lock"
mkdir -p "${fake_bin}" "${fake_source}" "${fake_target}"
printf 'aegs\n' >"${role_marker}"
printf 'database-before\n' >"${fake_source}/aegs.db"
printf 'wal-before\n' >"${fake_source}/aegs.db-wal"
printf 'running\n' >"${runtime_state_file}"
printf '0\n' >"${stop_count_file}"

cat >"${fake_bin}/docker" <<'FAKE_DOCKER'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"${FAKE_DOCKER_LOG}"
if [[ "${FAKE_BLOCK_DOCKER:-0}" == "1" \
  && ! -e "${FAKE_BLOCK_RELEASE_FILE}" ]]; then
  : >"${FAKE_BLOCK_ENTERED_FILE}"
  for _wait_attempt in {1..500}; do
    [[ ! -e "${FAKE_BLOCK_RELEASE_FILE}" ]] || break
    sleep 0.01
  done
  [[ -e "${FAKE_BLOCK_RELEASE_FILE}" ]] || exit 98
fi
runtime_state() {
  local value=stopped
  [[ ! -s "${FAKE_RUNTIME_STATE_FILE}" ]] \
    || IFS= read -r value <"${FAKE_RUNTIME_STATE_FILE}"
  printf '%s\n' "${value}"
}
set_runtime_state() { printf '%s\n' "$1" >"${FAKE_RUNTIME_STATE_FILE}"; }
runtime_unavailable=false
if [[ -n "${FAKE_RUNTIME_UNAVAILABLE_FILE:-}" \
  && -e "${FAKE_RUNTIME_UNAVAILABLE_FILE}" ]]; then
  runtime_unavailable=true
fi
if [[ "$1" == "ps" ]]; then
  [[ "${runtime_unavailable}" == false ]] || exit 127
  [[ " $* " == *" --filter label=com.docker.compose.project=${EXPECTED_PROJECT} "* ]] \
    || exit 89
  [[ " $* " == *" --filter label=com.docker.compose.service=dummy-aegs "* ]] \
    || exit 90
  [[ "$(runtime_state)" != running ]] || printf 'dummy-container-id\n'
  exit 0
fi
if [[ "$1" == "volume" && "$2" == "ls" ]]; then
  printf '%s\n' "${FAKE_DOCKER_VOLUMES:-}"
  exit 0
fi
if [[ "$1" == "compose" ]]; then
  if [[ " $* " == *" images --quiet dummy-aegs "* ]]; then
    printf 'sha256:fake-image\n'
    exit 0
  fi
  if [[ " $* " == *" stop dummy-aegs "* && "${FAKE_STOP_FAIL:-0}" == "1" ]]; then
    exit 31
  fi
  if [[ " $* " == *" stop dummy-aegs "* ]]; then
    stop_count=0
    [[ ! -s "${FAKE_STOP_COUNT_FILE}" ]] || stop_count="$(<"${FAKE_STOP_COUNT_FILE}")"
    stop_count=$((stop_count + 1))
    printf '%s\n' "${stop_count}" >"${FAKE_STOP_COUNT_FILE}"
    [[ "${runtime_unavailable}" == false ]] || exit 127
    if [[ -n "${FAKE_STOP_FAILURE_AT:-}" \
      && "${stop_count}" -eq "${FAKE_STOP_FAILURE_AT}" ]]; then
      exit 31
    fi
    [[ "${FAKE_STOP_LEAVES_RUNNING:-0}" != "1" ]] || exit 0
    set_runtime_state stopped
    exit 0
  fi
  if [[ " $* " == *" kill dummy-aegs "* ]]; then
    [[ "${runtime_unavailable}" == false ]] || exit 127
    [[ "${FAKE_KILL_LEAVES_RUNNING:-0}" != "1" ]] || exit 0
    set_runtime_state stopped
    exit 0
  fi
  if [[ " $* " == *" start --wait dummy-aegs "* ]]; then
    set_runtime_state running
    if [[ -n "${FAKE_SIGNAL_OUTER_ON_START_FILE:-}" \
      && ! -e "${FAKE_SIGNAL_OUTER_ON_START_FILE}" ]]; then
      : >"${FAKE_SIGNAL_OUTER_ON_START_FILE}"
      kill -TERM "${PPID}"
      sleep 1
      exit 0
    fi
    if [[ -n "${FAKE_START_FAILURES_FILE:-}" && -s "${FAKE_START_FAILURES_FILE}" ]]; then
      failures="$(<"${FAKE_START_FAILURES_FILE}")"
      if [[ "${failures}" -gt 0 ]]; then
        if [[ "${FAKE_EXPECT_CHECKPOINT_ON_START:-0}" == "1" ]]; then
          shopt -s nullglob
          checkpoints=("${FAKE_VOLUME_TARGET}"/.aegs.restore-*/state \
            "${FAKE_VOLUME_TARGET}"/.aegs.restore-*/previous.*)
          [[ "${#checkpoints[@]}" -ge 2 ]] || exit 33
        fi
        printf '%s\n' "$((failures - 1))" >"${FAKE_START_FAILURES_FILE}"
        [[ -z "${FAKE_RUNTIME_UNAVAILABLE_FILE:-}" ]] \
          || : >"${FAKE_RUNTIME_UNAVAILABLE_FILE}"
        exit 32
      fi
    fi
  fi
  exit 0
fi
if [[ "$1" == "run" ]]; then
  if [[ " $* " == *":/source:ro "* ]]; then
    [[ "${FAKE_BACKUP_COPY_FAIL:-0}" != "1" ]] || exit 9
    backup_name="${!#}"
    [[ -f "${FAKE_BACKUP_DIR}/${backup_name}" && ! -s "${FAKE_BACKUP_DIR}/${backup_name}" ]]
    cp "${FAKE_VOLUME_SOURCE}/aegs.db" "${FAKE_BACKUP_DIR}/${backup_name}.partial"
    chmod 600 "${FAKE_BACKUP_DIR}/${backup_name}.partial"
    mv "${FAKE_BACKUP_DIR}/${backup_name}.partial" "${FAKE_BACKUP_DIR}/${backup_name}"
    if [[ -s "${FAKE_VOLUME_SOURCE}/aegs.db-wal" ]]; then
      cp "${FAKE_VOLUME_SOURCE}/aegs.db-wal" "${FAKE_BACKUP_DIR}/${backup_name}-wal.partial"
      chmod 600 "${FAKE_BACKUP_DIR}/${backup_name}-wal.partial"
      mv "${FAKE_BACKUP_DIR}/${backup_name}-wal.partial" "${FAKE_BACKUP_DIR}/${backup_name}-wal"
    fi
    exit 0
  fi
  if [[ " $* " == *":/target "* ]]; then
    argument_count=$#
    backup_index=$((argument_count - 1))
    backup_name="${!backup_index}"
    token="${!argument_count}"
    script=""
    for ((index = 1; index <= argument_count; index += 1)); do
      if [[ "${!index}" == "-ceu" ]]; then
        script_index=$((index + 1))
        script="${!script_index}"
        break
      fi
    done
    [[ -n "${script}" ]]
    script="${script//\/target/${FAKE_VOLUME_TARGET}}"
    script="${script//\/backup/${FAKE_BACKUP_DIR}}"
    sh -ceu "${script}" -- "${backup_name}" "${token}"
    exit $?
  fi
fi
exit 2
FAKE_DOCKER
chmod 0755 "${fake_bin}/docker"

cat >"${fake_bin}/install" <<'FAKE_INSTALL'
#!/usr/bin/env bash
set -euo pipefail
destination="${!#}"
source_index=$(($# - 1))
source="${!source_index}"
if [[ "${FAKE_RESTORE_FAILURE:-none}" == "stage" && "${destination}" == *".restore-"* ]]; then
  exit 41
fi
cp "${source}" "${destination}"
chmod 600 "${destination}"
FAKE_INSTALL
chmod 0755 "${fake_bin}/install"

cat >"${fake_bin}/mv" <<'FAKE_MV'
#!/usr/bin/env bash
set -euo pipefail
source="$1"
destination="$2"
case "${FAKE_RESTORE_FAILURE:-none}" in
  mid-swap)
    [[ "${source}" != */stage.db ]] || exit 42
    ;;
  rollback)
    if [[ "${source}" == */stage.db || "${source}" == */previous.* ]]; then
      exit 43
    fi
    ;;
  term-swap)
    if [[ "${source}" == */stage.db ]]; then kill -TERM "${PPID}"; sleep 1; exit 44; fi
    ;;
  term-rollback)
    if [[ "${source}" == */previous.* ]]; then kill -TERM "${PPID}"; sleep 1; exit 45; fi
    ;;
esac
exec /bin/mv "$@"
FAKE_MV
chmod 0755 "${fake_bin}/mv"

cat >"${fake_bin}/date" <<'FAKE_DATE'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${FAKE_RESTORE_NAMESPACE_COLLISION:-0}" == "1" ]]; then
  timestamp=20000101T000000Z
  namespace="${FAKE_VOLUME_TARGET}/.aegs.restore-${timestamp}-${PPID}-00000"
  if [[ ! -e "${namespace}" ]]; then
    mkdir "${namespace}"
    printf 'preexisting-namespace\n' >"${namespace}/sentinel"
  fi
  printf '%s\n' "${timestamp}"
  exit 0
fi
if [[ "${FAKE_STEM_COLLISIONS:-0}" != "1" ]]; then exec /bin/date "$@"; fi
timestamp=20000101T000000Z
prefix="${FAKE_BACKUP_DIR}/dummy-${timestamp}-${PPID}"
if [[ -d "${FAKE_BACKUP_DIR}" ]]; then
  printf 'occupied-db\n' >"${prefix}-00000.db"
  printf 'occupied-db-partial\n' >"${prefix}-00001.db.partial"
  printf 'occupied-wal\n' >"${prefix}-00002.db-wal"
  printf 'occupied-wal-partial\n' >"${prefix}-00003.db-wal.partial"
  printf 'occupied-manifest\n' >"${prefix}-00004.db.sha256"
  printf 'occupied-manifest-partial\n' >"${prefix}-00005.db.sha256.partial"
  mkdir "${prefix}-00006.db.reserve"
fi
printf '%s\n' "${timestamp}"
FAKE_DATE
chmod 0755 "${fake_bin}/date"

export PATH="${fake_bin}:${PATH}"
export CHARIOX_AEGS_ENV_FILE="${temporary_dir}/aegs-dummy.env"
export CHARIOX_AEGS_BACKUP_DIR="${backup_dir}"
export CHARIOX_AEGS_HOST_ROLE_MARKER="${role_marker}"
export CHARIOX_AEGS_OPERATION_LOCK_DIR="${operation_lock_dir}"
export FAKE_DOCKER_LOG="${log}"
export EXPECTED_PROJECT=chariox-aegs-dummy-staging
export FAKE_RUNTIME_STATE_FILE="${runtime_state_file}"
export FAKE_STOP_COUNT_FILE="${stop_count_file}"
export FAKE_DOCKER_VOLUMES="chariox-aegs-dummy-staging_aegs-data"
export FAKE_VOLUME_SOURCE="${fake_source}"
export FAKE_VOLUME_TARGET="${fake_target}"
export FAKE_BACKUP_DIR="${backup_dir}"

backup_path="$("${deployment_dir}/backup.sh")"
backup_name="$(basename "${backup_path}")"
[[ "${backup_name}" =~ ^dummy-[0-9]{8}T[0-9]{6}Z-[0-9]+-[0-9]{5}\.db$ ]] \
  || fail "Backup filename is not generated safely: ${backup_name}"
[[ -s "${backup_path}" && -s "${backup_path}-wal" && -s "${backup_path}.sha256" ]] \
  || fail "Backup artifacts are incomplete"
[[ "$(file_mode "${backup_dir}")" == "700" ]] || fail "Backup directory is not mode 0700"
[[ "$(file_mode "${backup_path}")" == "600" ]] || fail "Backup database is not mode 0600"
[[ "$(file_mode "${backup_path}-wal")" == "600" ]] || fail "Backup WAL is not mode 0600"
[[ "$(file_mode "${backup_path}.sha256")" == "600" ]] || fail "Backup manifest is not mode 0600"
(cd "${backup_dir}" && checksum_check "${backup_name}.sha256")
manifest_names=()
while read -r _digest filename; do
  manifest_names+=("${filename}")
done <"${backup_path}.sha256"
[[ "${manifest_names[*]}" == "${backup_name} ${backup_name}-wal" ]] \
  || fail "Backup manifest does not exactly name DB and WAL"
assert_contains "${log}" "--project-name chariox-aegs-dummy-staging"
assert_contains "${log}" "label=com.docker.compose.project=chariox-aegs-dummy-staging"
assert_contains "${log}" "label=com.docker.compose.volume=aegs-data"
assert_contains "${log}" "stop dummy-aegs"
assert_contains "${log}" "start --wait dummy-aegs"

first_digest="$(file_digest "${backup_path}")"
second_backup_path="$("${deployment_dir}/backup.sh")"
[[ "${backup_path}" != "${second_backup_path}" ]] || fail "Two backups reused the same filename"
[[ "$(file_digest "${backup_path}")" == "${first_digest}" ]] || fail "Second backup overwrote the first"

export CHARIOX_AEGS_BACKUP_DIR="${temporary_dir}/verified-stop-backups"
export FAKE_BACKUP_DIR="${CHARIOX_AEGS_BACKUP_DIR}"
export FAKE_STOP_LEAVES_RUNNING=1
printf 'running\n' >"${runtime_state_file}"
: >"${log}"
"${deployment_dir}/backup.sh" >/dev/null
assert_contains "${log}" "kill dummy-aegs"
assert_contains "${log}" "label=com.docker.compose.project=chariox-aegs-dummy-staging"
assert_contains "${log}" "label=com.docker.compose.service=dummy-aegs"
unset FAKE_STOP_LEAVES_RUNNING

export CHARIOX_AEGS_BACKUP_DIR="${backup_dir}"
export FAKE_BACKUP_DIR="${backup_dir}"
printf 'still-running-db\n' >"${fake_target}/aegs.db"
printf 'still-running-wal\n' >"${fake_target}/aegs.db-wal"
printf 'still-running-shm\n' >"${fake_target}/aegs.db-shm"
export FAKE_STOP_LEAVES_RUNNING=1
export FAKE_KILL_LEAVES_RUNNING=1
printf 'running\n' >"${runtime_state_file}"
: >"${log}"
if "${deployment_dir}/restore.sh" --yes "${backup_name}" >/dev/null 2>&1; then
  fail "Restore mutated SQLite while the exact service was still running"
fi
assert_file_contents "${fake_target}/aegs.db" "still-running-db"
assert_file_contents "${fake_target}/aegs.db-wal" "still-running-wal"
assert_file_contents "${fake_target}/aegs.db-shm" "still-running-shm"
assert_contains "${log}" "kill dummy-aegs"
assert_not_contains "${log}" ":/target"
unset FAKE_STOP_LEAVES_RUNNING
unset FAKE_KILL_LEAVES_RUNNING

printf 'database-corrupt\n' >"${fake_target}/aegs.db"
printf 'wal-corrupt\n' >"${fake_target}/aegs.db-wal"
printf 'shm-corrupt\n' >"${fake_target}/aegs.db-shm"
"${deployment_dir}/restore.sh" --yes "${backup_name}" >/dev/null
cmp "${fake_source}/aegs.db" "${fake_target}/aegs.db"
cmp "${fake_source}/aegs.db-wal" "${fake_target}/aegs.db-wal"
[[ ! -e "${fake_target}/aegs.db-shm" ]] || fail "Restore retained the stale SHM"
assert_no_restore_checkpoints "${fake_target}"

block_entered="${temporary_dir}/block-entered"
block_release="${temporary_dir}/block-release"
holder_output="${temporary_dir}/holder-output"
contender_output="${temporary_dir}/contender-output"
export FAKE_BLOCK_DOCKER=1
export FAKE_BLOCK_ENTERED_FILE="${block_entered}"
export FAKE_BLOCK_RELEASE_FILE="${block_release}"
"${deployment_dir}/backup.sh" >"${holder_output}" 2>&1 &
holder_pid=$!
wait_for_path "${block_entered}" "${holder_pid}"
assert_contains "${operation_lock_dir}/owner" "pid=${holder_pid}"
docker_lines_before="$(wc -l <"${log}")"
if "${deployment_dir}/restore.sh" --yes "${backup_name}" \
    >"${contender_output}" 2>&1; then
  fail "Restore acquired the operation lock while backup held it"
fi
assert_contains "${contender_output}" "Another AEGS backup or restore operation holds"
[[ "$(wc -l <"${log}")" -eq "${docker_lines_before}" ]] \
  || fail "Contending restore contacted Docker while backup held the lock"
: >"${block_release}"
wait "${holder_pid}"
[[ ! -e "${operation_lock_dir}" ]] || fail "Backup left the operation lock behind"
rm -f "${block_entered}" "${block_release}"

"${deployment_dir}/restore.sh" --yes "${backup_name}" >"${holder_output}" 2>&1 &
holder_pid=$!
wait_for_path "${block_entered}" "${holder_pid}"
assert_contains "${operation_lock_dir}/owner" "pid=${holder_pid}"
docker_lines_before="$(wc -l <"${log}")"
if "${deployment_dir}/restore.sh" --yes "${backup_name}" \
    >"${contender_output}" 2>&1; then
  fail "Second restore acquired the operation lock while restore held it"
fi
assert_contains "${contender_output}" "Another AEGS backup or restore operation holds"
[[ "$(wc -l <"${log}")" -eq "${docker_lines_before}" ]] \
  || fail "Contending restore contacted Docker while restore held the lock"
: >"${block_release}"
wait "${holder_pid}"
[[ ! -e "${operation_lock_dir}" ]] || fail "Restore left the operation lock behind"
unset FAKE_BLOCK_DOCKER
unset FAKE_BLOCK_ENTERED_FILE
unset FAKE_BLOCK_RELEASE_FILE

printf 'live-health-db\n' >"${fake_target}/aegs.db"
printf 'live-health-wal\n' >"${fake_target}/aegs.db-wal"
printf 'live-health-shm\n' >"${fake_target}/aegs.db-shm"
restore_start_failures="${temporary_dir}/restore-start-failures"
printf '1\n' >"${restore_start_failures}"
printf '0\n' >"${stop_count_file}"
export FAKE_START_FAILURES_FILE="${restore_start_failures}"
export FAKE_EXPECT_CHECKPOINT_ON_START=1
export FAKE_STOP_FAILURE_AT=2
: >"${log}"
if "${deployment_dir}/restore.sh" --yes "${backup_name}" >/dev/null 2>&1; then
  fail "Restore accepted a restored workload that failed health"
fi
assert_file_contents "${fake_target}/aegs.db" "live-health-db"
assert_file_contents "${fake_target}/aegs.db-wal" "live-health-wal"
assert_file_contents "${fake_target}/aegs.db-shm" "live-health-shm"
[[ "$(grep -Fc "start --wait dummy-aegs" "${log}")" -eq 2 ]] \
  || fail "Restored-health failure did not health-gate both new and prior state"
[[ "$(grep -Fc "stop dummy-aegs" "${log}")" -eq 2 ]] \
  || fail "Restored-health failure did not stop before rollback"
assert_contains "${log}" "kill dummy-aegs"
assert_no_restore_checkpoints "${fake_target}"
unset FAKE_START_FAILURES_FILE
unset FAKE_EXPECT_CHECKPOINT_ON_START
unset FAKE_STOP_FAILURE_AT

printf 'two-start-db\n' >"${fake_target}/aegs.db"
printf 'two-start-wal\n' >"${fake_target}/aegs.db-wal"
printf 'two-start-shm\n' >"${fake_target}/aegs.db-shm"
printf '2\n' >"${restore_start_failures}"
printf '0\n' >"${stop_count_file}"
export FAKE_START_FAILURES_FILE="${restore_start_failures}"
: >"${log}"
if "${deployment_dir}/restore.sh" --yes "${backup_name}" >/dev/null 2>&1; then
  fail "Restore ignored two consecutive start-health failures"
fi
assert_file_contents "${fake_target}/aegs.db" "two-start-db"
assert_file_contents "${fake_target}/aegs.db-wal" "two-start-wal"
assert_file_contents "${fake_target}/aegs.db-shm" "two-start-shm"
[[ "$(<"${runtime_state_file}")" == stopped ]] \
  || fail "Two start-health failures did not leave the workload stopped"
[[ "$(grep -Fc "start --wait dummy-aegs" "${log}")" -eq 2 ]] \
  || fail "Two start-health failures did not test both restored and prior state"
assert_no_restore_checkpoints "${fake_target}"
unset FAKE_START_FAILURES_FILE

printf 'term-swap-db\n' >"${fake_target}/aegs.db"
printf 'term-swap-wal\n' >"${fake_target}/aegs.db-wal"
printf 'term-swap-shm\n' >"${fake_target}/aegs.db-shm"
printf 'running\n' >"${runtime_state_file}"
export FAKE_RESTORE_FAILURE=term-swap
: >"${log}"
if "${deployment_dir}/restore.sh" --yes "${backup_name}" >/dev/null 2>&1; then
  fail "Restore ignored TERM during the initial swap"
fi
assert_file_contents "${fake_target}/aegs.db" "term-swap-db"
assert_file_contents "${fake_target}/aegs.db-wal" "term-swap-wal"
assert_file_contents "${fake_target}/aegs.db-shm" "term-swap-shm"
[[ "$(<"${runtime_state_file}")" == running ]] \
  || fail "TERM rollback did not health-gate the prior state"
assert_no_restore_checkpoints "${fake_target}"
unset FAKE_RESTORE_FAILURE

printf 'term-rollback-db\n' >"${fake_target}/aegs.db"
printf 'term-rollback-wal\n' >"${fake_target}/aegs.db-wal"
printf 'term-rollback-shm\n' >"${fake_target}/aegs.db-shm"
printf '1\n' >"${restore_start_failures}"
export FAKE_START_FAILURES_FILE="${restore_start_failures}"
export FAKE_RESTORE_FAILURE=term-rollback
: >"${log}"
if "${deployment_dir}/restore.sh" --yes "${backup_name}" >/dev/null 2>&1; then
  fail "Restore ignored TERM during checkpoint rollback"
fi
retained_namespaces=("${fake_target}"/.aegs.restore-*)
[[ "${#retained_namespaces[@]}" -eq 1 \
  && -s "${retained_namespaces[0]}/state" \
  && -s "${retained_namespaces[0]}/previous.db" ]] \
  || fail "TERM rollback did not retain its owned checkpoint namespace"
assert_file_contents "${retained_namespaces[0]}/previous.db" "term-rollback-db"
[[ "$(<"${runtime_state_file}")" == stopped ]] \
  || fail "TERM rollback failure did not leave the workload stopped"
[[ "$(grep -Fc "start --wait dummy-aegs" "${log}")" -eq 1 ]] \
  || fail "TERM rollback failure tried to start ambiguous prior state"
cp "${retained_namespaces[0]}/previous.db" "${fake_target}/aegs.db"
cp "${retained_namespaces[0]}/previous.db-wal" "${fake_target}/aegs.db-wal"
cp "${retained_namespaces[0]}/previous.db-shm" "${fake_target}/aegs.db-shm"
find "${retained_namespaces[0]}" -depth -delete
unset FAKE_START_FAILURES_FILE
unset FAKE_RESTORE_FAILURE

printf 'unknown-runtime-db\n' >"${fake_target}/aegs.db"
printf 'unknown-runtime-wal\n' >"${fake_target}/aegs.db-wal"
printf 'unknown-runtime-shm\n' >"${fake_target}/aegs.db-shm"
printf 'running\n' >"${runtime_state_file}"
printf '1\n' >"${restore_start_failures}"
runtime_unavailable_file="${temporary_dir}/runtime-unavailable"
unknown_runtime_output="${temporary_dir}/unknown-runtime-output"
export FAKE_START_FAILURES_FILE="${restore_start_failures}"
export FAKE_RUNTIME_UNAVAILABLE_FILE="${runtime_unavailable_file}"
: >"${log}"
if "${deployment_dir}/restore.sh" --yes "${backup_name}" \
    >"${unknown_runtime_output}" 2>&1; then
  fail "Restore ignored unknown Docker state after restored-health failure"
fi
assert_contains "${unknown_runtime_output}" \
  "Docker is unavailable, runtime state is unknown, and checkpoints were retained"
unknown_namespaces=("${fake_target}"/.aegs.restore-*)
[[ "${#unknown_namespaces[@]}" -eq 1 \
  && -s "${unknown_namespaces[0]}/state" \
  && -s "${unknown_namespaces[0]}/previous.db" ]] \
  || fail "Unknown runtime state discarded prior-state checkpoints"
unset FAKE_START_FAILURES_FILE
unset FAKE_RUNTIME_UNAVAILABLE_FILE
rm -f "${runtime_unavailable_file}"
printf 'stopped\n' >"${runtime_state_file}"
cp "${unknown_namespaces[0]}/previous.db" "${fake_target}/aegs.db"
cp "${unknown_namespaces[0]}/previous.db-wal" "${fake_target}/aegs.db-wal"
cp "${unknown_namespaces[0]}/previous.db-shm" "${fake_target}/aegs.db-shm"
find "${unknown_namespaces[0]}" -depth -delete

printf 'namespace-db\n' >"${fake_target}/aegs.db"
printf 'namespace-wal\n' >"${fake_target}/aegs.db-wal"
printf 'namespace-shm\n' >"${fake_target}/aegs.db-shm"
printf 'running\n' >"${runtime_state_file}"
export FAKE_RESTORE_NAMESPACE_COLLISION=1
: >"${log}"
"${deployment_dir}/restore.sh" --yes "${backup_name}" >/dev/null
collision_namespaces=("${fake_target}"/.aegs.restore-*)
[[ "${#collision_namespaces[@]}" -eq 1 \
  && "$(<"${collision_namespaces[0]}/sentinel")" == preexisting-namespace ]] \
  || fail "Restore modified a pre-existing token namespace"
assert_contains "${log}" "-00000"
assert_contains "${log}" "-00001"
find "${collision_namespaces[0]}" -depth -delete
unset FAKE_RESTORE_NAMESPACE_COLLISION

printf 'outer-term-db\n' >"${fake_target}/aegs.db"
printf 'outer-term-wal\n' >"${fake_target}/aegs.db-wal"
printf 'outer-term-shm\n' >"${fake_target}/aegs.db-shm"
printf 'running\n' >"${runtime_state_file}"
outer_signal_file="${temporary_dir}/outer-signal-fired"
outer_signal_output="${temporary_dir}/outer-signal-output"
export FAKE_SIGNAL_OUTER_ON_START_FILE="${outer_signal_file}"
: >"${log}"
if "${deployment_dir}/restore.sh" --yes "${backup_name}" \
    >"${outer_signal_output}" 2>&1; then
  fail "Restore ignored TERM during its outer restart"
fi
assert_contains "${outer_signal_output}" \
  "TERM interrupted restore; workload was verified stopped and checkpoints were retained"
[[ "$(<"${runtime_state_file}")" == stopped ]] \
  || fail "Outer TERM did not leave the workload stopped"
outer_signal_namespaces=("${fake_target}"/.aegs.restore-*)
[[ "${#outer_signal_namespaces[@]}" -eq 1 \
  && -s "${outer_signal_namespaces[0]}/state" \
  && -s "${outer_signal_namespaces[0]}/previous.db" ]] \
  || fail "Outer TERM discarded restore checkpoints"
assert_file_contents "${outer_signal_namespaces[0]}/previous.db" "outer-term-db"
assert_contains "${log}" "label=com.docker.compose.project=chariox-aegs-dummy-staging"
assert_contains "${log}" "label=com.docker.compose.service=dummy-aegs"
cp "${outer_signal_namespaces[0]}/previous.db" "${fake_target}/aegs.db"
cp "${outer_signal_namespaces[0]}/previous.db-wal" "${fake_target}/aegs.db-wal"
cp "${outer_signal_namespaces[0]}/previous.db-shm" "${fake_target}/aegs.db-shm"
find "${outer_signal_namespaces[0]}" -depth -delete
unset FAKE_SIGNAL_OUTER_ON_START_FILE

printf 'live-stage-db\n' >"${fake_target}/aegs.db"
printf 'live-stage-wal\n' >"${fake_target}/aegs.db-wal"
printf 'live-stage-shm\n' >"${fake_target}/aegs.db-shm"
export FAKE_RESTORE_FAILURE=stage
: >"${log}"
if "${deployment_dir}/restore.sh" --yes "${backup_name}" >/dev/null 2>&1; then
  fail "Restore ignored a staging failure"
fi
assert_file_contents "${fake_target}/aegs.db" "live-stage-db"
assert_file_contents "${fake_target}/aegs.db-wal" "live-stage-wal"
assert_file_contents "${fake_target}/aegs.db-shm" "live-stage-shm"
assert_contains "${log}" "start --wait dummy-aegs"

printf 'live-swap-db\n' >"${fake_target}/aegs.db"
printf 'live-swap-wal\n' >"${fake_target}/aegs.db-wal"
printf 'live-swap-shm\n' >"${fake_target}/aegs.db-shm"
export FAKE_RESTORE_FAILURE=mid-swap
: >"${log}"
if "${deployment_dir}/restore.sh" --yes "${backup_name}" >/dev/null 2>&1; then
  fail "Restore ignored a mid-swap failure"
fi
assert_file_contents "${fake_target}/aegs.db" "live-swap-db"
assert_file_contents "${fake_target}/aegs.db-wal" "live-swap-wal"
assert_file_contents "${fake_target}/aegs.db-shm" "live-swap-shm"
assert_contains "${log}" "start --wait dummy-aegs"

export FAKE_RESTORE_FAILURE=rollback
: >"${log}"
if "${deployment_dir}/restore.sh" --yes "${backup_name}" >/dev/null 2>&1; then
  fail "Restore ignored a rollback failure"
fi
assert_contains "${log}" "stop dummy-aegs"
assert_not_contains "${log}" "start --wait dummy-aegs"
unset FAKE_RESTORE_FAILURE
printf 'database-recovered\n' >"${fake_target}/aegs.db"
printf 'wal-recovered\n' >"${fake_target}/aegs.db-wal"
printf 'shm-recovered\n' >"${fake_target}/aegs.db-shm"

start_failures="${temporary_dir}/start-failures"
printf '1\n' >"${start_failures}"
export FAKE_START_FAILURES_FILE="${start_failures}"
export CHARIOX_AEGS_BACKUP_DIR="${temporary_dir}/restart-failure-backups"
export FAKE_BACKUP_DIR="${CHARIOX_AEGS_BACKUP_DIR}"
: >"${log}"
if "${deployment_dir}/backup.sh" >/dev/null 2>&1; then
  fail "Backup ignored a restart failure"
fi
[[ "$(grep -Fc "start --wait dummy-aegs" "${log}")" -eq 2 ]] \
  || fail "Restart failure did not receive one cleanup retry"
restart_artifacts=("${CHARIOX_AEGS_BACKUP_DIR}"/*)
[[ "${#restart_artifacts[@]}" -eq 3 ]] \
  || fail "Restart failure discarded or incompletely retained the finished backup"
unset FAKE_START_FAILURES_FILE

export CHARIOX_AEGS_BACKUP_DIR="${temporary_dir}/copy-failure-backups"
export FAKE_BACKUP_DIR="${CHARIOX_AEGS_BACKUP_DIR}"
export FAKE_BACKUP_COPY_FAIL=1
: >"${log}"
if "${deployment_dir}/backup.sh" >/dev/null 2>&1; then
  fail "Backup ignored a copy failure"
fi
assert_contains "${log}" "stop dummy-aegs"
assert_contains "${log}" "start --wait dummy-aegs"
copy_failure_artifacts=("${CHARIOX_AEGS_BACKUP_DIR}"/*)
[[ "${#copy_failure_artifacts[@]}" -eq 1 \
  && -d "${copy_failure_artifacts[0]}" \
  && "${copy_failure_artifacts[0]}" == *.reserve ]] \
  || fail "Failed backup did not retain only its collision tombstone"
unset FAKE_BACKUP_COPY_FAIL

export CHARIOX_AEGS_BACKUP_DIR="${backup_dir}"
export FAKE_BACKUP_DIR="${backup_dir}"
: >"${log}"
if "${deployment_dir}/restore.sh" "${backup_name}" >/dev/null 2>&1; then
  fail "Restore accepted a request without --yes"
fi
[[ ! -s "${log}" ]] || fail "Rejected restore contacted Docker"

empty_db_name="dummy-20000101T000000Z-123-40000.db"
: >"${backup_dir}/${empty_db_name}"
chmod 0600 "${backup_dir}/${empty_db_name}"
(cd "${backup_dir}" && checksum_create "${empty_db_name}" >"${empty_db_name}.sha256")
chmod 0600 "${backup_dir}/${empty_db_name}.sha256"
: >"${log}"
if "${deployment_dir}/restore.sh" --yes "${empty_db_name}" >/dev/null 2>&1; then
  fail "Restore accepted an empty database artifact"
fi
[[ ! -s "${log}" ]] || fail "Empty database rejection contacted Docker"

empty_wal_name="dummy-20000101T000000Z-123-40001.db"
cp "${backup_path}" "${backup_dir}/${empty_wal_name}"
: >"${backup_dir}/${empty_wal_name}-wal"
chmod 0600 "${backup_dir}/${empty_wal_name}" "${backup_dir}/${empty_wal_name}-wal"
(
  cd "${backup_dir}"
  checksum_create "${empty_wal_name}" "${empty_wal_name}-wal" \
    >"${empty_wal_name}.sha256"
)
chmod 0600 "${backup_dir}/${empty_wal_name}.sha256"
: >"${log}"
if "${deployment_dir}/restore.sh" --yes "${empty_wal_name}" >/dev/null 2>&1; then
  fail "Restore accepted an empty WAL artifact"
fi
[[ ! -s "${log}" ]] || fail "Empty WAL rejection contacted Docker"

printf 'corrupt\n' >>"${backup_path}"
: >"${log}"
if "${deployment_dir}/restore.sh" --yes "${backup_name}" >/dev/null 2>&1; then
  fail "Restore accepted a corrupt backup"
fi
[[ ! -s "${log}" ]] || fail "Checksum failure contacted Docker"
printf 'database-before\n' >"${backup_path}"

manifest_real="${temporary_dir}/manifest-real"
mv "${backup_path}.sha256" "${manifest_real}"
ln -s "${manifest_real}" "${backup_path}.sha256"
: >"${log}"
if "${deployment_dir}/restore.sh" --yes "${backup_name}" >/dev/null 2>&1; then
  fail "Restore accepted a symlink checksum manifest"
fi
[[ ! -s "${log}" ]] || fail "Symlink rejection contacted Docker"
rm "${backup_path}.sha256"
mv "${manifest_real}" "${backup_path}.sha256"

uncovered_name="dummy-20000101T000000Z-123-45678.db"
cp "${backup_path}" "${backup_dir}/${uncovered_name}"
cp "${backup_path}-wal" "${backup_dir}/${uncovered_name}-wal"
chmod 0600 "${backup_dir}/${uncovered_name}" "${backup_dir}/${uncovered_name}-wal"
(cd "${backup_dir}" && checksum_create "${uncovered_name}" >"${uncovered_name}.sha256")
chmod 0600 "${backup_dir}/${uncovered_name}.sha256"
: >"${log}"
if "${deployment_dir}/restore.sh" --yes "${uncovered_name}" >/dev/null 2>&1; then
  fail "Restore accepted a WAL omitted from the checksum manifest"
fi
[[ ! -s "${log}" ]] || fail "Manifest coverage rejection contacted Docker"

export FAKE_DOCKER_VOLUMES=$'one\ntwo'
: >"${log}"
if "${deployment_dir}/restore.sh" --yes "${backup_name}" >/dev/null 2>&1; then
  fail "Restore accepted more than one owned volume"
fi
assert_contains "${log}" "volume ls"
assert_not_contains "${log}" "stop dummy-aegs"
export FAKE_DOCKER_VOLUMES="chariox-aegs-dummy-staging_aegs-data"

root_mode_before="$(file_mode /)"
export CHARIOX_AEGS_BACKUP_DIR="/"
: >"${log}"
if "${deployment_dir}/backup.sh" >/dev/null 2>&1; then fail "Backup accepted / as its backup directory"; fi
[[ "$(file_mode /)" == "${root_mode_before}" ]] || fail "Backup changed root directory permissions"
[[ ! -s "${log}" ]] || fail "Unsafe backup directory contacted Docker"

export CHARIOX_AEGS_BACKUP_DIR="/tmp/../root"
: >"${log}"
if "${deployment_dir}/backup.sh" >/dev/null 2>&1; then fail "Backup accepted a root-like alias"; fi
[[ ! -s "${log}" ]] || fail "Root-like backup directory contacted Docker"

safe_directory="${temporary_dir}/safe-backups"
mkdir "${safe_directory}"
chmod 0700 "${safe_directory}"
symlink_directory="${temporary_dir}/symlink-backups"
ln -s "${safe_directory}" "${symlink_directory}"
export CHARIOX_AEGS_BACKUP_DIR="${symlink_directory}"
: >"${log}"
if "${deployment_dir}/backup.sh" >/dev/null 2>&1; then fail "Backup accepted a symlink directory"; fi
[[ ! -s "${log}" ]] || fail "Symlink backup directory contacted Docker"

wrong_mode_directory="${temporary_dir}/wrong-mode-backups"
mkdir "${wrong_mode_directory}"
chmod 0755 "${wrong_mode_directory}"
export CHARIOX_AEGS_BACKUP_DIR="${wrong_mode_directory}"
: >"${log}"
if "${deployment_dir}/backup.sh" >/dev/null 2>&1; then fail "Backup silently repaired an unsafe existing directory"; fi
[[ "$(file_mode "${wrong_mode_directory}")" == "755" ]] || fail "Backup chmodded an existing directory"
[[ ! -s "${log}" ]] || fail "Wrong-mode backup directory contacted Docker"

export CHARIOX_AEGS_BACKUP_DIR="${temporary_dir}/stem-collision-backups"
export FAKE_BACKUP_DIR="${CHARIOX_AEGS_BACKUP_DIR}"
export FAKE_STEM_COLLISIONS=1
: >"${log}"
collision_backup_path="$("${deployment_dir}/backup.sh")"
[[ "$(basename "${collision_backup_path}")" \
  =~ ^dummy-20000101T000000Z-[0-9]+-00007\.db$ ]] \
  || fail "Backup reused a stem with a final, partial, manifest, or reservation leftover"
collision_leftovers=("${CHARIOX_AEGS_BACKUP_DIR}"/dummy-20000101T000000Z-*-0000*)
[[ "${#collision_leftovers[@]}" -eq 10 ]] \
  || fail "Stem collision test did not retain all seven leftovers and three new artifacts"
unset FAKE_STEM_COLLISIONS

export CHARIOX_AEGS_COMPOSE_PROJECT_NAME="arroba-aegs-dummy-staging"
export FAKE_DOCKER_VOLUMES="arroba-aegs-dummy-staging_aegs-data"
export EXPECTED_PROJECT="arroba-aegs-dummy-staging"
export CHARIOX_AEGS_BACKUP_DIR="${temporary_dir}/old-project-backup"
export FAKE_BACKUP_DIR="${CHARIOX_AEGS_BACKUP_DIR}"
: >"${log}"
"${deployment_dir}/backup.sh" >/dev/null
assert_contains "${log}" "--project-name arroba-aegs-dummy-staging"
assert_contains "${log}" "label=com.docker.compose.project=arroba-aegs-dummy-staging"
unset CHARIOX_AEGS_COMPOSE_PROJECT_NAME
export EXPECTED_PROJECT=chariox-aegs-dummy-staging

if grep -Fq -- '--volumes' "${deployment_dir}/backup.sh" "${deployment_dir}/restore.sh"; then
  fail "Data scripts contain a volume-deletion option"
fi

printf 'dummy AEGS backup/restore tests passed\n'
