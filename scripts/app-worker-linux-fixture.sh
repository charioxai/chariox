#!/usr/bin/env bash
# Called only by the bounded, disposable hosted systemd fixture service.
set -euo pipefail
[[ $# == 6 && "$(id -u)" == 0 && "$(uname -s)" == Linux ]]
fixture_repo="$(realpath -e -- "$1")"
fixture_scratch="$(realpath -e -- "$2")"
fixture_uid="$3"
fixture_gid="$4"
fixture_node="$5"
fixture_unit="$6"
[[ "$fixture_uid" =~ ^[1-9][0-9]*$ && "$fixture_gid" =~ ^[1-9][0-9]*$ ]]
[[ "$fixture_unit" =~ ^chariox-native-[0-9]+-[0-9]+$ ]]
[[ "$(cat /proc/self/cgroup)" == "0::/system.slice/$fixture_unit.service" ]]
[[ "$(readlink /proc/self/ns/mnt)" != "$(readlink /proc/1/ns/mnt)" ]]
fixture_cgroup="/sys/fs/cgroup/system.slice/$fixture_unit.service"
[[ "$(cat "$fixture_cgroup/memory.max")" == 1073741824 ]]
[[ "$(cat "$fixture_cgroup/memory.swap.max")" == 0 ]]
[[ "$(cat "$fixture_cgroup/pids.max")" == 64 ]]
read -r fixture_cpu_quota fixture_cpu_period < "$fixture_cgroup/cpu.max"
[[ "$fixture_cpu_quota" == "$fixture_cpu_period" ]]
fixture_mounts="$fixture_scratch/mounts"
mkdir -m 755 "$fixture_mounts"
cleanup_mounts() {
  for fixture_path in "$fixture_mounts"/*; do umount "$fixture_path" 2>/dev/null || true; done
}
trap cleanup_mounts EXIT
for fixture_name in package runtime good-data good-tmp weak-data weak-tmp bad-data bad-tmp; do
  mkdir "$fixture_mounts/$fixture_name"
  fixture_flags="rw,nodev,nosuid,noexec,size=8m,mode=0700,uid=$fixture_uid,gid=$fixture_gid"
  if [[ "$fixture_name" == runtime || "$fixture_name" == bad-data ]]; then
    fixture_flags="rw,nodev,nosuid,exec,size=8m,mode=0700,uid=$fixture_uid,gid=$fixture_gid"
  fi
  mount -t tmpfs -o "$fixture_flags" chariox-native-fixture "$fixture_mounts/$fixture_name"
done
printf 'private supervisor fixture\n' > "$fixture_scratch/secret"
chmod 600 "$fixture_scratch/secret"
ln -s "$fixture_scratch/secret" "$fixture_mounts/package/escape"
dd if=/dev/zero of="$fixture_mounts/package/fixture.bin" bs=4096 count=1 status=none
cp "$fixture_scratch/bin/chariox-app-worker" "$fixture_scratch/bin/weakened-test-worker" \
  "$fixture_scratch/bin/libchariox-app-runtime.so" "$fixture_mounts/runtime/"
mount -o remount,ro,nodev,nosuid,noexec "$fixture_mounts/package"
mount -o remount,ro,nodev,nosuid,exec "$fixture_mounts/runtime"
# Root controls provisioning only. The harness drops supplementary groups and
# uses uid/gid in spawn before bundled bwrap executes; no host-root descriptor
# enters the worker. The private fixture sentinel is the only extra test FD.
"$fixture_node" --max-old-space-size=256 "$fixture_repo/apps/app-worker/tests/linux-native-fixture.mjs" \
  "$fixture_repo" "$fixture_scratch" "$fixture_uid" "$fixture_gid" "$fixture_unit"
