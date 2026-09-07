#!/bin/sh
# Bridge the system boot/stop lifecycle to the dedicated user's service manager.
set -eu
PATH=/usr/bin:/bin:/usr/sbin:/sbin
export PATH

[ "$#" -eq 1 ] || exit 64
docker_uid=$(id -u chariox-docker)
case "$docker_uid" in ''|0|*[!0-9]*) exit 1 ;; esac

case "$1" in
  prepare)
    [ "$(id -u)" -eq 0 ] || exit 1
    exec systemctl start "user@$docker_uid.service"
    ;;
  start|stop|ready)
    [ "$(id -u)" -eq "$docker_uid" ] || exit 1
    # The daemon's private runtime directory is intentionally NOT the bus dir.
    export XDG_RUNTIME_DIR="/run/user/$docker_uid"
    export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
    if [ "$1" = ready ]; then
      attempt=0
      while [ "$attempt" -lt 30 ]; do
        attempt=$((attempt + 1))
        if capability=$(timeout 2 docker --host unix:///run/chariox-docker/docker.sock info \
          --format '{{.CgroupDriver}} {{.CgroupVersion}} {{.MemoryLimit}} {{.CPUCfsQuota}} {{.PidsLimit}}' 2>/dev/null); then
          [ "$capability" = "systemd 2 true true true" ] && exit 0
          echo "rootless Docker resource controls are not enforced" >&2
          exit 1
        fi
        sleep 0.5
      done
      echo "rootless Docker readiness timed out" >&2
      exit 1
    fi
    if [ "$1" = start ]; then
      systemctl --user daemon-reload
      exec systemctl --user --wait start chariox-rootless-engine.service
    fi
    exec systemctl --user stop chariox-rootless-engine.service
    ;;
  *) exit 64 ;;
esac
