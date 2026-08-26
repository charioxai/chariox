#!/bin/sh
set -eu

share_root=/var/lib/chariox-slice-share
docker_user=chariox-docker
subuid_start=231072
slice_uid=1001
mapped_slice_uid=$((subuid_start + slice_uid - 1))

fail() {
  echo "managed-publication-access.sh: $*" >&2
  exit 1
}

[ "$#" -ge 4 ] || fail "usage: managed-publication-access.sh <grant|revoke> <storage-root> <destination> <repository>..."
action=$1
storage_root=$2
destination=$3
shift 3
[ "$action" = grant ] || [ "$action" = revoke ] || fail "unsupported action"
cd /

canonical_directory() {
  candidate=$1
  [ -d "$candidate" ] && [ ! -L "$candidate" ] || fail "unsafe publication directory: $candidate"
  canonical=$(readlink -f -- "$candidate")
  [ "$canonical" = "$candidate" ] || fail "publication path is not canonical: $candidate"
  printf '%s\n' "$canonical"
}

[ "$(grep -Fxc "$docker_user:$subuid_start:65536" /etc/subuid)" -eq 1 ] \
  || fail "chariox-docker subordinate UID mapping is not pinned"
share_root=$(canonical_directory "$share_root")
storage_root=$(canonical_directory "$storage_root")
destination=$(canonical_directory "$destination")
case "$storage_root" in "$share_root"/*) ;; *) fail "storage root escaped the managed share" ;; esac
[ "$(dirname -- "$destination")" = "$storage_root" ] || fail "publication destination escaped its storage root"

chariox_uid=$(id -u chariox)
if [ "$action" = grant ]; then
  current=$share_root
  relative=${destination#"$share_root"/}
  old_ifs=$IFS
  IFS=/
  for component in $relative; do
    current=$current/$component
    canonical_directory "$current" >/dev/null
    setfacl -P -m "u:$docker_user:--x" -- "$current"
  done
  IFS=$old_ifs
fi

for repository in "$@"; do
  repository=$(canonical_directory "$repository")
  [ "$(dirname -- "$repository")" = "$destination" ] \
    || fail "repository is not a direct child of its publication"
  if [ "$action" = grant ]; then
    if find -P "$repository" -xdev -type f -links +1 -print -quit | grep -q .; then
      fail "publication contains a multiply linked file"
    fi
    setfacl -P -R -m "u:$chariox_uid:rwX,u:$mapped_slice_uid:rwX,g::---,m::rwx,o::---" -- "$repository"
    setfacl -P -m "u:$docker_user:--x" -- "$repository"
    find -P "$repository" -type d -exec setfacl -P -m \
      "d:u::rwx,d:u:$chariox_uid:rwx,d:u:$mapped_slice_uid:rwx,d:g::---,d:m::rwx,d:o::---" -- {} +
    if find -P "$repository" -xdev -type f -links +1 -print -quit | grep -q .; then
      setfacl -P -R -x "u:$mapped_slice_uid,u:$docker_user" -- "$repository" 2>/dev/null || true
      find -P "$repository" -type d -exec setfacl -P -x \
        "d:u:$mapped_slice_uid,d:u:$chariox_uid" -- {} + 2>/dev/null || true
      fail "publication gained a multiply linked file during access grant"
    fi
  else
    setfacl -P -R -x "u:$mapped_slice_uid,u:$docker_user" -- "$repository" 2>/dev/null || true
    find -P "$repository" -type d -exec setfacl -P -x \
      "d:u:$mapped_slice_uid,d:u:$chariox_uid" -- {} + 2>/dev/null || true
  fi
done
