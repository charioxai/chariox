#!/bin/sh
set -eu

if [ "$(id -u)" -ne 0 ]; then
  echo "install-image.sh must run as root" >&2
  exit 1
fi
if [ "$#" -ne 3 ] && [ "$#" -ne 4 ]; then
  echo "usage: install-image.sh <managed-kernel-rootfs> <expected-release-digest> <trusted-public-key> [path1|shared_host]" >&2
  exit 1
fi

image_root=$1
expected_release_digest=$2
trusted_public_key=$3
managed_provider_topology=${4:-shared_host}
case "$managed_provider_topology" in
  path1)
    selected_bootstrap_service=chariox-path1-managed-bootstrap.service
    trusted_builder_public_key=${CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY:-}
    if [ -z "$trusted_builder_public_key" ]; then
      echo "Path-1 install requires CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY outside the image" >&2
      exit 1
    fi
    ;;
  shared_host) selected_bootstrap_service=chariox-managed-bootstrap.service ;;
  *)
    echo "managed provider topology must be path1 or shared_host" >&2
    exit 1
    ;;
esac
install_root=${CHARIOX_IMAGE_INSTALL_ROOT:-}
state_root=$install_root/var/lib/chariox
managed_home=$install_root/home/chariox
managed_state=$managed_home/.chariox
legacy_home=$state_root/home
script_root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
staging_root=$(mktemp -d "${TMPDIR:-/tmp}/chariox-image-install.XXXXXX")
chmod 0700 "$staging_root"
cleanup() {
  if [ -n "${pending_release:-}" ] && [ -d "$pending_release" ]; then
    rm -rf -- "$pending_release"
  fi
  rm -rf -- "$staging_root"
}
terminate() {
  signal=$1
  trap - EXIT HUP INT TERM
  set +e
  cleanup
  kill -s "$signal" "$$"
  exit 1
}
trap cleanup EXIT
trap 'terminate HUP' HUP
trap 'terminate INT' INT
trap 'terminate TERM' TERM

if [ -L "$image_root" ] || [ ! -d "$image_root" ]; then
  echo "managed kernel image root must be a directory, not a symlink" >&2
  exit 1
fi
if [ "$managed_provider_topology" = path1 ]; then
  if [ -L "$trusted_builder_public_key" ] || [ ! -f "$trusted_builder_public_key" ]; then
    echo "trusted builder public key must be a regular file outside the image" >&2
    exit 1
  fi
  image_canonical=$(realpath "$image_root")
  builder_key_canonical=$(realpath "$trusted_builder_public_key")
  case "$builder_key_canonical" in
    "$image_canonical"|"$image_canonical"/*)
      echo "trusted builder public key must be supplied outside the image" >&2
      exit 1
      ;;
  esac
fi

mkdir "$staging_root/image"
(umask 000; cp -RP "$image_root/." "$staging_root/image/")
cp -P "$trusted_public_key" "$staging_root/trusted-public-key"
if [ "$managed_provider_topology" = path1 ]; then
  cp -P "$trusted_builder_public_key" "$staging_root/trusted-builder-public-key"
  trusted_builder_public_key=$staging_root/trusted-builder-public-key
fi
image_root=$staging_root/image
trusted_public_key=$staging_root/trusted-public-key

require_regular_file() {
  source_path=$1
  if [ -L "$source_path" ] || [ ! -f "$source_path" ]; then
    echo "managed kernel image contains an invalid file: $source_path" >&2
    exit 1
  fi
}

require_directory() {
  source_path=$1
  if [ -L "$source_path" ] || [ ! -d "$source_path" ]; then
    echo "managed kernel image contains an invalid directory: $source_path" >&2
    exit 1
  fi
}

require_regular_file "$trusted_public_key"
verify_selected_release() {
  if [ "$managed_provider_topology" = path1 ]; then
    node "$script_root/verify-image-release.mjs" "$@" path1 "$trusted_builder_public_key"
  else
    node "$script_root/verify-image-release.mjs" "$@" shared_host
  fi
}
verify_selected_release "$image_root" "$expected_release_digest" "$trusted_public_key"

require_regular_file "$image_root/usr/local/bin/chariox-kernel"
require_regular_file "$image_root/usr/local/bin/chariox-managed-bootstrap"
require_regular_file "$image_root/usr/lib/chariox/release-manifest.json"
require_regular_file "$image_root/usr/lib/chariox/release-manifest.sig"
require_regular_file "$image_root/usr/lib/chariox/release-public-key"
require_regular_file "$image_root/usr/lib/chariox/build-attestation.json"
require_regular_file "$image_root/usr/lib/chariox/build-attestation.sig"
require_regular_file "$image_root/usr/lib/chariox/builder-public-key"
require_regular_file "$image_root/etc/systemd/system/chariox-managed-bootstrap.service"
if [ -e "$image_root/etc/systemd/system/chariox-path1-managed-bootstrap.service" ] \
  || [ -L "$image_root/etc/systemd/system/chariox-path1-managed-bootstrap.service" ]; then
  require_regular_file "$image_root/etc/systemd/system/chariox-path1-managed-bootstrap.service"
fi
require_regular_file "$image_root/etc/systemd/system/chariox-disposable-worker-bootstrap.service"
require_regular_file "$image_root/etc/systemd/system/chariox-rootless-docker.service"
require_regular_file "$image_root/etc/systemd/system/chariox-slice-broker.service"
require_directory "$image_root/usr/lib/chariox/slice-build-context"
rootless_context=usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker
for rootless_file in managed-rootless-service.sh chariox-rootless-engine.service chariox-rootless-user-manager.conf; do
  require_regular_file "$image_root/$rootless_context/$rootless_file"
done

install_lock=${CHARIOX_IMAGE_INSTALL_LOCK:-/run/lock/chariox-managed-image-install.lock}
exec 9>"$install_lock"
flock 9

path_exists() {
  [ -e "$1" ] || [ -L "$1" ]
}

require_real_directory() {
  directory=$1
  label=$2
  if [ -L "$directory" ] || { [ -e "$directory" ] && [ ! -d "$directory" ]; }; then
    echo "$label is not a real directory" >&2
    exit 1
  fi
}

ensure_directory_parent() {
  directory=$1
  label=$2
  if path_exists "$directory"; then
    require_real_directory "$directory" "$label"
  else
    install -d -o root -g root -m 0755 "$directory"
  fi
}

require_real_ancestor_chain() {
  ancestor_path=$1
  ancestor_label=$2
  case "$ancestor_path" in
    /*/../*|/*/./*|*/..|*/.)
      echo "$ancestor_label contains an unsafe ancestor" >&2
      exit 1
      ;;
    /*) ;;
    *)
      echo "$ancestor_label must be an absolute path" >&2
      exit 1
      ;;
  esac
  while :; do
    if [ -L "$ancestor_path" ] || { [ -e "$ancestor_path" ] && [ ! -d "$ancestor_path" ]; }; then
      echo "$ancestor_label ancestor is not a real directory: $ancestor_path" >&2
      exit 1
    fi
    [ "$ancestor_path" = / ] && break
    ancestor_path=${ancestor_path%/*}
    [ -n "$ancestor_path" ] || ancestor_path=/
  done
}

require_root_private_file() {
  state_path=$1
  state_label=$2
  if [ -L "$state_path" ] || [ ! -f "$state_path" ]; then
    echo "$state_label is not a regular file" >&2
    exit 1
  fi
  state_owner=$(stat -c %u "$state_path") || {
    echo "$state_label owner could not be inspected" >&2
    exit 1
  }
  [ "$state_owner" = 0 ] || {
    echo "$state_label must be root-owned" >&2
    exit 1
  }
  state_mode=$(stat -c %a "$state_path") || {
    echo "$state_label mode could not be inspected" >&2
    exit 1
  }
  [ "$state_mode" = 600 ] || {
    echo "$state_label must have mode 0600" >&2
    exit 1
  }
  state_writable=$(find "$state_path" -maxdepth 0 -perm /022 -print -quit) || {
    echo "$state_label permissions could not be inspected" >&2
    exit 1
  }
  [ -z "$state_writable" ] || {
    echo "$state_label is writable by group or other" >&2
    exit 1
  }
}

repair_root_control_file() {
  control_path=$1
  control_label=$2
  if ! path_exists "$control_path"; then
    return 0
  fi
  if [ -L "$control_path" ] || [ ! -f "$control_path" ]; then
    echo "$control_label is not a real regular file" >&2
    exit 1
  fi
  chown root:root -- "$control_path"
  chmod 0600 -- "$control_path"
}

repair_root_control_tree() {
  control_path=$1
  control_label=$2
  if ! path_exists "$control_path"; then
    return 0
  fi
  require_real_directory "$control_path" "$control_label"
  control_symlink=$(find "$control_path" -type l -print -quit) || {
    echo "$control_label could not be inspected" >&2
    exit 1
  }
  [ -z "$control_symlink" ] || {
    echo "$control_label contains a symbolic link" >&2
    exit 1
  }
  chown -R root:root -- "$control_path"
  find "$control_path" -type d -exec chmod 0700 {} + || {
    echo "$control_label directory modes could not be repaired" >&2
    exit 1
  }
  find "$control_path" -type f -exec chmod 0600 {} + || {
    echo "$control_label file modes could not be repaired" >&2
    exit 1
  }
}

repair_root_control_state() {
  repair_root_control_tree "$state_root/managed" "managed control state"
  repair_root_control_tree "$state_root/disposable-worker" "disposable-worker control state"
  repair_root_control_tree "$state_root/kernels" "managed kernel presence state"
  repair_root_control_file "$state_root/managed-bootstrap.json" "managed bootstrap state"
  repair_root_control_file "$state_root/disposable-worker-bootstrap.json" "disposable-worker bootstrap state"
}

validate_state_root_entries() {
  if ! path_exists "$state_root"; then
    return 0
  fi
  require_real_directory "$state_root" "managed kernel state root"
  if ! unknown_state_entry=$(find "$state_root" -mindepth 1 -maxdepth 1 \
    ! \( \
      -name managed-bootstrap.json -o \
      -name disposable-worker-bootstrap.json -o \
      -name managed -o \
      -name disposable-worker -o \
      -name kernels -o \
      -name home -o \
      -name home-migration.json -o \
      -name home-migration-complete -o \
      -name provider-home -o \
      -name release-verification \
    \) -print -quit); then
    echo "managed kernel state root could not be inspected" >&2
    exit 1
  fi
  if [ -n "$unknown_state_entry" ]; then
    echo "managed kernel state root contains an unrecognized entry" >&2
    exit 1
  fi
}

migrate_legacy_home_transaction() {
  migration_journal=$state_root/home-migration.json
  migration_complete=$state_root/home-migration-complete
  migration_helper=$script_root/managed-kernel-home-migration.mjs

  require_real_ancestor_chain "$state_root" "managed kernel state root"
  require_real_ancestor_chain "$managed_home" "managed service-account home"

  if path_exists "$migration_complete"; then
    require_root_private_file "$migration_complete" "managed home migration completion marker"
    if ! [ -f "$migration_journal" ]; then
      echo "managed home migration completion marker has no journal" >&2
      exit 1
    fi
    migration_complete_value=$(cat "$migration_complete") || {
      echo "managed home migration completion marker could not be read" >&2
      exit 1
    }
    migration_complete_lines=$(wc -l < "$migration_complete" | tr -d ' ') || {
      echo "managed home migration completion marker could not be inspected" >&2
      exit 1
    }
    if [ "$migration_complete_lines" -ne 1 ] || [ "$migration_complete_value" != complete ]; then
      echo "managed home migration completion marker is invalid" >&2
      exit 1
    fi
  fi

  if path_exists "$migration_journal"; then
    require_root_private_file "$migration_journal" "managed home migration journal"
  else
    node "$migration_helper" plan \
      "$migration_journal" \
      "$state_root" "$legacy_home" "$managed_home" "$managed_state" \
      "$chariox_uid" "$chariox_gid"
    require_root_private_file "$migration_journal" "managed home migration journal"
  fi

  node "$migration_helper" apply \
    "$migration_journal" \
    "$state_root" "$legacy_home" "$managed_home" "$managed_state" \
    "$chariox_uid" "$chariox_gid"
  repair_root_control_state
  require_root_private_file "$migration_journal" "managed home migration journal"
  require_root_private_file "$migration_complete" "managed home migration completion marker"
}

if ! getent group chariox >/dev/null 2>&1; then
  groupadd --system chariox
fi
if ! getent group chariox-slice >/dev/null 2>&1; then
  groupadd --system chariox-slice
fi
if ! getent group chariox-docker >/dev/null 2>&1; then
  groupadd --system chariox-docker
fi
if ! id chariox >/dev/null 2>&1; then
  useradd --system --gid chariox --home-dir /home/chariox --shell /usr/sbin/nologin chariox
fi
chariox_home_from_passwd=$(getent passwd chariox | cut -d: -f6)
if [ "$chariox_home_from_passwd" = "/var/lib/chariox/home" ]; then
  usermod --home /home/chariox chariox
  chariox_home_from_passwd=$(getent passwd chariox | cut -d: -f6)
fi
if [ "$chariox_home_from_passwd" != "/home/chariox" ]; then
  echo "existing chariox user has an incompatible home directory" >&2
  exit 1
fi
if [ "$(id -gn chariox)" != "chariox" ]; then
  echo "existing chariox user has an incompatible primary group" >&2
  exit 1
fi
chariox_uid=$(id -u chariox)
chariox_gid=$(id -g chariox)
case "$chariox_uid:$chariox_gid" in
  ''|*[!0-9:]*|0:*|*:0)
    echo "invalid managed service-account uid or gid" >&2
    exit 1
    ;;
esac

require_real_ancestor_chain "$state_root" "managed kernel state root"
require_real_ancestor_chain "$managed_home" "managed service-account home"
install -d -o root -g root -m 0750 "$state_root"
validate_state_root_entries
migrate_legacy_home_transaction
validate_state_root_entries

if ! id chariox-docker >/dev/null 2>&1; then
  useradd --system --gid chariox-docker --home-dir /var/lib/chariox-docker/home --shell /usr/sbin/nologin chariox-docker
fi
if [ "$(getent passwd chariox-docker | cut -d: -f6)" != "/var/lib/chariox-docker/home" ]; then
  echo "existing chariox-docker user has an incompatible home directory" >&2
  exit 1
fi
if [ "$(id -gn chariox-docker)" != "chariox-docker" ]; then
  echo "existing chariox-docker user has an incompatible primary group" >&2
  exit 1
fi
docker_uid=$(id -u chariox-docker)
case "$docker_uid" in
  ''|0|*[!0-9]*) echo "invalid managed Docker uid" >&2; exit 1 ;;
esac
usermod --append --groups chariox-slice chariox

install -d -o chariox -g chariox -m 0700 "$managed_home" "$managed_state"
install -d -o chariox-docker -g chariox-docker -m 0700 \
  "$install_root/var/lib/chariox-docker" \
  "$install_root/var/lib/chariox-docker/home"
install -d -o root -g chariox-slice -m 0710 "$install_root/var/lib/chariox-slice-share"
setfacl -P -m "u:chariox-docker:--x" -- "$install_root/var/lib/chariox-slice-share"
install -d -o root -g root -m 0711 "$install_root/var/lib/chariox-slice-share/.broker-private"
install -d -o chariox-docker -g chariox-docker -m 0700 \
  "$install_root/var/lib/chariox-slice-share/.broker-private/output" \
  "$install_root/var/lib/chariox-slice-share/.broker-private/artifacts" \
  "$install_root/var/lib/chariox-slice-share/.broker-private/artifacts/states" \
  "$install_root/var/lib/chariox-slice-share/.broker-private/artifacts/backups"
install -d -o chariox-docker -g chariox-slice -m 2710 \
  "$install_root/var/lib/chariox-slice-share/.broker-private/control"
install -d -o chariox -g chariox-slice -m 0700 \
  "$install_root/var/lib/chariox-slice-share/slices" \
  "$install_root/var/lib/chariox-slice-share/slices/development"
atomic_symlink() {
  target_path=$1
  link_path=$2
  if [ -e "$link_path" ] && [ ! -L "$link_path" ]; then
    echo "managed release link is obstructed: $link_path" >&2
    exit 1
  fi
  temporary_link=$link_path.new
  if [ -e "$temporary_link" ] && [ ! -L "$temporary_link" ]; then
    echo "managed release temporary link is obstructed: $temporary_link" >&2
    exit 1
  fi
  rm -f -- "$temporary_link"
  if [ -L "$link_path" ] && [ "$(readlink "$link_path")" = "$target_path" ]; then
    node "$script_root/managed-kernel-upgrade-state.mjs" sync-directory "$(dirname "$link_path")"
    return
  fi
  ln -s "$target_path" "$temporary_link"
  node - "$temporary_link" "$link_path" <<'NODE'
const { renameSync } = require("node:fs")
const [source, destination] = process.argv.slice(2)
renameSync(source, destination)
NODE
  node "$script_root/managed-kernel-upgrade-state.mjs" sync-directory "$(dirname "$link_path")"
}

releases_root=$install_root/usr/lib/chariox/releases
release_name=${expected_release_digest#sha256:}
pending_release=$releases_root/.new-$release_name
published_release=$releases_root/$release_name
install -d -o root -g root -m 0755 \
  "$install_root/usr/local/bin" \
  "$install_root/usr/lib/chariox" \
  "$install_root/etc/systemd/system" \
  "$install_root/etc/systemd/user" \
  "$install_root/etc/systemd/system/user@$docker_uid.service.d" \
  "$releases_root"
if [ "$managed_provider_topology" = path1 ]; then
  trusted_builder_runtime_key=$install_root/etc/chariox/trusted-builder-public-key
  require_real_ancestor_chain "$install_root/etc/chariox" "trusted builder key directory"
  ensure_directory_parent "$install_root/etc/chariox" "trusted builder key directory"
  if [ "$(stat -c %u "$install_root/etc/chariox")" != 0 ] \
    || [ -n "$(find "$install_root/etc/chariox" -maxdepth 0 -perm /022 -print -quit)" ]; then
    echo "trusted builder key directory owner or permissions are unsafe" >&2
    exit 1
  fi
  if path_exists "$trusted_builder_runtime_key"; then
    require_regular_file "$trusted_builder_runtime_key"
    if [ "$(stat -c %u "$trusted_builder_runtime_key")" != 0 ] \
      || [ -n "$(find "$trusted_builder_runtime_key" -maxdepth 0 -perm /022 -print -quit)" ] \
      || ! cmp -s "$trusted_builder_public_key" "$trusted_builder_runtime_key"; then
      echo "installed trusted builder key differs from the independent input" >&2
      exit 1
    fi
  fi
  install -o root -g root -m 0644 "$trusted_builder_public_key" "$trusted_builder_runtime_key"
  node "$script_root/managed-kernel-upgrade-state.mjs" sync-tree "$install_root/etc/chariox"
fi
rm -rf -- "$pending_release"
if [ -e "$published_release" ] || [ -L "$published_release" ]; then
  if ! verify_selected_release "$published_release" "$expected_release_digest" "$trusted_public_key"; then
    echo "existing digest-named managed release is invalid; refusing to replace immutable release: $published_release" >&2
    exit 1
  fi
  node "$script_root/managed-kernel-upgrade-state.mjs" sync-tree "$published_release"
  node "$script_root/managed-kernel-upgrade-state.mjs" sync-directory "$releases_root"
fi
if [ ! -e "$published_release" ]; then
  install -d -o root -g root -m 0755 \
    "$pending_release" \
    "$pending_release/usr" \
    "$pending_release/usr/local" \
    "$pending_release/usr/local/bin" \
    "$pending_release/usr/lib" \
    "$pending_release/usr/lib/chariox" \
    "$pending_release/etc" \
    "$pending_release/etc/systemd" \
    "$pending_release/etc/systemd/system"
  install -o root -g root -m 0755 "$image_root/usr/local/bin/chariox-kernel" "$pending_release/usr/local/bin/chariox-kernel"
  install -o root -g root -m 0755 "$image_root/usr/local/bin/chariox-managed-bootstrap" "$pending_release/usr/local/bin/chariox-managed-bootstrap"
  install -o root -g root -m 0644 "$image_root/usr/lib/chariox/release-manifest.json" "$pending_release/usr/lib/chariox/release-manifest.json"
  install -o root -g root -m 0644 "$image_root/usr/lib/chariox/release-manifest.sig" "$pending_release/usr/lib/chariox/release-manifest.sig"
  install -o root -g root -m 0644 "$image_root/usr/lib/chariox/release-public-key" "$pending_release/usr/lib/chariox/release-public-key"
  install -o root -g root -m 0644 "$image_root/usr/lib/chariox/build-attestation.json" "$pending_release/usr/lib/chariox/build-attestation.json"
  install -o root -g root -m 0644 "$image_root/usr/lib/chariox/build-attestation.sig" "$pending_release/usr/lib/chariox/build-attestation.sig"
  install -o root -g root -m 0644 "$image_root/usr/lib/chariox/builder-public-key" "$pending_release/usr/lib/chariox/builder-public-key"
  install -o root -g root -m 0644 "$image_root/etc/systemd/system/chariox-managed-bootstrap.service" "$pending_release/etc/systemd/system/chariox-managed-bootstrap.service"
  if path_exists "$image_root/etc/systemd/system/chariox-path1-managed-bootstrap.service"; then
    install -o root -g root -m 0644 "$image_root/etc/systemd/system/chariox-path1-managed-bootstrap.service" "$pending_release/etc/systemd/system/chariox-path1-managed-bootstrap.service"
  fi
  install -o root -g root -m 0644 "$image_root/etc/systemd/system/chariox-disposable-worker-bootstrap.service" "$pending_release/etc/systemd/system/chariox-disposable-worker-bootstrap.service"
  install -o root -g root -m 0644 "$image_root/etc/systemd/system/chariox-rootless-docker.service" "$pending_release/etc/systemd/system/chariox-rootless-docker.service"
  install -o root -g root -m 0644 "$image_root/etc/systemd/system/chariox-slice-broker.service" "$pending_release/etc/systemd/system/chariox-slice-broker.service"
  (umask 000; cp -RP "$image_root/usr/lib/chariox/slice-build-context" "$pending_release/usr/lib/chariox/slice-build-context")
  verify_selected_release "$pending_release" "$expected_release_digest" "$trusted_public_key"
  node "$script_root/managed-kernel-upgrade-state.mjs" sync-tree "$pending_release"
  mv "$pending_release" "$published_release"
  node "$script_root/managed-kernel-upgrade-state.mjs" sync-directory "$releases_root"
fi

atomic_symlink "../../../usr/lib/chariox/current/usr/local/bin/chariox-kernel" "$install_root/usr/local/bin/chariox-kernel"
atomic_symlink "../../../usr/lib/chariox/current/usr/local/bin/chariox-managed-bootstrap" "$install_root/usr/local/bin/chariox-managed-bootstrap"
atomic_symlink "../../../usr/lib/chariox/current/etc/systemd/system/chariox-managed-bootstrap.service" "$install_root/etc/systemd/system/chariox-managed-bootstrap.service"
if path_exists "$image_root/etc/systemd/system/chariox-path1-managed-bootstrap.service"; then
  atomic_symlink "../../../usr/lib/chariox/current/etc/systemd/system/chariox-path1-managed-bootstrap.service" "$install_root/etc/systemd/system/chariox-path1-managed-bootstrap.service"
elif path_exists "$install_root/etc/systemd/system/chariox-path1-managed-bootstrap.service"; then
  echo "Path-1 managed-home service link is present but the signed release does not declare that service" >&2
  exit 1
fi
atomic_symlink "../../../usr/lib/chariox/current/etc/systemd/system/chariox-disposable-worker-bootstrap.service" "$install_root/etc/systemd/system/chariox-disposable-worker-bootstrap.service"
atomic_symlink "../../../usr/lib/chariox/current/etc/systemd/system/chariox-rootless-docker.service" "$install_root/etc/systemd/system/chariox-rootless-docker.service"
atomic_symlink "../../../usr/lib/chariox/current/etc/systemd/system/chariox-slice-broker.service" "$install_root/etc/systemd/system/chariox-slice-broker.service"
atomic_symlink "current/usr/lib/chariox/release-manifest.json" "$install_root/usr/lib/chariox/release-manifest.json"
atomic_symlink "current/usr/lib/chariox/release-manifest.sig" "$install_root/usr/lib/chariox/release-manifest.sig"
atomic_symlink "current/usr/lib/chariox/release-public-key" "$install_root/usr/lib/chariox/release-public-key"
atomic_symlink "current/usr/lib/chariox/build-attestation.json" "$install_root/usr/lib/chariox/build-attestation.json"
atomic_symlink "current/usr/lib/chariox/build-attestation.sig" "$install_root/usr/lib/chariox/build-attestation.sig"
atomic_symlink "current/usr/lib/chariox/builder-public-key" "$install_root/usr/lib/chariox/builder-public-key"
atomic_symlink "current/usr/lib/chariox/slice-build-context" "$install_root/usr/lib/chariox/slice-build-context"
atomic_symlink "../../../usr/lib/chariox/current/$rootless_context/chariox-rootless-engine.service" "$install_root/etc/systemd/user/chariox-rootless-engine.service"
atomic_symlink "../../../../usr/lib/chariox/current/$rootless_context/chariox-rootless-user-manager.conf" "$install_root/etc/systemd/system/user@$docker_uid.service.d/50-chariox-docker.conf"
previous_current_target=
if [ -L "$install_root/usr/lib/chariox/current" ]; then
  previous_current_target=$(readlink "$install_root/usr/lib/chariox/current")
fi
restore_previous_current() {
  if [ -n "$previous_current_target" ]; then
    atomic_symlink "$previous_current_target" "$install_root/usr/lib/chariox/current" || return 1
  else
    rm -f -- "$install_root/usr/lib/chariox/current" || return 1
    node "$script_root/managed-kernel-upgrade-state.mjs" sync-directory "$install_root/usr/lib/chariox" || return 1
  fi
}

if ! atomic_symlink "releases/$release_name" "$install_root/usr/lib/chariox/current"; then
  if restore_previous_current; then
    systemctl daemon-reload >/dev/null 2>&1 || true
    echo "managed release current-link activation failed; restored previous current release" >&2
  else
    echo "managed release current-link activation failed; previous current restoration remains incomplete" >&2
  fi
  exit 1
fi

if ! rm -f -- "$install_root/etc/systemd/system/multi-user.target.wants/chariox-slice-broker.service" \
  || ! systemctl daemon-reload \
  || ! loginctl enable-linger chariox-docker \
  || ! systemctl enable chariox-rootless-docker.service \
  || ! systemctl enable "$selected_bootstrap_service"; then
  if restore_previous_current; then
    systemctl daemon-reload >/dev/null 2>&1 || true
    echo "managed release activation failed; restored previous current release" >&2
  else
    echo "managed release activation failed; previous current restoration remains incomplete" >&2
  fi
  exit 1
fi
