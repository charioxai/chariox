#!/usr/bin/env bash
# Root provisions only this disposable fixture; the worker runs as the runner UID.
set -euo pipefail
[[ $# == 6 && "$(id -u)" == 0 && "$(uname -sm)" == 'Linux x86_64' ]]
fixture_repo="$(realpath -e -- "$1")"
fixture_scratch="$(realpath -e -- "$2")"
fixture_uid="$3"; fixture_gid="$4"; fixture_node="$5"; fixture_unit="$6"
[[ "$fixture_uid" =~ ^[1-9][0-9]*$ && "$fixture_gid" =~ ^[1-9][0-9]*$ ]]
[[ "$fixture_unit" =~ ^chariox-embedded-[0-9]+-[0-9]+$ ]]
[[ "$(cat /proc/self/cgroup)" == "0::/system.slice/$fixture_unit.service" ]]
[[ "$(readlink /proc/self/ns/mnt)" != "$(readlink /proc/1/ns/mnt)" ]]
fixture_cgroup="/sys/fs/cgroup/system.slice/$fixture_unit.service"
[[ "$(cat "$fixture_cgroup/memory.max")" == 2147483648 && "$(cat "$fixture_cgroup/memory.swap.max")" == 0 ]]
[[ "$(cat "$fixture_cgroup/pids.max")" == 128 ]]
read -r fixture_quota fixture_period < "$fixture_cgroup/cpu.max"
[[ "$fixture_quota" == "$fixture_period" ]]
fixture_mounts="$fixture_scratch/mounts"
mkdir -m 755 "$fixture_mounts"
cleanup_mounts() { for fixture_path in "$fixture_mounts"/*; do umount "$fixture_path" 2>/dev/null || true; done; }
trap cleanup_mounts EXIT
for fixture_name in package run-data run-tmp disconnect-data disconnect-tmp deadline-data deadline-tmp; do
  mkdir "$fixture_mounts/$fixture_name"
  mount -t tmpfs -o "rw,nodev,nosuid,noexec,size=8m,mode=0700,uid=$fixture_uid,gid=$fixture_gid" \
    chariox-embedded-fixture "$fixture_mounts/$fixture_name"
done
mkdir "$fixture_mounts/runtime"
mount --bind "$fixture_scratch/bundle" "$fixture_mounts/runtime"
mount -o remount,bind,ro,nodev,nosuid,exec "$fixture_mounts/runtime"
mkdir "$fixture_mounts/package/runtime"
cp "$fixture_repo/apps/app-worker/tests/embedded-app/"* "$fixture_mounts/package/runtime/"
printf 'export default async () => new Promise(() => {});\n' > "$fixture_mounts/package/runtime/stalled.mjs"
printf 'not a native addon\n' > "$fixture_mounts/package/addon.node"
printf 'private supervisor fixture\n' > "$fixture_scratch/secret"
chmod 600 "$fixture_scratch/secret"
ln -s "$fixture_scratch/secret" "$fixture_mounts/package/escape"
mount -o remount,ro,nodev,nosuid,noexec "$fixture_mounts/package"
# No GitHub token, browser profile or host credential enters bwrap. This root
# coordinator drops supplementary groups before spawning its non-root worker.
"$fixture_node" --max-old-space-size=256 "$fixture_repo/apps/app-worker/tests/linux-embedded-fixture.mjs" \
  "$fixture_repo" "$fixture_scratch" "$fixture_uid" "$fixture_gid" "$fixture_unit"
