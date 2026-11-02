#!/usr/bin/env bash
# The ordinary-user half of a local Linux Chariox install; it never needs root.
# Once an administrator has run install-root.sh for this user, this installs the
# kernel as the systemd --user unit chariox-kernel.service and starts it. The
# kernel then installs, updates and runs Apps itself.
#
#   install-user.sh install --kernel PATH   the chariox-kernel binary to run; the
#                                           chariox-app-wasm-worker beside it is
#                                           installed beside the installed kernel
#   install-user.sh uninstall               stop and remove the unit; ~/.chariox is kept
#
# Every change is printed; a repeated install with the same binary changes nothing
# and leaves the kernel running.
set -euo pipefail
umask 077

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
KERNEL=$HOME/.local/bin/chariox-kernel
# The kernel opens its worker only beside itself and trusts it only when it is not writable by its owner.
WORKER=$HOME/.local/bin/chariox-app-wasm-worker
START=$HOME/.local/lib/chariox/start-kernel.sh
UNIT=$HOME/.config/systemd/user/chariox-kernel.service
SERVICE=chariox-kernel.service

say() { printf '[chariox-install] %s\n' "$*"; }
die() { printf '[chariox-install] error: %s\n' "$*" >&2; exit 1; }
usage() { echo "usage: install-user.sh install --kernel PATH | install-user.sh uninstall"; }
# put SOURCE DEST MODE: copy unless DEST already has the same bytes and mode.
# Returns 1 when unchanged.
put() {
  if [[ -f "$2" && ! -L "$2" ]] && [[ "$(stat -c %a -- "$2")" == "${3#0}" ]] && cmp -s -- "$1" "$2"; then
    say "unchanged $2"
    return 1
  fi
  say "install $2"
  install -D -m "$3" -- "$1" "$2.chariox-new" && mv -f -- "$2.chariox-new" "$2" || die "failed to install $2"
}

[[ "$(id -u)" != 0 ]] || die "run this as the kernel's ordinary user, not root"
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
command=${1:-}; shift || true
case "$command" in
  install)
    kernel=
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --kernel) kernel=${2:?--kernel needs a path}; shift 2 ;;
        *) usage >&2; die "unknown argument: $1" ;;
      esac
    done
    [[ -n "$kernel" ]] || { usage >&2; exit 2; }
    kernel=$(realpath -e -- "$kernel") || die "no file $kernel"
    [[ -f "$kernel" && -x "$kernel" ]] || die "$kernel is not an executable file"
    worker=$(dirname -- "$kernel")/chariox-app-wasm-worker
    [[ -f "$worker" && -x "$worker" ]] || die "$worker is missing; an installed kernel cannot run any App without the worker built with it"
    restart=0
    install -d -m 0700 "$HOME/.chariox" "$HOME/.chariox/logs" "$HOME/.config/chariox"
    put "$worker" "$WORKER" 0555 && restart=1
    put "$kernel" "$KERNEL" 0755 && restart=1
    put "$here/start-kernel.sh" "$START" 0755 && restart=1
    if put "$here/chariox-kernel.service" "$UNIT" 0644; then
      restart=1
      say "reload systemd --user units"
      systemctl --user daemon-reload
    fi
    systemctl --user is-enabled --quiet "$SERVICE" 2>/dev/null || { say "enable $SERVICE"; systemctl --user enable --quiet "$SERVICE"; }
    if ! systemctl --user is-active --quiet "$SERVICE"; then
      say "start $SERVICE"
      systemctl --user start "$SERVICE"
    elif (( restart )); then
      say "restart $SERVICE"
      systemctl --user restart "$SERVICE"
    else
      say "unchanged $SERVICE (running)"
    fi
    systemctl --user is-active --quiet "$SERVICE" || die "$SERVICE did not start; see journalctl --user -u $SERVICE"
    say "done: the kernel runs as $(id -un) (systemctl --user status $SERVICE)"
    ;;
  uninstall)
    [[ $# -eq 0 ]] || { usage >&2; exit 2; }
    if systemctl --user is-active --quiet "$SERVICE" || systemctl --user is-enabled --quiet "$SERVICE" 2>/dev/null; then
      say "stop and disable $SERVICE"
      systemctl --user disable --now --quiet "$SERVICE"
    fi
    for path in "$UNIT" "$START" "$KERNEL" "$WORKER"; do
      if [[ -e "$path" ]]; then say "remove $path"; rm -f -- "$path"; fi
    done
    systemctl --user daemon-reload
    say "done: kept $HOME/.chariox. Apps keep their storage until they are uninstalled with their data"
    ;;
  -h|--help|help) usage ;;
  *) usage >&2; exit 2 ;;
esac
