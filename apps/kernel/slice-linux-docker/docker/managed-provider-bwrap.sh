#!/usr/bin/env bash
set -Eeuo pipefail

filter="/usr/local/libexec/chariox/managed-provider-seccomp.bpf"
[[ -f "$filter" && ! -L "$filter" ]] || {
  printf 'managed provider seccomp filter is unavailable\n' >&2
  exit 1
}

exec 3<"$filter"
# Run Bubblewrap unprivileged, exactly as the provisioner's compatibility probe
# does. The image's bwrap is setuid; in setuid mode the sandbox user namespace
# is created by root, so --disable-userns cannot write the namespace's own
# user.max_user_namespaces and every managed provider launch fails. Bubblewrap
# sets no_new_privs inside the sandbox for --seccomp anyway.
exec /usr/bin/setpriv --no-new-privs /usr/bin/bwrap --seccomp 3 "$@"
