#!/bin/sh
# Bridge the system boot/stop lifecycle to the dedicated user's service manager.
set -eu
PATH=/usr/bin:/bin:/usr/sbin:/sbin
export PATH

ROOTLESS_HOME=/var/lib/chariox-docker/home
ROOTLESS_DATA_ROOT=/var/lib/chariox-docker/data
ROOTLESS_DOCKER_CONFIG="$ROOTLESS_HOME/.config/docker/daemon.json"
ROOTLESS_QUOTA_DAEMON_CONFIG='{"features":{"containerd-snapshotter":false},"storage-driver":"overlay2"}'

quota_storage_ready() {
  # Peer propagation can list one mount twice at the same target; distinct entries still fail.
  quota_mount=$(findmnt --noheadings --raw --target "$ROOTLESS_DATA_ROOT" --output TARGET,FSTYPE,OPTIONS 2>/dev/null | sort -u) || return 1
  set -- $quota_mount
  [ "$#" -eq 3 ] || return 1
  [ "$1" = "$ROOTLESS_DATA_ROOT" ] && [ "$2" = xfs ] || return 1
  case ",$3," in *,pquota,*|*,prjquota,*) ;; *) return 1 ;; esac
  quota_state=$(xfs_quota -x -D /dev/null -P /dev/null -c 'state -p' "$1" 2>/dev/null) || return 1
  printf '%s\n' "$quota_state" | grep -Fq 'Project quota state on ' || return 1
  printf '%s\n' "$quota_state" | grep -Fq 'Accounting: ON' || return 1
  printf '%s\n' "$quota_state" | grep -Fq 'Enforcement: ON' || return 1
  xfs_info "$1" 2>/dev/null | grep -Eq '(^|[[:space:],])ftype=1([[:space:],]|$)' || return 1
}

ensure_rootless_config_parent() {
  if [ -L "$ROOTLESS_HOME/.config" ] || { [ -e "$ROOTLESS_HOME/.config" ] && [ ! -d "$ROOTLESS_HOME/.config" ]; }; then
    echo "rootless Docker config parent is not a real directory" >&2
    exit 1
  fi
  # Repair ancestors left root-owned by earlier image preparation as well.
  install -d -o chariox-docker -g chariox-docker -m 0700 "$ROOTLESS_HOME/.config"
}

prepare_fresh_overlay2_backend() {
  [ "$(id -u)" -eq 0 ] || exit 1
  if [ -L "$ROOTLESS_DATA_ROOT" ] || { [ -e "$ROOTLESS_DATA_ROOT" ] && [ ! -d "$ROOTLESS_DATA_ROOT" ]; }; then
    echo "managed Docker data root is not a real directory" >&2
    exit 1
  fi
  if [ ! -d "$ROOTLESS_DATA_ROOT" ]; then
    install -d -o chariox-docker -g chariox-docker -m 0700 "$ROOTLESS_DATA_ROOT"
  fi
  if [ -L "$ROOTLESS_DOCKER_CONFIG" ] || { [ -e "$ROOTLESS_DOCKER_CONFIG" ] && [ ! -f "$ROOTLESS_DOCKER_CONFIG" ]; }; then
    echo "rootless Docker daemon configuration is not a regular file" >&2
    exit 1
  fi
  if ! quota_storage_ready; then
    if [ "${CHARIOX_PATH1_DATA_VOLUME_REQUIRED:-0}" = 1 ]; then
      echo "Path-1 rootless Docker requires its admitted XFS project-quota mount" >&2
      exit 1
    fi
    # Existing image builders and self-hosted engines keep their current
    # driver. Bounded managed admission separately requires the executable
    # allocator probe to prove this exact XFS project-quota backend.
    return 0
  fi
  ensure_rootless_config_parent
  if [ -e "$ROOTLESS_DOCKER_CONFIG" ]; then
    return 0
  fi
  if [ -d "$ROOTLESS_DATA_ROOT" ] && find "$ROOTLESS_DATA_ROOT" -mindepth 1 -print -quit | grep -q .; then
    # Preserve an existing rootless engine store. Classic overlay2 with XFS
    # project quotas is enabled only for a fresh image/data root.
    return 0
  fi
  chown chariox-docker:chariox-docker "$ROOTLESS_DATA_ROOT"
  chmod 0700 "$ROOTLESS_DATA_ROOT"
  install -d -o root -g root -m 0755 "$ROOTLESS_HOME/.config/docker"
  temporary="$ROOTLESS_DOCKER_CONFIG.new.$$"
  (umask 022; printf '%s\n' "$ROOTLESS_QUOTA_DAEMON_CONFIG" > "$temporary")
  chown root:root "$temporary"
  chmod 0644 "$temporary"
  mv "$temporary" "$ROOTLESS_DOCKER_CONFIG"
}

[ "$#" -eq 1 ] || exit 64
docker_uid=$(id -u chariox-docker)
case "$docker_uid" in ''|0|*[!0-9]*) exit 1 ;; esac

case "$1" in
  prepare)
    [ "$(id -u)" -eq 0 ] || exit 1
    prepare_fresh_overlay2_backend
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
          --format '{{.Driver}} {{.CgroupDriver}} {{.CgroupVersion}} {{.MemoryLimit}} {{.CPUCfsQuota}} {{.PidsLimit}}' 2>/dev/null); then
          storage_driver=${capability%% *}
          resource_capability=${capability#* }
          if [ -f "$ROOTLESS_DOCKER_CONFIG" ] && cmp -s "$ROOTLESS_DOCKER_CONFIG" - <<EOF
$ROOTLESS_QUOTA_DAEMON_CONFIG
EOF
          then
            quota_storage_ready || {
              echo "pinned overlay2 managed engine requires its exact XFS project-quota mount" >&2
              exit 1
            }
            [ "$storage_driver" = overlay2 ] || {
              echo "fresh managed rootless engine did not start with the pinned overlay2 driver" >&2
              exit 1
            }
          fi
          [ "$resource_capability" = "systemd 2 true true true" ] && exit 0
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
