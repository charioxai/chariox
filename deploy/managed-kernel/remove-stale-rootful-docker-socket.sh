#!/bin/sh
set -eu

fail() {
  echo "remove-stale-rootful-docker-socket.sh: $*" >&2
  exit 1
}

if [ "$#" -ne 4 ]; then
  fail "usage: remove-stale-rootful-docker-socket.sh <socket-path> <docker-service-state> <docker-socket-state> <lsof-path>"
fi

socket_path=$1
docker_service_state=$2
docker_socket_state=$3
lsof_path=$4

[ "$docker_service_state" = inactive ] \
  || fail "docker.service remained $docker_service_state"
[ "$docker_socket_state" = inactive ] \
  || fail "docker.socket remained $docker_socket_state"
[ -x "$lsof_path" ] && [ ! -L "$lsof_path" ] \
  || fail "lsof must be an executable regular file"

if [ ! -e "$socket_path" ] && [ ! -L "$socket_path" ]; then
  exit 0
fi
[ ! -L "$socket_path" ] && [ -S "$socket_path" ] \
  || fail "$socket_path is not a trusted Unix socket"

socket_owners=$(
  "$lsof_path" -t -- "$socket_path" 2>/dev/null
) && lsof_status=0 || lsof_status=$?
case "$lsof_status" in
  0)
    [ -z "$socket_owners" ] \
      || fail "$socket_path is still owned by process: $socket_owners"
    fail "lsof reported success without a socket owner"
    ;;
  1)
    ;;
  *)
    fail "could not prove that $socket_path is unowned (lsof exit $lsof_status)"
    ;;
esac

rm -f -- "$socket_path"
if [ -e "$socket_path" ] || [ -L "$socket_path" ]; then
  fail "$socket_path remained after stale socket removal"
fi
