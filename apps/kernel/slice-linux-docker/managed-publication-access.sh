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

[ "$#" -ge 4 ] || fail "usage: managed-publication-access.sh <grant|verify|revoke> <storage-root> <destination> <repository>..."
action=$1
storage_root=$2
destination=$3
shift 3
[ "$action" = grant ] || [ "$action" = verify ] || [ "$action" = revoke ] || fail "unsupported action"
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
docker_uid=$(id -u "$docker_user")

verify_traversal_access() {
  path=$1
  acl=$(getfacl -cpEn -- "$path" 2>/dev/null) \
    || fail "cannot inspect recovered publication traversal ACL"
  printf '%s\n' "$acl" | grep -Fqx "user:$docker_uid:--x" \
    || fail "recovered publication traversal ACL is invalid"
  printf '%s\n' "$acl" | grep -Eq '^mask::..x$' \
    || fail "recovered publication traversal ACL is invalid"
}

verify_publication_access() {
  repository=$1
  if find -P "$repository" -xdev -type f -links +1 -print -quit | grep -q .; then
    fail "publication contains a multiply linked file"
  fi
  find -P "$repository" -xdev -exec getfacl -cEn -- {} + >/dev/null 2>&1 \
    || fail "cannot inspect recovered publication access ACLs"
  getfacl -cEnRP --one-file-system -- "$repository" 2>/dev/null | awk \
    -v chariox_uid="$chariox_uid" -v mapped_slice_uid="$mapped_slice_uid" '
      BEGIN { RS = "" }
      {
        chariox = mapped = group = mask = other = 0
        default_owner = default_chariox = default_mapped = 0
        default_group = default_mask = default_other = 0
        directory = 0
        for (index = 1; index <= NF; index++) {
          line = $index
          if (line == "user:" chariox_uid ":rw-" || line == "user:" chariox_uid ":rwx") chariox = 1
          if (line == "user:" mapped_slice_uid ":rw-" || line == "user:" mapped_slice_uid ":rwx") mapped = 1
          if (line == "group::---") group = 1
          if (line == "mask::rw-" || line == "mask::rwx") mask = 1
          if (line == "other::---") other = 1
          if (line == "default:user::rwx") { directory = 1; default_owner = 1 }
          if (line == "default:user:" chariox_uid ":rwx") default_chariox = 1
          if (line == "default:user:" mapped_slice_uid ":rwx") default_mapped = 1
          if (line == "default:group::---") default_group = 1
          if (line == "default:mask::rwx") default_mask = 1
          if (line == "default:other::---") default_other = 1
        }
        if (!chariox || !mapped || !group || !mask || !other) exit 1
        if (directory && (!default_owner || !default_chariox || !default_mapped || !default_group || !default_mask || !default_other)) exit 1
      }
      END { if (NR == 0) exit 1 }
    ' || fail "recovered publication access ACLs are invalid"
  find -P "$repository" -xdev -type d -exec sh -c '
    chariox_uid=$1
    mapped_slice_uid=$2
    shift 2
    for path do
      acl=$(getfacl -cpEn -- "$path" 2>/dev/null) || exit 1
      for expected in \
        "user:$chariox_uid:rwx" \
        "user:$mapped_slice_uid:rwx" \
        "mask::rwx" \
        "default:user::rwx" \
        "default:user:$chariox_uid:rwx" \
        "default:user:$mapped_slice_uid:rwx" \
        "default:group::---" \
        "default:mask::rwx" \
        "default:other::---"
      do
        printf "%s\n" "$acl" | grep -Fqx "$expected" || exit 1
      done
    done
  ' _ "$chariox_uid" "$mapped_slice_uid" {} + \
    || fail "recovered publication directory ACLs are invalid"
}

if [ "$action" = grant ] || [ "$action" = verify ]; then
  current=$share_root
  relative=${destination#"$share_root"/}
  old_ifs=$IFS
  IFS=/
  for component in $relative; do
    current=$current/$component
    canonical_directory "$current" >/dev/null
    if [ "$action" = grant ]; then
      setfacl -P -m "u:$docker_user:--x" -- "$current"
    else
      verify_traversal_access "$current"
    fi
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
  elif [ "$action" = verify ]; then
    verify_publication_access "$repository"
    verify_traversal_access "$repository"
  else
    setfacl -P -R -x "u:$mapped_slice_uid,u:$docker_user" -- "$repository" 2>/dev/null || true
    find -P "$repository" -type d -exec setfacl -P -x \
      "d:u:$mapped_slice_uid,d:u:$chariox_uid" -- {} + 2>/dev/null || true
  fi
done
