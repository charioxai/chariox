#!/bin/sh
set -eu

state=/run/chariox-docker/dockerd-rootless/child_pid
attempt=0
child_pid=
while [ "$attempt" -lt 100 ]; do
  attempt=$((attempt + 1))
  if [ -f "$state" ] && [ ! -L "$state" ]; then
    child_pid=$(sed -n '1p' "$state")
    if printf '%s\n' "$child_pid" | grep -Eq '^[1-9][0-9]*$' \
      && [ -r "/proc/$child_pid/status" ] \
      && [ -L "/proc/$child_pid/ns/user" ] \
      && [ -L "/proc/$child_pid/ns/mnt" ]; then
      target_uid=$(awk '$1 == "Uid:" { print $2; exit }' "/proc/$child_pid/status")
      [ "$target_uid" = "$(id -u)" ] && break
    fi
  fi
  child_pid=
  sleep 0.1
done

[ -n "$child_pid" ] || {
  echo "rootless Docker namespace is not ready" >&2
  exit 1
}
[ "$#" -gt 0 ] || {
  echo "rootless Docker namespace command is missing" >&2
  exit 1
}

exec /usr/bin/nsenter --target "$child_pid" --user --mount -- "$@"
