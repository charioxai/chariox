#!/usr/bin/env bash
set -Eeuo pipefail

filter="/usr/local/libexec/chariox/managed-provider-seccomp.bpf"
[[ -f "$filter" && ! -L "$filter" ]] || {
  printf 'managed provider seccomp filter is unavailable\n' >&2
  exit 1
}

# Under rootless Docker the container root is not host root, so setuid bwrap
# cannot write the sysctl --disable-userns needs; its unprivileged mode can.
bwrap=(/usr/bin/bwrap)
if ! awk 'NR == 1 { exit !($1 == 0 && $2 == 0) }' /proc/self/uid_map; then
  bwrap=(setpriv --no-new-privs /usr/bin/bwrap)
fi
exec 3<"$filter"
exec "${bwrap[@]}" --seccomp 3 "$@"
