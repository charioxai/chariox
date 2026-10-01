#!/bin/bash
# The program of the Chariox kernel LaunchAgent (dev.chariox.kernel). launchd
# runs it as the logged-in user in their gui domain; it is the macOS counterpart
# of deploy/local-linux/chariox-kernel.service. It sets the kernel's environment
# from $HOME, then applies ~/.config/chariox/kernel.env (KEY=VALUE lines, taken
# literally; its values win) and execs the kernel, appending its output to
# $CHARIOX_LOG_DIR/kernel.launchd.log.
set -eu
umask 077
# Empty in the package; tests render a fake root.
R='@@ROOT@@'
: "${HOME:?launchd sets HOME}"
export CHARIOX_HOME="$HOME/.chariox"
export CHARIOX_LOG_DIR="$CHARIOX_HOME/logs"
export CHARIOX_KERNEL_HOST=127.0.0.1
env_file="$HOME/.config/chariox/kernel.env"
if [ -f "$env_file" ]; then
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in ''|'#'*) continue ;; esac
    key=${line%%=*}
    case "$key" in
      "$line"|''|[0-9]*|*[!A-Za-z0-9_]*)
        echo "start-kernel: ignoring a line of $env_file that is not KEY=VALUE" >&2
        continue ;;
    esac
    export "$key=${line#*=}"
  done < "$env_file"
fi
mkdir -p "$CHARIOX_LOG_DIR"
log="$CHARIOX_LOG_DIR/kernel.launchd.log"
# Every user's kernel defaults to the same endpoint, and so do their clients.
# While something already listens there (with Fast User Switching, usually
# another user's kernel), wait and start once it is free: a failed kernel would
# be restarted every 5 s, and one that exited cleanly never again.
# CHARIOX_KERNEL_PORT in kernel.env, for this user's clients too, runs this
# user's kernel beside the other one.
endpoint="${CHARIOX_KERNEL_HOST}/${CHARIOX_KERNEL_PORT:-43118}"
if (exec 3<>"/dev/tcp/$endpoint") 2>/dev/null; then
  echo "start-kernel: ${endpoint/\//:} is in use; waiting for it to be free. Set CHARIOX_KERNEL_PORT in $env_file to run this kernel beside the other one." >>"$log"
  while (exec 3<>"/dev/tcp/$endpoint") 2>/dev/null; do sleep 5; done
  echo "start-kernel: ${endpoint/\//:} is free; starting the kernel" >>"$log"
fi
cd "$HOME"
exec "$R/usr/local/bin/chariox-kernel" >>"$log" 2>&1
