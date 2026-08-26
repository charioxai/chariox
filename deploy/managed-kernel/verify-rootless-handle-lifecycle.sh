#!/bin/sh
set -eu

[ "$(id -u)" -eq 0 ] || { echo "rootless handle drill requires root" >&2; exit 1; }
docker_host=unix:///run/chariox-docker/docker.sock
docker_user=chariox-docker
docker_uid=$(id -u "$docker_user")
docker_gid=$(id -g "$docker_user")
helper=/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/managed-publication-access.sh
root=/var/lib/chariox-slice-share/slices/development/.handle-drill-$$
destination=$root/development
repository=$destination/repository
handle_root=/var/lib/chariox-docker/mount-handles
handle=$handle_root/handle-drill
holder_script=/var/lib/chariox-docker/handle-drill.mjs
ready=/var/lib/chariox-docker/handle-drill.ready
image=chariox-handle-drill:local
container=chariox-handle-drill
scratch=$(mktemp -d /tmp/chariox-handle-image.XXXXXX)
holder_pid=
docker_as_owner() {
  setpriv --reuid="$docker_uid" --regid="$docker_gid" --clear-groups -- \
    env HOME=/var/lib/chariox-docker/home DOCKER_HOST="$docker_host" PATH=/usr/bin:/bin docker "$@"
}
stop_holder() {
  if [ -n "$holder_pid" ]; then
    kill "$holder_pid" >/dev/null 2>&1 || true
    wait "$holder_pid" >/dev/null 2>&1 || true
    holder_pid=
  fi
}
cleanup() {
  stop_holder
  docker_as_owner rm -f "$container" >/dev/null 2>&1 || true
  docker_as_owner image rm -f "$image" >/dev/null 2>&1 || true
  rmdir "$handle" >/dev/null 2>&1 || true
  rm -f -- "$holder_script" "$ready"
  rmdir "$handle_root" >/dev/null 2>&1 || true
  rm -rf -- "$scratch" "$root"
}
trap cleanup EXIT HUP INT TERM

install -d -o chariox -g chariox-slice -m 0700 \
  "$root" \
  "$destination" \
  "$repository"
runuser -u chariox -- "$helper" grant "$root" "$destination" "$repository"
install -d -o "$docker_user" -g "$docker_user" -m 0700 "$handle_root"
if [ -L "$handle" ]; then
  rm -f -- "$handle"
elif [ -d "$handle" ]; then
  rmdir "$handle"
elif [ -e "$handle" ]; then
  echo "rootless handle drill path is obstructed" >&2
  exit 1
fi
install -d -m 0755 "$scratch/rootfs/bin"
install -m 0755 /bin/busybox "$scratch/rootfs/bin/busybox"
tar -C "$scratch/rootfs" -cf "$scratch/rootfs.tar" .
chown "$docker_user:$docker_user" "$scratch" "$scratch/rootfs.tar"
chmod 0700 "$scratch"
chmod 0600 "$scratch/rootfs.tar"
docker_as_owner import "$scratch/rootfs.tar" "$image" >/dev/null
cat > "$holder_script" <<'NODE'
import { spawnSync } from "node:child_process"
import { closeSync, constants, mkdirSync, openSync, rmSync, writeFileSync } from "node:fs"
const [source, handle, ready] = process.argv.slice(2)
const fd = openSync(source, 0x200000 | constants.O_DIRECTORY | constants.O_NOFOLLOW)
mkdirSync(handle, { recursive: true, mode: 0o700 })
const mounted = spawnSync("/usr/bin/mount", ["--bind", "/proc/self/fd/3", handle], {
  stdio: ["ignore", "pipe", "pipe", fd],
})
if (mounted.status !== 0) {
  rmSync(handle, { recursive: true, force: true })
  closeSync(fd)
  process.exit(1)
}
writeFileSync(ready, "ready", { mode: 0o600 })
process.on("SIGTERM", () => {
  spawnSync("/usr/bin/umount", [handle], { stdio: "ignore" })
  rmSync(handle, { recursive: true, force: true })
  closeSync(fd)
  process.exit(0)
})
setInterval(() => {}, 60_000)
NODE
chown "$docker_user:$docker_user" "$holder_script"
chmod 0600 "$holder_script"
start_holder() {
  rm -f -- "$ready"
  rootless_child=$(cat /run/chariox-docker/dockerd-rootless/child_pid)
  printf '%s\n' "$rootless_child" | grep -Eq '^[1-9][0-9]*$'
  setpriv --reuid="$docker_uid" --regid="$docker_gid" --clear-groups -- \
    nsenter --target "$rootless_child" --user --mount -- \
    node "$holder_script" "$repository" "$handle" "$ready" &
  holder_pid=$!
  attempt=0
  while [ ! -f "$ready" ]; do
    attempt=$((attempt + 1))
    [ "$attempt" -lt 100 ] || { echo "handle holder did not start" >&2; exit 1; }
    sleep 0.05
  done
}

start_holder
docker_as_owner create --name "$container" --user 1001:1001 \
  -v "$handle:/probe:rw" "$image" /bin/busybox sh -c 'printf mapped >> /probe/from-container' >/dev/null
[ "$(docker_as_owner inspect --format '{{(index .Mounts 0).Source}}' "$container")" = "$handle" ]
docker_as_owner start -a "$container"
runuser -u chariox -- sh -c 'test "$(cat "$1")" = mapped' _ "$repository/from-container"
stop_holder
if docker_as_owner start -a "$container" >/dev/null 2>&1; then
  echo "container restarted through a stale mount handle" >&2
  exit 1
fi
start_holder
docker_as_owner start -a "$container"
runuser -u chariox -- sh -c 'test "$(cat "$1")" = mappedmapped' _ "$repository/from-container"
stop_holder
runuser -u chariox -- "$helper" revoke "$root" "$destination" "$repository"
echo "rootless stable mount handle drill passed"
