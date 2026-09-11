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

[ "$#" -ge 4 ] || fail INVALID_ARGUMENTS
action=$1
storage_root=$2
destination=$3
shift 3
[ "$action" = grant ] || [ "$action" = verify ] || [ "$action" = revoke ] || fail INVALID_ACTION
cd /
script_root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
acl_validator=$script_root/managed-publication-acl.awk
[ -f "$acl_validator" ] && [ ! -L "$acl_validator" ] || fail ACL_VALIDATOR_INVALID
acl_snapshot=
cleanup() {
  [ -z "$acl_snapshot" ] || rm -f -- "$acl_snapshot"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

canonical_directory() {
  candidate=$1
  [ -d "$candidate" ] && [ ! -L "$candidate" ] || fail UNSAFE_DIRECTORY
  canonical=$(readlink -f -- "$candidate")
  [ "$canonical" = "$candidate" ] || fail NONCANONICAL_DIRECTORY
  printf '%s\n' "$canonical"
}

[ "$(grep -Fxc "$docker_user:$subuid_start:65536" /etc/subuid)" -eq 1 ] \
  || fail SUBUID_MAPPING_INVALID
share_root=$(canonical_directory "$share_root")
storage_root=$(canonical_directory "$storage_root")
destination=$(canonical_directory "$destination")
case "$storage_root" in "$share_root"/*) ;; *) fail STORAGE_ROOT_ESCAPE ;; esac
[ "$(dirname -- "$destination")" = "$storage_root" ] || fail DESTINATION_ESCAPE

chariox_uid=$(id -u chariox)
docker_uid=$(id -u "$docker_user")

reset_acl_snapshot() {
  if [ -z "$acl_snapshot" ]; then
    acl_snapshot=$(mktemp "${TMPDIR:-/tmp}/chariox-publication-acl.XXXXXX") \
      || fail ACL_INSPECTION_FAILED
  fi
  : >"$acl_snapshot" || fail ACL_INSPECTION_FAILED
}

validate_acl_output() {
  mode=$1
  required=$2
  shift 2
  reset_acl_snapshot
  "$@" >"$acl_snapshot" 2>/dev/null || fail ACL_INSPECTION_FAILED
  awk -v mode="$mode" -v required="$required" \
    -v chariox_uid="$chariox_uid" -v docker_uid="$docker_uid" \
    -v mapped_slice_uid="$mapped_slice_uid" \
    -f "$acl_validator" "$acl_snapshot" || fail ACL_INVALID
}

verify_traversal_access() {
  validate_acl_output traversal 1 getfacl -cpEn -- "$1"
}

reject_hard_links() {
  repository=$1
  reset_acl_snapshot
  find -P "$repository" -xdev -type f -links +1 -print -quit >"$acl_snapshot" 2>/dev/null \
    || fail ACL_INSPECTION_FAILED
  [ ! -s "$acl_snapshot" ] || fail HARD_LINK_DETECTED
}

verify_publication_access() {
  repository=$1
  reject_hard_links "$repository"
  reset_acl_snapshot
  find -P "$repository" -xdev ! -type d ! -type f ! -type l -print -quit >"$acl_snapshot" 2>/dev/null \
    || fail ACL_INSPECTION_FAILED
  [ ! -s "$acl_snapshot" ] || fail UNSUPPORTED_FILE_TYPE
  validate_acl_output repository 1 getfacl -cpEn -- "$repository"
  validate_acl_output directory 0 find -P "$repository" -xdev -mindepth 1 -type d \
    -exec getfacl -cpEn -- {} +
  validate_acl_output file 0 find -P "$repository" -xdev -type f \
    -exec getfacl -cpEn -- {} +
  reject_hard_links "$repository"
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
      setfacl -P -m "u:$docker_user:--x" -- "$current" || fail ACL_MUTATION_FAILED
    else
      verify_traversal_access "$current"
    fi
  done
  IFS=$old_ifs
fi

for repository in "$@"; do
  repository=$(canonical_directory "$repository")
  [ "$(dirname -- "$repository")" = "$destination" ] \
    || fail REPOSITORY_ESCAPE
  if [ "$action" = grant ]; then
    reject_hard_links "$repository"
    setfacl -P -R -m "u:$chariox_uid:rwX,u:$mapped_slice_uid:rwX,g::---,m::rwx,o::---" -- "$repository" \
      || fail ACL_MUTATION_FAILED
    setfacl -P -m "u:$docker_user:--x" -- "$repository" || fail ACL_MUTATION_FAILED
    find -P "$repository" -type d -exec setfacl -P -m \
      "d:u::rwx,d:u:$chariox_uid:rwx,d:u:$mapped_slice_uid:rwx,d:g::---,d:m::rwx,d:o::---" -- {} + \
      || fail ACL_MUTATION_FAILED
    reset_acl_snapshot
    find -P "$repository" -xdev -type f -links +1 -print -quit >"$acl_snapshot" 2>/dev/null \
      || fail ACL_INSPECTION_FAILED
    if [ -s "$acl_snapshot" ]; then
      setfacl -P -R -x "u:$mapped_slice_uid,u:$docker_user" -- "$repository" 2>/dev/null || true
      find -P "$repository" -type d -exec setfacl -P -x \
        "d:u:$mapped_slice_uid,d:u:$chariox_uid" -- {} + 2>/dev/null || true
      fail HARD_LINK_RACE
    fi
  elif [ "$action" = verify ]; then
    verify_publication_access "$repository"
  else
    setfacl -P -R -x "u:$mapped_slice_uid,u:$docker_user" -- "$repository" 2>/dev/null || true
    find -P "$repository" -type d -exec setfacl -P -x \
      "d:u:$mapped_slice_uid,d:u:$chariox_uid" -- {} + 2>/dev/null || true
  fi
done
