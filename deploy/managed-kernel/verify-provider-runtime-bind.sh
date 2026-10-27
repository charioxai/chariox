#!/bin/sh
set -eu

fail() {
  echo "verify-provider-runtime-bind.sh: $*" >&2
  exit 1
}

[ "$(id -u)" -eq 0 ] || fail "run as root while preparing the managed image"
bwrap=/usr/bin/bwrap
[ -f "$bwrap" ] && [ ! -L "$bwrap" ] && [ -x "$bwrap" ] \
  && [ "$(stat -c %u "$bwrap")" -eq 0 ] \
  && [ -z "$(find "$bwrap" -perm /022 -print -quit)" ] \
  || fail "bubblewrap must be the root-owned, non-writable /usr/bin/bwrap executable"
id chariox >/dev/null 2>&1 || fail "managed kernel user is unavailable"

probe_root=$(mktemp -d /tmp/chariox-provider-runtime-bind.XXXXXX)
probe_file=$probe_root/mcp-config.json
cleanup() {
  rm -rf "$probe_root"
}
cleanup_signal() {
  signal=$1
  trap - 0 HUP INT TERM
  cleanup
  trap - "$signal"
  kill -s "$signal" "$$"
  exit 1
}
trap cleanup 0
trap 'cleanup_signal HUP' HUP
trap 'cleanup_signal INT' INT
trap 'cleanup_signal TERM' TERM

chown chariox:chariox "$probe_root"
chmod 0700 "$probe_root"
printf '%s\n' chariox-managed-runtime-bind-ok > "$probe_file"
chown chariox:chariox "$probe_file"
chmod 0600 "$probe_file"

runuser -u chariox -- "$bwrap" \
  --die-with-parent \
  --new-session \
  --unshare-user \
  --unshare-pid \
  --unshare-ipc \
  --unshare-uts \
  --unshare-cgroup-try \
  --disable-userns \
  --uid 0 \
  --gid 0 \
  --cap-drop ALL \
  --ro-bind / / \
  --tmpfs /tmp \
  --proc /proc \
  --dev /dev \
  --dir "$probe_root" \
  --ro-bind "$probe_root" "$probe_root" \
  -- /bin/sh -eu -c '
    probe_file=$1
    [ "$(cat "$probe_file")" = chariox-managed-runtime-bind-ok ]
    if (printf "%s\n" mutation > "$probe_file") 2>/dev/null; then
      exit 1
    fi
  ' sh "$probe_file" || fail "private runtime directory is not readable and immutable inside the managed namespace"

cleanup
trap - 0 HUP INT TERM
echo "managed provider runtime bind probe passed"
