#!/bin/bash
# Removes what the Chariox macOS package (dev.chariox.pkg) installed:
#   sudo /usr/local/libexec/chariox/uninstall.sh [--dry-run]
#   1. stops the kernel LaunchAgent in every logged-in user's gui domain and
#      removes it; it refuses while a Chariox kernel or App worker still runs;
#   2. withdraws the App runtime enrollment, then retires every runtime
#      generation with chariox-app-runtime-install cleanup, which takes each
#      generation's exclusive lease first;
#   3. removes the binaries the package put in /usr/local/bin and the slice
#      build context in /usr/local/share/chariox, but only the files whose bytes
#      are still the package's; a replaced file is kept and reported;
#   4. removes the package's own files and forgets its receipt.
# Every user's ~/.chariox (kernel state, Apps and their data) is kept; delete it
# to remove those too. --dry-run prints each change without making it. A failed
# run can be repeated.
set -euo pipefail
umask 022
PATH=/usr/bin:/bin:/usr/sbin:/sbin
export PATH

# Empty in the package. Tests render this script with a fake root under which
# every host path, including the launchctl and pkgutil it drives, resolves.
R='@@ROOT@@'
LABEL=dev.chariox.kernel
SUPPORT="$R/Library/Application Support/Chariox"
PACKAGE="$R/usr/local/libexec/chariox"
INSTALLER="$PACKAGE/chariox-app-runtime-install"
MANIFEST="$PACKAGE/bin.sha256"
CONTEXT="$R/usr/local/share/chariox/slice-build-context"
CONTEXT_MANIFEST="$PACKAGE/slice-build-context.sha256"
RUNTIMES="$SUPPORT/AppRuntimes"
ENROLLMENT_DIR="$SUPPORT/AppRuntime"
AGENT="$R/Library/LaunchAgents/$LABEL.plist"
KERNEL="$R/usr/local/bin/chariox-kernel"
LAUNCHCTL="$R/bin/launchctl"
PKGUTIL="$R/usr/sbin/pkgutil"

say() { printf '[chariox-uninstall] %s\n' "$*"; }
die() { printf '[chariox-uninstall] error: %s\n' "$*" >&2; exit 1; }
dry_run=0
case "$#:${1:-}" in
  0:) ;;
  1:--dry-run) dry_run=1 ;;
  *) echo "usage: sudo '$0' [--dry-run]" >&2; exit 2 ;;
esac
# act DESCRIPTION COMMAND...: print a change, then make it unless --dry-run.
act() {
  local what=$1; shift
  if [ "$dry_run" = 1 ]; then say "would $what"; return 0; fi
  say "$what"
  "$@" || die "failed to $what"
}
# Removes an emptied directory; one with entries this script does not own stays.
prune() {
  [ -d "$1" ] || return 0
  if [ -n "$(ls -A -- "$1")" ] && [ "$dry_run" = 0 ]; then say "kept $1: it is not empty"; return 0; fi
  act "remove $1" rmdir -- "$1"
}
# Root runs the runtime installer: only while root owns it and every directory
# above it, and no one else can write any of them.
trusted() {
  local path=$1
  while :; do
    [ "$(stat -f %u "$path")" = 0 ] || return 1
    case "$(stat -f %Sp "$path")" in ?????w????|????????w?) return 1 ;; esac
    [ "$path" != / ] || return 0
    path=$(dirname "$path")
  done
}
# Chariox kernels and App workers of any user, by executable path.
running() {
  ps -axo pid=,comm= | while read -r pid comm; do
    case "$comm" in "$KERNEL"|"$RUNTIMES"/*) printf '%s ' "$pid" ;; esac
  done
}
if [ -z "$R" ]; then
  [ "$(id -u)" = 0 ] || die "run it with sudo"
  [ ! -e "$INSTALLER" ] || trusted "$INSTALLER" || die "$INSTALLER or a directory above it is not root's alone"
  [ ! -e "$CONTEXT" ] || trusted "$CONTEXT" || die "$CONTEXT or a directory above it is not root's alone"
fi

# 1. Kernels. A loginwindow runs as each logged-in user.
for uid in $(ps -axo uid=,comm= | awk '$1 != 0 && $2 ~ /\/loginwindow$/ {print $1}' | sort -u); do
  if "$LAUNCHCTL" print "gui/$uid/$LABEL" >/dev/null 2>&1; then
    act "stop the kernel of uid $uid" "$LAUNCHCTL" bootout "gui/$uid/$LABEL"
  fi
done
# App workers run in their own sessions and may outlive their kernel briefly.
for _ in 1 2 3 4 5 6 7 8 9 10; do
  pids=$(running)
  [ -n "$pids" ] && [ "$dry_run" = 0 ] || break
  sleep 1
done
if [ -n "$pids" ]; then
  [ "$dry_run" = 1 ] || die "Chariox processes still run (pids $pids); quit them or log their users out, then rerun"
  say "would refuse: Chariox processes still run (pids $pids)"
fi
[ ! -e "$AGENT" ] || act "remove $AGENT" rm -f -- "$AGENT"

# 2. The App runtime. Without an enrollment no generation is current, so
# cleanup may retire each one once its lease is free.
[ ! -e "$ENROLLMENT_DIR/.runtime-enrollment.pending" ] \
  || die "an interrupted runtime enrollment is pending; install the package again, then uninstall"
[ ! -e "$ENROLLMENT_DIR/runtime-enrollment.json" ] \
  || act "withdraw the App runtime enrollment" rm -f -- "$ENROLLMENT_DIR/runtime-enrollment.json"
for generation in "$RUNTIMES"/* "$RUNTIMES"/.retiring-*; do
  name=${generation##*/}
  name=${name#.retiring-}
  [ "${#name}" = 64 ] || continue
  case "$name" in *[!0-9a-f]*) continue ;; esac
  act "retire App runtime $name" "$INSTALLER" cleanup --inventory-sha256 "$name"
done
[ ! -e "$ENROLLMENT_DIR/.runtime-installer.lock" ] \
  || act "remove the runtime installer lock" rm -f -- "$ENROLLMENT_DIR/.runtime-installer.lock"
prune "$RUNTIMES"
prune "$ENROLLMENT_DIR"

# 3. Binaries and the slice build context.
if [ -f "$MANIFEST" ]; then
  while read -r sum path; do
    case "$path" in /usr/local/bin/chariox*) ;; *) die "unexpected entry in $MANIFEST: $path" ;; esac
    file="$R$path"
    [ -f "$file" ] && [ ! -L "$file" ] || continue
    now=$(shasum -a 256 "$file")
    if [ "${now%% *}" = "$sum" ]; then
      act "remove $path" rm -f -- "$file"
    else
      say "kept $path: it changed after the package installed it"
    fi
  done < "$MANIFEST"
fi
# The slice build context: one shasum run checks every file of its manifest.
if [ -f "$CONTEXT_MANIFEST" ]; then
  bad=$(grep -v -m 1 -E '^[0-9a-f]{64}  /usr/local/share/chariox/slice-build-context(/\.?[A-Za-z0-9_+-][A-Za-z0-9._+-]*)+$' "$CONTEXT_MANIFEST" || true)
  [ -z "$bad" ] || die "unexpected entry in $CONTEXT_MANIFEST: $bad"
  unchanged=()
  while IFS= read -r line; do
    case "$line" in
      *': OK') unchanged+=("${line%: OK}") ;;
      *': FAILED') say "kept ${line%: FAILED}: it changed after the package installed it" ;;
    esac
  done < <(sed "s|  /|  $R/|" "$CONTEXT_MANIFEST" | shasum -a 256 -c - 2>/dev/null || true)
  [ "${#unchanged[@]}" = 0 ] \
    || act "remove the ${#unchanged[@]} unchanged files of $CONTEXT" rm -f -- "${unchanged[@]}"
fi
if [ -d "$CONTEXT" ]; then
  act "remove the emptied directories of $CONTEXT" find "$CONTEXT" -type d -empty -delete
  [ ! -d "$CONTEXT" ] || [ "$dry_run" = 1 ] || say "kept $CONTEXT: it is not empty"
fi
prune "$R/usr/local/share/chariox"

# 4. The package itself.
for file in "$INSTALLER" "$PACKAGE/start-kernel.sh" "$MANIFEST" "$CONTEXT_MANIFEST" "$PACKAGE/uninstall.sh"; do
  [ ! -e "$file" ] || act "remove $file" rm -f -- "$file"
done
[ ! -d "$PACKAGE/staging" ] || act "remove $PACKAGE/staging" rm -rf -- "$PACKAGE/staging"
prune "$PACKAGE"
prune "$SUPPORT"
if "$PKGUTIL" --pkg-info dev.chariox.pkg >/dev/null 2>&1; then
  act "forget the package receipt dev.chariox.pkg" "$PKGUTIL" --forget dev.chariox.pkg
fi
say "done; each user's ~/.chariox is kept"
