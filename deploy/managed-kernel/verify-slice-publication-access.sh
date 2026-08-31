#!/bin/sh
set -eu

[ "$(id -u)" -eq 0 ] || { echo "publication ACL drill requires root" >&2; exit 1; }
helper=/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/managed-publication-access.sh
dockerfile=/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/docker/Dockerfile
root=/var/lib/chariox-slice-share/slices/development/.acl-drill-$$
destination=$root/development
repository=$destination/repository
mount_root=$(mktemp -d /tmp/chariox-slice-acl-bind.XXXXXX)
cleanup() {
  umount "$mount_root" >/dev/null 2>&1 || true
  rm -rf -- "$mount_root" "$root"
}
trap cleanup EXIT HUP INT TERM

grep -Fq 'groupadd --gid 1001 slice' "$dockerfile"
grep -Fq 'useradd --uid 1001 --gid 1001' "$dockerfile"

install -d -o chariox -g chariox-slice -m 0700 \
  "$root" \
  "$destination" \
  "$repository"
install -o chariox -g chariox-slice -m 0600 /dev/null "$destination/receipt.json"
install -o chariox -g chariox-slice -m 0600 /dev/null "$repository/host-created"
runuser -u chariox -- sh -c 'printf host > "$1"' _ "$repository/host-created"
runuser -u chariox -- ln -s host-created "$repository/legitimate-link"
runuser -u chariox -- "$helper" grant "$root" "$destination" "$repository"

runuser -u chariox-docker -- node -e \
  'const fs=require("node:fs"); const fd=fs.openSync(process.argv[1],0x200000|fs.constants.O_DIRECTORY|fs.constants.O_NOFOLLOW); fs.fstatSync(fd); fs.closeSync(fd)' \
  "$repository"
if runuser -u chariox-docker -- ls "$repository" >/dev/null 2>&1; then
  echo "rootless Docker principal can list publication content" >&2
  exit 1
fi
mount --bind "$repository" "$mount_root"
setpriv --reuid=232072 --regid=232072 --clear-groups -- \
  sh -c 'test "$(cat "$1")" = host' _ "$mount_root/host-created"
setpriv --reuid=232072 --regid=232072 --clear-groups -- \
  sh -c 'test "$(cat "$1")" = host' _ "$mount_root/legitimate-link"
setpriv --reuid=232072 --regid=232072 --clear-groups -- \
  sh -c 'umask 077; printf mapped > "$1/mapped-created"' _ "$mount_root"
runuser -u chariox -- sh -c 'test "$(cat "$1/mapped-created")" = mapped; mkdir "$1/host-directory"' _ "$repository"
setpriv --reuid=232072 --regid=232072 --clear-groups -- \
  sh -c 'cd "$1"' _ "$mount_root/host-directory"
if setpriv --reuid=232072 --regid=232072 --clear-groups -- cat "$destination/receipt.json" >/dev/null 2>&1; then
  echo "mapped slice user can read the private publication receipt" >&2
  exit 1
fi
if setpriv --reuid=232072 --regid=232072 --clear-groups -- cat /var/lib/chariox/home/.chariox/vault/vault.json >/dev/null 2>&1; then
  echo "mapped slice user can read the managed Vault" >&2
  exit 1
fi
if setpriv --reuid=232072 --regid=232072 --clear-groups -- cat "$repository/host-created" >/dev/null 2>&1; then
  echo "mapped slice user can traverse the original private publication path" >&2
  exit 1
fi
umount "$mount_root"
runuser -u chariox -- "$helper" revoke "$root" "$destination" "$repository"
if getfacl -cp "$repository" | grep -Eq '^user:232072:'; then
  echo "mapped slice ACL survived revocation" >&2
  exit 1
fi
echo "managed publication ACL drill passed"
