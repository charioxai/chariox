#!/bin/sh
set -eu

if [ "$(id -u)" -ne 0 ]; then
  echo "upgrade-image.sh must run as root" >&2
  exit 1
fi
if [ "$#" -ne 4 ]; then
  echo "usage: upgrade-image.sh <managed-kernel-rootfs> <expected-current-release-digest> <expected-new-release-digest> <trusted-public-key>" >&2
  exit 1
fi

image_root=$1
expected_current_digest=$2
expected_new_digest=$3
trusted_public_key=$4
install_root=${CHARIOX_MANAGED_UPGRADE_ROOT:-}
script_root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
service_name=chariox-managed-bootstrap.service
chariox_root=$install_root/usr/lib/chariox
releases_root=$chariox_root/releases
current_link=$chariox_root/current
receipt_path=${CHARIOX_MANAGED_UPGRADE_RECEIPT:-$install_root/var/lib/chariox/home/managed/bootstrap-receipt.json}
transaction_root=$chariox_root/.managed-kernel-upgrade
health_host=${CHARIOX_MANAGED_UPGRADE_HEALTH_HOST:-127.0.0.1}
health_port=${CHARIOX_MANAGED_UPGRADE_HEALTH_PORT:-43118}
health_timeout_ms=${CHARIOX_MANAGED_UPGRADE_HEALTH_TIMEOUT_MS:-30000}
staging_root=$(mktemp -d "${TMPDIR:-/tmp}/chariox-managed-upgrade.XXXXXX")
chmod 0700 "$staging_root"
pending_release=
pending_transaction=
transaction_active=0
rolling_back=0

cleanup() {
  if [ -n "$pending_release" ] && [ -d "$pending_release" ]; then
    rm -rf -- "$pending_release"
  fi
  if [ -n "$pending_transaction" ] && [ -d "$pending_transaction" ]; then
    rm -rf -- "$pending_transaction"
  fi
  rm -rf -- "$staging_root"
}

require_regular_file() {
  source_path=$1
  if [ -L "$source_path" ] || [ ! -f "$source_path" ]; then
    echo "managed kernel upgrade contains an invalid file: $source_path" >&2
    exit 1
  fi
}

require_directory() {
  source_path=$1
  if [ -L "$source_path" ] || [ ! -d "$source_path" ]; then
    echo "managed kernel upgrade contains an invalid directory: $source_path" >&2
    exit 1
  fi
}

read_single_line() {
  if [ -L "$1" ] || [ ! -f "$1" ]; then
    echo "managed kernel upgrade transaction is invalid" >&2
    return 1
  fi
  value=$(sed -n '1p' "$1")
  if [ -z "$value" ] || [ "$(wc -l < "$1" | tr -d ' ')" -ne 1 ]; then
    echo "managed kernel upgrade transaction is invalid" >&2
    return 1
  fi
  printf '%s\n' "$value"
}

atomic_symlink() {
  node "$script_root/managed-kernel-upgrade-state.mjs" atomic-symlink "$1" "$2"
}

atomic_receipt() {
  node "$script_root/managed-kernel-upgrade-state.mjs" atomic-file "$1" "$receipt_path"
}

write_phase() {
  node "$script_root/managed-kernel-upgrade-state.mjs" atomic-text "$1" "$transaction_root/phase"
}

protocol_version() {
  binary=$1
  version=$(timeout 5s "$binary" --print-local-daemon-protocol-version 2>/dev/null) || {
    echo "managed kernel binary did not report its local daemon protocol" >&2
    return 1
  }
  case "$version" in
    ''|*[!0-9]*)
      echo "managed kernel binary reported an invalid local daemon protocol" >&2
      return 1
      ;;
  esac
  printf '%s\n' "$version"
}

check_health() {
  expected_protocol=$1
  active_protocol=$(protocol_version "$current_link/usr/local/bin/chariox-kernel") || return 1
  if [ "$active_protocol" != "$expected_protocol" ]; then
    echo "active managed kernel protocol does not match the staged release" >&2
    return 1
  fi
  systemctl is-active --quiet "$service_name" || return 1
  node "$script_root/check-managed-kernel-health.mjs" "$health_host" "$health_port" "$health_timeout_ms"
}

validate_digest() {
  digest_hex=${1#sha256:}
  if [ "$digest_hex" = "$1" ] || [ "${#digest_hex}" -ne 64 ]; then
    return 1
  fi
  case "$digest_hex" in
    *[!a-f0-9]*) return 1 ;;
    *) return 0 ;;
  esac
}

rollback_transaction() {
  if [ ! -d "$transaction_root" ]; then
    transaction_active=0
    return 0
  fi
  rolling_back=1
  previous_target=$(read_single_line "$transaction_root/previous-current") || return 1
  previous_digest=$(read_single_line "$transaction_root/previous-digest") || return 1
  validate_digest "$previous_digest" || {
    echo "managed kernel upgrade transaction has an invalid previous digest" >&2
    return 1
  }
  [ "$previous_target" = "releases/${previous_digest#sha256:}" ] || {
    echo "managed kernel upgrade transaction has an invalid previous target" >&2
    return 1
  }
  node "$script_root/managed-kernel-upgrade-state.mjs" validate-receipt \
    "$transaction_root/previous-receipt.json" "$previous_digest"
  if ! systemctl stop "$service_name"; then
    echo "managed kernel rollback could not stop the kernel service" >&2
    return 1
  fi
  atomic_receipt "$transaction_root/previous-receipt.json"
  atomic_symlink "$previous_target" "$current_link"
  systemctl daemon-reload
  systemctl start "$service_name"
  previous_protocol=$(protocol_version "$current_link/usr/local/bin/chariox-kernel")
  check_health "$previous_protocol"
  rm -rf -- "$transaction_root"
  transaction_active=0
  rolling_back=0
}

recover_transaction() {
  if [ ! -e "$transaction_root" ]; then
    return 0
  fi
  if [ -L "$transaction_root" ] || [ ! -d "$transaction_root" ]; then
    echo "managed kernel upgrade transaction path is obstructed" >&2
    return 1
  fi
  phase=$(read_single_line "$transaction_root/phase") || return 1
  if [ "$phase" = committed ]; then
    target_digest=$(read_single_line "$transaction_root/target-digest") || return 1
    target_current=$(read_single_line "$transaction_root/target-current") || return 1
    validate_digest "$target_digest" || return 1
    [ "$target_current" = "releases/${target_digest#sha256:}" ] || return 1
    [ "$(readlink "$current_link")" = "$target_current" ] || return 1
    node "$script_root/managed-kernel-upgrade-state.mjs" validate-receipt "$receipt_path" "$target_digest"
    rm -rf -- "$transaction_root"
    return 0
  fi
  case "$phase" in prepared|stopped|activated) rollback_transaction ;;
    *) echo "managed kernel upgrade transaction phase is invalid" >&2; return 1 ;;
  esac
}

terminate() {
  signal=$1
  trap - EXIT HUP INT TERM
  set +e
  if [ "$transaction_active" -eq 1 ] && [ "$rolling_back" -eq 0 ]; then
    rollback_transaction
  fi
  cleanup
  kill -s "$signal" "$$"
  exit 1
}

trap cleanup EXIT
trap 'terminate HUP' HUP
trap 'terminate INT' INT
trap 'terminate TERM' TERM

if ! validate_digest "$expected_current_digest" || ! validate_digest "$expected_new_digest"; then
  echo "managed release digest is invalid" >&2
  exit 1
fi
if [ "$expected_current_digest" = "$expected_new_digest" ]; then
  echo "target release is not newer than current" >&2
  exit 1
fi
if [ -L "$image_root" ] || [ ! -d "$image_root" ]; then
  echo "managed kernel image root must be a directory, not a symlink" >&2
  exit 1
fi
require_regular_file "$trusted_public_key"
mkdir "$staging_root/image"
(umask 000; cp -RP "$image_root/." "$staging_root/image/")
cp "$trusted_public_key" "$staging_root/trusted-public-key"
image_root=$staging_root/image
trusted_public_key=$staging_root/trusted-public-key
require_regular_file "$trusted_public_key"

upgrade_lock=${CHARIOX_MANAGED_UPGRADE_LOCK:-/run/lock/chariox-managed-image-install.lock}
exec 9>"$upgrade_lock"
flock 9
recover_transaction

if [ ! -L "$current_link" ]; then
  echo "registered managed kernel current release link is missing" >&2
  exit 1
fi
current_target=$(readlink "$current_link")
expected_current_target=releases/${expected_current_digest#sha256:}
if [ "$current_target" != "$expected_current_target" ]; then
  echo "installed release does not match the expected current release" >&2
  exit 1
fi
require_regular_file "$receipt_path"
node "$script_root/managed-kernel-upgrade-state.mjs" validate-receipt "$receipt_path" "$expected_current_digest"
node "$script_root/verify-image-release.mjs" \
  "$releases_root/${expected_current_digest#sha256:}" "$expected_current_digest" "$trusted_public_key"
node "$script_root/verify-image-release.mjs" "$image_root" "$expected_new_digest" "$trusted_public_key"

current_protocol=$(protocol_version "$current_link/usr/local/bin/chariox-kernel")
target_protocol=$(protocol_version "$image_root/usr/local/bin/chariox-kernel")
if [ "$target_protocol" != "$current_protocol" ]; then
  echo "target local daemon protocol $target_protocol is incompatible with installed protocol $current_protocol" >&2
  exit 1
fi

release_name=${expected_new_digest#sha256:}
published_release=$releases_root/$release_name
if [ -e "$published_release" ] || [ -L "$published_release" ]; then
  require_directory "$published_release"
  node "$script_root/verify-image-release.mjs" "$published_release" "$expected_new_digest" "$trusted_public_key"
else
  pending_release=$(mktemp -d "$releases_root/.new-$release_name.XXXXXX")
  chmod 0755 "$pending_release"
  install -d -o root -g root -m 0755 \
    "$pending_release/usr/local/bin" \
    "$pending_release/usr/lib/chariox" \
    "$pending_release/etc/systemd/system"
  install -o root -g root -m 0755 "$image_root/usr/local/bin/chariox-kernel" "$pending_release/usr/local/bin/chariox-kernel"
  install -o root -g root -m 0755 "$image_root/usr/local/bin/chariox-managed-bootstrap" "$pending_release/usr/local/bin/chariox-managed-bootstrap"
  for release_file in release-manifest.json release-manifest.sig release-public-key build-attestation.json build-attestation.sig builder-public-key; do
    install -o root -g root -m 0644 "$image_root/usr/lib/chariox/$release_file" "$pending_release/usr/lib/chariox/$release_file"
  done
  for unit in chariox-managed-bootstrap.service chariox-rootless-docker.service chariox-slice-broker.service; do
    install -o root -g root -m 0644 "$image_root/etc/systemd/system/$unit" "$pending_release/etc/systemd/system/$unit"
  done
  (umask 000; cp -RP "$image_root/usr/lib/chariox/slice-build-context" "$pending_release/usr/lib/chariox/slice-build-context")
  node "$script_root/verify-image-release.mjs" "$pending_release" "$expected_new_digest" "$trusted_public_key"
  mv "$pending_release" "$published_release"
  pending_release=
fi

pending_transaction=$chariox_root/.managed-kernel-upgrade.pending
if [ -e "$pending_transaction" ] || [ -L "$pending_transaction" ]; then
  if [ -L "$pending_transaction" ] || [ ! -d "$pending_transaction" ]; then
    echo "managed kernel upgrade pending transaction path is obstructed" >&2
    exit 1
  fi
  rm -rf -- "$pending_transaction"
fi
install -d -o root -g root -m 0700 "$pending_transaction"
cp -P "$receipt_path" "$pending_transaction/previous-receipt.json"
chmod 0600 "$pending_transaction/previous-receipt.json"
node "$script_root/managed-kernel-upgrade-state.mjs" prepare-receipt \
  "$receipt_path" "$expected_current_digest" "$expected_new_digest" "$pending_transaction/target-receipt.json"
printf '%s\n' "$current_target" > "$pending_transaction/previous-current"
printf '%s\n' "$expected_current_digest" > "$pending_transaction/previous-digest"
printf '%s\n' "releases/$release_name" > "$pending_transaction/target-current"
printf '%s\n' "$expected_new_digest" > "$pending_transaction/target-digest"
printf '%s\n' prepared > "$pending_transaction/phase"
chmod 0600 "$pending_transaction"/*
mv "$pending_transaction" "$transaction_root"
pending_transaction=
transaction_active=1

if ! systemctl stop "$service_name"; then
  if rollback_transaction; then
    echo "managed kernel service could not be stopped; restored previous managed kernel release" >&2
  else
    echo "managed kernel service could not be stopped; rollback remains pending" >&2
  fi
  exit 1
fi
write_phase stopped
if ! atomic_receipt "$transaction_root/target-receipt.json" \
  || ! atomic_symlink "releases/$release_name" "$current_link"; then
  if rollback_transaction; then
    echo "managed kernel activation failed; restored previous managed kernel release" >&2
  else
    echo "managed kernel activation failed; rollback remains pending" >&2
  fi
  exit 1
fi
write_phase activated
if ! systemctl daemon-reload \
  || ! systemctl start "$service_name" \
  || ! check_health "$target_protocol"; then
  if rollback_transaction; then
    echo "managed kernel health check failed; restored previous managed kernel release" >&2
  else
    echo "managed kernel health check failed; rollback remains pending" >&2
  fi
  exit 1
fi
node "$script_root/managed-kernel-upgrade-state.mjs" validate-receipt "$receipt_path" "$expected_new_digest"
write_phase committed
rm -rf -- "$transaction_root"
transaction_active=0
printf 'managed kernel upgraded to %s\n' "$expected_new_digest"
