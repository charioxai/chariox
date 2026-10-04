#!/usr/bin/env bash
# Regression drill in an owned Linux VM: enroll the real local Docker helper
# while /run stays noexec. Forward the installer's reviewed public input pins.
set -euo pipefail

[[ $(id -u) == 0 ]] || { echo 'run in the owned VM as root' >&2; exit 2; }
[[ $# -gt 1 ]] || { echo 'usage: noexec-docker-enrollment.sh INSTALLER INSTALLER_ARGS...' >&2; exit 2; }
installer=$1
shift
check_noexec() {
  findmnt -n -o OPTIONS --target /run | tr ',' '\n' | grep -qx noexec \
    || { echo '/run must remain noexec for this regression drill' >&2; exit 1; }
}
check_noexec
python3 "$installer" "$@"
check_noexec
if find /usr/lib/chariox/slice-local-dev -maxdepth 1 -type d \
    -name 'chariox-local-broker-build-*' -print -quit | grep -q .; then
  echo 'Buildx scratch remains after successful enrollment' >&2
  exit 1
fi
