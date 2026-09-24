#!/usr/bin/env bash
set -Eeuo pipefail

filter="/usr/local/libexec/chariox/managed-provider-seccomp.bpf"
[[ -f "$filter" && ! -L "$filter" ]] || {
  printf 'managed provider seccomp filter is unavailable\n' >&2
  exit 1
}

exec 3<"$filter"
exec /usr/bin/bwrap --seccomp 3 "$@"
