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

if [ -L "$image_root" ] || [ ! -d "$image_root" ]; then
  echo "managed kernel image root must be a directory, not a symlink" >&2
  exit 1
fi

require_regular_file() {
  source_path=$1
  if [ -L "$source_path" ] || [ ! -f "$source_path" ]; then
    echo "managed kernel image contains an invalid file: $source_path" >&2
    exit 1
  fi
}

require_regular_file "$image_root/usr/local/bin/chariox-kernel"
require_regular_file "$image_root/usr/local/bin/chariox-managed-bootstrap"
require_regular_file "$image_root/usr/lib/chariox/release-manifest.json"
require_regular_file "$image_root/usr/lib/chariox/release-manifest.sig"
require_regular_file "$image_root/usr/lib/chariox/release-public-key"
require_regular_file "$image_root/etc/systemd/system/chariox-managed-bootstrap.service"
require_regular_file "$trusted_public_key"

node "$script_root/verify-image-release.mjs" "$image_root" "$expected_release_digest" "$trusted_public_key"

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

install -d -o chariox -g chariox -m 0700 "$state_root" "$state_root/home"
install -d -o root -g root -m 0755 "$install_root/usr/local/bin" "$install_root/usr/lib/chariox" "$install_root/etc/systemd/system"
install -o root -g root -m 0755 "$image_root/usr/local/bin/chariox-kernel" "$install_root/usr/local/bin/chariox-kernel"
install -o root -g root -m 0755 "$image_root/usr/local/bin/chariox-managed-bootstrap" "$install_root/usr/local/bin/chariox-managed-bootstrap"
install -o root -g root -m 0644 "$image_root/usr/lib/chariox/release-manifest.json" "$install_root/usr/lib/chariox/release-manifest.json"
install -o root -g root -m 0644 "$image_root/usr/lib/chariox/release-manifest.sig" "$install_root/usr/lib/chariox/release-manifest.sig"
install -o root -g root -m 0644 "$image_root/usr/lib/chariox/release-public-key" "$install_root/usr/lib/chariox/release-public-key"
install -o root -g root -m 0644 "$image_root/etc/systemd/system/chariox-managed-bootstrap.service" "$install_root/etc/systemd/system/chariox-managed-bootstrap.service"

systemctl daemon-reload
systemctl enable chariox-managed-bootstrap.service
