#!/usr/bin/env bash
set -Eeuo pipefail

filter="/usr/local/libexec/chariox/managed-provider-seccomp.bpf"
[[ -f "$filter" && ! -L "$filter" ]] || {
  printf 'managed provider seccomp filter is unavailable\n' >&2
  exit 1
}

# The provisioner admits provider sandboxes with no-new-privs. Use that same
# unprivileged setup on rootful Docker too: its setuid path may lack permission
# to write the sysctl required by --disable-userns. Provider caps and the
# nested-namespace seccomp filter remain unchanged.
exec 3<"$filter"
exec setpriv --no-new-privs /usr/bin/bwrap --seccomp 3 "$@"
