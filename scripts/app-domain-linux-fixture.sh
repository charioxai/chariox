#!/usr/bin/env bash
# Fixed disposable root setup. Rust production modules own child cgroup policy,
# pre-exec entry, namespace inspection, lifecycle and reap; no production API is
# granted by the fixture environment variables below.
set -euo pipefail
[[ $# == 6 && "$(id -u)" == 0 ]]
domain_scratch="$(realpath -e -- "$1")"
domain_uid="$2" domain_gid="$3" domain_binary="$(realpath -e -- "$4")" domain_unit="$5"
domain_temp="$(realpath -e -- "$6")"
[[ "$domain_uid" =~ ^[1-9][0-9]*$ && "$domain_gid" =~ ^[1-9][0-9]*$ && "$domain_unit" =~ ^chariox-domain-[0-9]+-[0-9]+$ ]]
[[ "$(dirname -- "$domain_scratch")" == "$domain_temp" && "$(basename -- "$domain_scratch")" == chariox-sandbox.* ]]
[[ "$domain_binary" == "$domain_scratch/rust-build/"* && "$(cat /proc/self/cgroup)" == "0::/system.slice/$domain_unit.service" ]]
[[ "$(readlink /proc/self/ns/mnt)" != "$(readlink /proc/1/ns/mnt)" ]]
domain_cgroup="/sys/fs/cgroup/system.slice/$domain_unit.service"
[[ "$(cat "$domain_cgroup/memory.max")" == 1073741824 && "$(cat "$domain_cgroup/memory.swap.max")" == 0 && "$(cat "$domain_cgroup/pids.max")" == 128 ]]
read -r domain_cpu_quota domain_cpu_period < "$domain_cgroup/cpu.max"
[[ "$domain_cpu_quota" == "$domain_cpu_period" ]]
mkdir "$domain_cgroup/supervisor"
printf '%s\n' "$$" > "$domain_cgroup/supervisor/cgroup.procs"
printf '+cpu +memory +pids\n' > "$domain_cgroup/cgroup.subtree_control"
mkdir "$domain_cgroup/apps"
printf '+cpu +memory +pids\n' > "$domain_cgroup/apps/cgroup.subtree_control"
chown "$domain_uid:$domain_gid" "$domain_cgroup/apps" "$domain_cgroup/apps/cgroup.procs" "$domain_cgroup/apps/cgroup.subtree_control"
# Moving from supervisor to apps also requires write access at their common
# delegated ancestor; resource limits at this service boundary stay root-owned.
chown "$domain_uid:$domain_gid" "$domain_cgroup/cgroup.procs"
mkdir -m 755 "$domain_scratch/domain-mounts"
cleanup() {
  for domain_name in tmp data runtime package; do umount "$domain_scratch/domain-mounts/$domain_name" 2>/dev/null || true; done
}
trap cleanup EXIT
for domain_name in package data tmp runtime; do
  mkdir "$domain_scratch/domain-mounts/$domain_name"
  domain_exec=noexec
  [[ "$domain_name" != runtime ]] || domain_exec=exec
  mount -t tmpfs -o "rw,nodev,nosuid,$domain_exec,size=8m,mode=0700,uid=$domain_uid,gid=$domain_gid" \
    chariox-domain-fixture "$domain_scratch/domain-mounts/$domain_name"
done
ln -s "$domain_scratch/secret" "$domain_scratch/domain-mounts/package/escape"
dd if=/dev/zero of="$domain_scratch/domain-mounts/package/fixture.bin" bs=4096 count=1 status=none
cp "$domain_scratch/bin/chariox-app-worker" "$domain_scratch/bin/libchariox-app-runtime.so" "$domain_scratch/domain-mounts/runtime/"
mount -o remount,ro,nodev,nosuid,noexec "$domain_scratch/domain-mounts/package"
mount -o remount,ro,nodev,nosuid,exec "$domain_scratch/domain-mounts/runtime"
# The production observer runs as the kernel owner, without supplementary
# groups/root capabilities. Matching owner UID grants namespace ptrace rights.
/usr/bin/setpriv --reuid="$domain_uid" --regid="$domain_gid" --clear-groups \
  /usr/bin/env -i GITHUB_ACTIONS=true RUNNER_ENVIRONMENT=github-hosted GITHUB_REPOSITORY=charioxai/chariox \
  "RUNNER_TEMP=$domain_temp" "CHARIOX_LINUX_DOMAIN_SCRATCH=$domain_scratch" \
  "CHARIOX_LINUX_DOMAIN_CGROUP=$domain_cgroup/apps" \
  "$domain_binary" worker_process::platform_linux::tests::hosted:: --ignored --nocapture --test-threads=1
