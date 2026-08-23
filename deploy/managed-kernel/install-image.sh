#!/bin/sh
set -eu

if [ "$(id -u)" -ne 0 ]; then
  echo "install-image.sh must run as root" >&2
  exit 1
fi
if [ "$#" -ne 3 ]; then
  echo "usage: install-image.sh <managed-kernel-rootfs> <expected-release-digest> <trusted-public-key>" >&2
  exit 1
fi

image_root=$1
expected_release_digest=$2
trusted_public_key=$3
install_root=${CHARIOX_IMAGE_INSTALL_ROOT:-}
state_root=$install_root/var/lib/chariox
script_root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
staging_root=$(mktemp -d "${TMPDIR:-/tmp}/chariox-image-install.XXXXXX")
chmod 0700 "$staging_root"
cleanup() {
  if [ -n "${pending_release:-}" ] && [ -d "$pending_release" ]; then
    rm -rf -- "$pending_release"
  fi
  rm -rf -- "$staging_root"
}
trap cleanup EXIT HUP INT TERM

if [ -L "$image_root" ] || [ ! -d "$image_root" ]; then
  echo "managed kernel image root must be a directory, not a symlink" >&2
  exit 1
fi

mkdir "$staging_root/image"
(umask 000; cp -RP "$image_root/." "$staging_root/image/")
cp -P "$trusted_public_key" "$staging_root/trusted-public-key"
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
node "$script_root/verify-image-release.mjs" "$image_root" "$expected_release_digest" "$trusted_public_key"

require_regular_file "$image_root/usr/local/bin/chariox-kernel"
require_regular_file "$image_root/usr/local/bin/chariox-managed-bootstrap"
require_regular_file "$image_root/usr/lib/chariox/release-manifest.json"
require_regular_file "$image_root/usr/lib/chariox/release-manifest.sig"
require_regular_file "$image_root/usr/lib/chariox/release-public-key"
require_regular_file "$image_root/usr/lib/chariox/build-attestation.json"
require_regular_file "$image_root/usr/lib/chariox/build-attestation.sig"
require_regular_file "$image_root/usr/lib/chariox/builder-public-key"
require_regular_file "$image_root/etc/systemd/system/chariox-managed-bootstrap.service"
require_regular_file "$image_root/etc/systemd/system/chariox-rootless-docker.service"
require_regular_file "$image_root/etc/systemd/system/chariox-slice-broker.service"
require_directory "$image_root/usr/lib/chariox/slice-build-context"

install_lock=${CHARIOX_IMAGE_INSTALL_LOCK:-/run/lock/chariox-managed-image-install.lock}
exec 9>"$install_lock"
flock 9

if [ -L "$state_root" ] || { [ -e "$state_root" ] && [ ! -d "$state_root" ]; }; then
  echo "managed kernel state root is not a directory" >&2
  exit 1
fi
if [ -L "$state_root/home" ] || { [ -e "$state_root/home" ] && [ ! -d "$state_root/home" ]; }; then
  echo "managed kernel home is not a directory" >&2
  exit 1
fi
if [ -d "$state_root" ]; then
  if ! state_entry=$(find "$state_root" -mindepth 1 ! -path "$state_root/home" -print -quit); then
    echo "managed kernel state root could not be inspected" >&2
    exit 1
  fi
  if [ -n "$state_entry" ]; then
    echo "managed kernel state root is not pristine" >&2
    exit 1
  fi
fi

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
  useradd --system --gid chariox --home-dir /var/lib/chariox/home --shell /usr/sbin/nologin chariox
fi
if [ "$(getent passwd chariox | cut -d: -f6)" != "/var/lib/chariox/home" ]; then
  echo "existing chariox user has an incompatible home directory" >&2
  exit 1
fi
if [ "$(id -gn chariox)" != "chariox" ]; then
  echo "existing chariox user has an incompatible primary group" >&2
  exit 1
fi
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
usermod --append --groups chariox-slice chariox

install -d -o chariox -g chariox -m 0700 "$state_root" "$state_root/home"
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
    return
  fi
  ln -s "$target_path" "$temporary_link"
  mv -Tf -- "$temporary_link" "$link_path"
}

releases_root=$install_root/usr/lib/chariox/releases
release_name=${expected_release_digest#sha256:}
pending_release=$releases_root/.new-$release_name
published_release=$releases_root/$release_name
install -d -o root -g root -m 0755 \
  "$install_root/usr/local/bin" \
  "$install_root/usr/lib/chariox" \
  "$install_root/etc/systemd/system" \
  "$releases_root"
rm -rf -- "$pending_release"
if [ -e "$published_release" ] || [ -L "$published_release" ]; then
  if ! node "$script_root/verify-image-release.mjs" "$published_release" "$expected_release_digest" "$trusted_public_key"; then
    rm -rf -- "$published_release"
  fi
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
  install -o root -g root -m 0644 "$image_root/etc/systemd/system/chariox-rootless-docker.service" "$pending_release/etc/systemd/system/chariox-rootless-docker.service"
  install -o root -g root -m 0644 "$image_root/etc/systemd/system/chariox-slice-broker.service" "$pending_release/etc/systemd/system/chariox-slice-broker.service"
  (umask 000; cp -RP "$image_root/usr/lib/chariox/slice-build-context" "$pending_release/usr/lib/chariox/slice-build-context")
  node "$script_root/verify-image-release.mjs" "$pending_release" "$expected_release_digest" "$trusted_public_key"
  mv "$pending_release" "$published_release"
fi

atomic_symlink "../../../usr/lib/chariox/current/usr/local/bin/chariox-kernel" "$install_root/usr/local/bin/chariox-kernel"
atomic_symlink "../../../usr/lib/chariox/current/usr/local/bin/chariox-managed-bootstrap" "$install_root/usr/local/bin/chariox-managed-bootstrap"
atomic_symlink "../../../usr/lib/chariox/current/etc/systemd/system/chariox-managed-bootstrap.service" "$install_root/etc/systemd/system/chariox-managed-bootstrap.service"
atomic_symlink "../../../usr/lib/chariox/current/etc/systemd/system/chariox-rootless-docker.service" "$install_root/etc/systemd/system/chariox-rootless-docker.service"
atomic_symlink "../../../usr/lib/chariox/current/etc/systemd/system/chariox-slice-broker.service" "$install_root/etc/systemd/system/chariox-slice-broker.service"
atomic_symlink "current/usr/lib/chariox/release-manifest.json" "$install_root/usr/lib/chariox/release-manifest.json"
atomic_symlink "current/usr/lib/chariox/release-manifest.sig" "$install_root/usr/lib/chariox/release-manifest.sig"
atomic_symlink "current/usr/lib/chariox/release-public-key" "$install_root/usr/lib/chariox/release-public-key"
atomic_symlink "current/usr/lib/chariox/build-attestation.json" "$install_root/usr/lib/chariox/build-attestation.json"
atomic_symlink "current/usr/lib/chariox/build-attestation.sig" "$install_root/usr/lib/chariox/build-attestation.sig"
atomic_symlink "current/usr/lib/chariox/builder-public-key" "$install_root/usr/lib/chariox/builder-public-key"
atomic_symlink "current/usr/lib/chariox/slice-build-context" "$install_root/usr/lib/chariox/slice-build-context"
previous_current_target=
if [ -L "$install_root/usr/lib/chariox/current" ]; then
  previous_current_target=$(readlink "$install_root/usr/lib/chariox/current")
fi
atomic_symlink "releases/$release_name" "$install_root/usr/lib/chariox/current"

if ! rm -f -- "$install_root/etc/systemd/system/multi-user.target.wants/chariox-slice-broker.service" \
  || ! systemctl daemon-reload \
  || ! systemctl enable chariox-rootless-docker.service \
  || ! systemctl enable chariox-managed-bootstrap.service; then
  if [ -n "$previous_current_target" ]; then
    atomic_symlink "$previous_current_target" "$install_root/usr/lib/chariox/current"
  else
    rm -f -- "$install_root/usr/lib/chariox/current"
  fi
  systemctl daemon-reload >/dev/null 2>&1 || true
  echo "managed release activation failed; restored previous current release" >&2
  exit 1
fi
