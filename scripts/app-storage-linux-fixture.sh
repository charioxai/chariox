#!/usr/bin/env bash
# Dedicated runner only. Uses the production helper, its fixed enrollment, and
# the managed service's real namespace/delegation settings with a sleep main.
set -euo pipefail
[[ $# == 4 && "$(id -u)" == 0 && -d /run/systemd/system ]]
storage_repo="$(realpath -e "$1")" storage_scratch="$(realpath -e "$2")"
storage_tests="$(realpath -e "$3")" storage_helper="$(realpath -e "$4")"
[[ "$storage_scratch" == /home/runner/work/_temp/chariox-storage.* || "$storage_scratch" == /home/runner/work/_temp/*/chariox-storage.* ]]
[[ "$storage_tests" == "$storage_scratch/build/"* && "$storage_helper" == "$storage_scratch/build/"* ]]
for storage_path in /etc/chariox /var/lib/chariox-app-storage /usr/libexec/chariox-app-storage /etc/systemd/system/chariox-managed-bootstrap.service; do
  [[ ! -e "$storage_path" && ! -L "$storage_path" ]]
done
[[ "$(stat -f -c %T /sys/fs/cgroup)" == cgroup2fs ]]
getent passwd chariox >/dev/null && exit 1
getent group chariox >/dev/null || groupadd --system chariox
getent group chariox-slice >/dev/null || groupadd --system chariox-slice
useradd --system --gid chariox --home-dir /var/lib/chariox/home --shell /usr/sbin/nologin chariox
storage_uid="$(id -u chariox)" storage_gid="$(id -g chariox)"
install -d -o root -g root -m 755 /etc/chariox /usr/libexec /etc/systemd/system/chariox-managed-bootstrap.service.d /etc/systemd/system/chariox-app-storage.service.d
install -d -o root -g root -m 711 /var/lib/chariox-app-storage
install -d -o chariox -g chariox -m 700 /var/lib/chariox /var/lib/chariox/home
install -d -o root -g chariox-slice -m 710 /var/lib/chariox-slice-share
install -o root -g root -m 555 "$storage_helper" /usr/libexec/chariox-app-storage
install -o root -g root -m 555 "$storage_tests" /usr/libexec/chariox-app-storage-tests
install -m 644 "$storage_repo/deploy/managed-kernel/chariox-app-storage.service" /etc/systemd/system/chariox-app-storage.service
install -m 644 "$storage_repo/deploy/managed-kernel/chariox-managed-bootstrap.service" /etc/systemd/system/chariox-managed-bootstrap.service
printf '{"schema":"chariox.app-storage-enrollment.v1","owners":[{"uid":%s,"gid":%s,"cgroup_root":"/sys/fs/cgroup/system.slice/chariox-managed-bootstrap.service/apps"}]}\n' "$storage_uid" "$storage_gid" > /etc/chariox/app-storage.json
chmod 644 /etc/chariox/app-storage.json
cat > /etc/systemd/system/chariox-managed-bootstrap.service.d/fixture.conf <<'UNIT'
[Unit]
ConditionPathExists=
[Service]
ExecStart=
ExecStart=/usr/bin/sleep infinity
ExecStartPre=
ExecStartPre=+/usr/libexec/chariox-app-storage --prepare-managed-domain
MemoryMax=1G
MemorySwapMax=0
TasksMax=64
CPUQuota=100%
UNIT
cat > /etc/systemd/system/chariox-app-storage.service.d/fixture.conf <<'UNIT'
[Service]
Restart=no
UNIT
cleanup() {
  set +e
  systemctl stop chariox-storage-actual.service chariox-storage-crash.service chariox-managed-bootstrap.service chariox-app-storage.service
  journalctl --no-pager -u chariox-app-storage.service -u chariox-managed-bootstrap.service -n 120
  # No force/lazy unmount or foreign-loop cleanup. Preserve journal/identity
  # evidence on failure; this runner is discarded after bounded artifact capture.
  find /var/lib/chariox-app-storage -maxdepth 3 -name journal.json -type f -size -17k -exec cp --parents '{}' "$storage_scratch/evidence/" \;
  findmnt --json -R /var/lib/chariox-app-storage > "$storage_scratch/evidence/final-mounts.json" 2>/dev/null
  losetup --json --list --output NAME,BACK-FILE,SIZELIMIT,AUTOCLEAR > "$storage_scratch/evidence/final-loops.json"
  chown -R "$(stat -c %u "$storage_scratch"):$(stat -c %g "$storage_scratch")" "$storage_scratch/evidence"
}
trap cleanup EXIT
systemctl daemon-reload
systemctl start chariox-app-storage.service chariox-managed-bootstrap.service
storage_kernel="$(systemctl show -p MainPID --value chariox-managed-bootstrap.service)"
[[ "$storage_kernel" =~ ^[1-9][0-9]+$ ]]
[[ "$(readlink /proc/$storage_kernel/ns/mnt)" != "$(readlink /proc/1/ns/mnt)" ]]
storage_server="$(systemctl show -p MainPID --value chariox-app-storage.service)"
[[ "$(readlink /proc/$storage_server/ns/mnt)" == "$(readlink /proc/1/ns/mnt)" ]]
run_test() {
  local storage_unit="$1" storage_filter="$2"
  systemd-run --unit="$storage_unit" --wait --collect --pipe \
    --property=MemoryMax=1G --property=MemorySwapMax=0 --property=CPUQuota=100% \
    --property=TasksMax=64 --property=RuntimeMaxSec=180 --property=KillMode=control-group \
    /usr/bin/nsenter --target "$storage_kernel" --mount /usr/bin/setpriv \
    --reuid="$storage_uid" --regid="$storage_gid" --clear-groups /usr/bin/env -i \
    GITHUB_ACTIONS=true RUNNER_ENVIRONMENT=github-hosted GITHUB_REPOSITORY=charioxai/chariox \
    CHARIOX_STORAGE_HOSTED=fixed-production-helper \
    CHARIOX_STORAGE_CRASH_MARKER=/var/lib/chariox/home/storage-crash-ready \
    /usr/libexec/chariox-app-storage-tests "$storage_filter" --ignored --nocapture --test-threads=1
}
run_test chariox-storage-actual hosted_private_capacity_persistence_tmp_reset_and_noexec
run_test chariox-storage-crash hosted_crash_fixture_holds_lease_until_owner_is_killed > "$storage_scratch/evidence/crash-holder.log" 2>&1 &
storage_waiter=$!
for storage_attempt in $(seq 1 300); do
  [[ ! -f /var/lib/chariox/home/storage-crash-ready ]] || break
  kill -0 "$storage_waiter" 2>/dev/null || { wait "$storage_waiter"; exit 1; }
  sleep 0.1
done
[[ -f /var/lib/chariox/home/storage-crash-ready ]]
systemctl kill --kill-whom=all --signal=SIGKILL chariox-app-storage.service
systemctl stop chariox-storage-crash.service
wait "$storage_waiter" || true
systemctl reset-failed chariox-app-storage.service || true
systemctl start chariox-app-storage.service
run_test chariox-storage-actual hosted_recovery_preserves_data_after_abrupt_helper_exit
systemctl stop chariox-managed-bootstrap.service chariox-app-storage.service
if findmnt --noheadings --raw --output TARGET | grep -F '/var/lib/chariox-app-storage/' ; then exit 1; fi
if losetup --list --noheadings --output BACK-FILE | grep -F '/var/lib/chariox-app-storage/' ; then exit 1; fi
printf 'Production helper quota, host-to-kernel mount propagation, noexec, persistence, abrupt exit recovery and exact reclamation passed.\n'
