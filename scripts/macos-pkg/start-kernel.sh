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
cd "$HOME"
exec "$R/usr/local/bin/chariox-kernel" >>"$CHARIOX_LOG_DIR/kernel.launchd.log" 2>&1
