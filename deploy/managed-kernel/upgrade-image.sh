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
slice_build_context_link=$chariox_root/slice-build-context
signed_slice_build_context_target=current/usr/lib/chariox/slice-build-context
receipt_path=${CHARIOX_MANAGED_UPGRADE_RECEIPT:-$install_root/var/lib/chariox/home/managed/bootstrap-receipt.json}
release_override_path=${CHARIOX_MANAGED_UPGRADE_RELEASE_OVERRIDE:-${receipt_path%/*}/release-override.json}
transaction_root=$chariox_root/.managed-kernel-upgrade
terminal_transaction=$chariox_root/.managed-kernel-upgrade.terminal
health_host=${CHARIOX_MANAGED_UPGRADE_HEALTH_HOST:-127.0.0.1}
health_port=${CHARIOX_MANAGED_UPGRADE_HEALTH_PORT:-43118}
health_timeout_ms=${CHARIOX_MANAGED_UPGRADE_HEALTH_TIMEOUT_MS:-120000}
presence_root=$install_root/var/lib/chariox/home/kernels/active
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

require_root_owned_directory() {
  require_directory "$1"
  if [ "$(stat -c %u "$1")" != 0 ]; then
    echo "managed kernel upgrade authority owner is unsafe" >&2
    exit 1
  fi
  writable=$(find "$1" -maxdepth 0 -perm /022 -print -quit) || {
    echo "managed kernel upgrade authority permissions could not be inspected" >&2
    exit 1
  }
  if [ -n "$writable" ]; then
    echo "managed kernel upgrade authority permissions are unsafe" >&2
    exit 1
  fi
}

require_private_regular_file() {
  private_file_label=$2
  require_regular_file "$1"
  writable=$(find "$1" -maxdepth 0 -perm /022 -print -quit) || {
    echo "$private_file_label permissions could not be inspected" >&2
    exit 1
  }
  if [ -n "$writable" ]; then
    echo "$private_file_label permissions are unsafe" >&2
    exit 1
  fi
}

require_root_owned_private_regular_file() {
  require_private_regular_file "$1" "$2"
  if [ "$(stat -c %u "$1")" != 0 ]; then
    echo "$2 owner is unsafe" >&2
    exit 1
  fi
}

require_root_owned_ancestor_chain() {
  authority_path=$1
  authority_label=$2
  case "$authority_path" in
    /*/../*|/*/./*|*/..|*/.)
      echo "$authority_label ancestor is unsafe" >&2
      exit 1
      ;;
    /*) ;;
    *)
      echo "$authority_label path must be absolute" >&2
      exit 1
      ;;
  esac
  authority_ancestor=${authority_path%/*}
  [ -n "$authority_ancestor" ] || authority_ancestor=/
  while :; do
    if [ -L "$authority_ancestor" ] || [ ! -d "$authority_ancestor" ]; then
      echo "$authority_label ancestor is unsafe: $authority_ancestor" >&2
      exit 1
    fi
    if [ "$authority_ancestor" != / ] && [ "$(stat -c %u "$authority_ancestor")" != 0 ]; then
      echo "$authority_label ancestor owner is unsafe: $authority_ancestor" >&2
      exit 1
    fi
    writable=$(find "$authority_ancestor" -maxdepth 0 -perm /022 -print -quit) || {
      echo "$authority_label ancestor permissions could not be inspected" >&2
      exit 1
    }
    if [ -n "$writable" ]; then
      sticky=$(find "$authority_ancestor" -maxdepth 0 -perm -1000 -print -quit) || {
        echo "$authority_label ancestor permissions could not be inspected" >&2
        exit 1
      }
      if [ -z "$sticky" ]; then
        echo "$authority_label ancestor permissions are unsafe: $authority_ancestor" >&2
        exit 1
      fi
    fi
    [ "$authority_ancestor" = / ] && break
    authority_ancestor=${authority_ancestor%/*}
    [ -n "$authority_ancestor" ] || authority_ancestor=/
  done
}

require_safe_ancestor_chain() {
  authority_path=$1
  authority_label=$2
  case "$authority_path" in
    /*/../*|/*/./*|*/..|*/.)
      echo "$authority_label ancestor is unsafe" >&2
      exit 1
      ;;
    /*) ;;
    *)
      echo "$authority_label path must be absolute" >&2
      exit 1
      ;;
  esac
  authority_owner=$(stat -c %u "$authority_path") || {
    echo "$authority_label owner could not be inspected" >&2
    exit 1
  }
  authority_ancestor=${authority_path%/*}
  [ -n "$authority_ancestor" ] || authority_ancestor=/
  while :; do
    if [ -L "$authority_ancestor" ] || [ ! -d "$authority_ancestor" ]; then
      echo "$authority_label ancestor is unsafe: $authority_ancestor" >&2
      exit 1
    fi
    ancestor_owner=$(stat -c %u "$authority_ancestor") || {
      echo "$authority_label ancestor owner could not be inspected" >&2
      exit 1
    }
    if [ "$authority_ancestor" != / ] \
      && [ "$ancestor_owner" != 0 ] \
      && [ "$ancestor_owner" != "$authority_owner" ]; then
      echo "$authority_label ancestor owner is unsafe: $authority_ancestor" >&2
      exit 1
    fi
    writable=$(find "$authority_ancestor" -maxdepth 0 -perm /022 -print -quit) || {
      echo "$authority_label ancestor permissions could not be inspected" >&2
      exit 1
    }
    if [ -n "$writable" ]; then
      sticky=$(find "$authority_ancestor" -maxdepth 0 -perm -1000 -print -quit) || {
        echo "$authority_label ancestor permissions could not be inspected" >&2
        exit 1
      }
      if [ -z "$sticky" ]; then
        echo "$authority_label ancestor permissions are unsafe: $authority_ancestor" >&2
        exit 1
      fi
    fi
    [ "$authority_ancestor" = / ] && break
    authority_ancestor=${authority_ancestor%/*}
    [ -n "$authority_ancestor" ] || authority_ancestor=/
  done
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

atomic_release_override() {
  node "$script_root/managed-kernel-upgrade-state.mjs" atomic-sidecar \
    "$1" "$release_override_path" "$receipt_path"
}

remove_release_override() {
  node "$script_root/managed-kernel-upgrade-state.mjs" remove-state-file "$release_override_path"
}

write_phase() {
  node "$script_root/managed-kernel-upgrade-state.mjs" atomic-text "$1" "$transaction_root/phase"
}

verify_slice_build_context_facade() {
  expected_target=$1
  if [ ! -L "$slice_build_context_link" ] \
    || [ "$(readlink "$slice_build_context_link")" != "$expected_target" ] \
    || [ ! -d "$slice_build_context_link" ]; then
    echo "managed kernel slice build context facade is invalid" >&2
    return 1
  fi
}

verify_signed_slice_build_context_facade() {
  verify_slice_build_context_facade "$signed_slice_build_context_target" || return 1
  facade_path=$(readlink -f "$slice_build_context_link") || return 1
  signed_path=$(readlink -f "$current_link/usr/lib/chariox/slice-build-context") || return 1
  if [ "$facade_path" != "$signed_path" ]; then
    echo "managed kernel slice build context facade is outside the current signed release" >&2
    return 1
  fi
}

discard_terminal_transaction() {
  rm -rf -- "$terminal_transaction" || return 1
  node "$script_root/managed-kernel-upgrade-state.mjs" sync-directory "$chariox_root"
}

tombstone_transaction() {
  node "$script_root/managed-kernel-upgrade-state.mjs" tombstone-transaction \
    "$transaction_root" "$terminal_transaction" || return 1
  discard_terminal_transaction
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
  expected_digest=$2
  not_before_ms=$3
  active_protocol=$(protocol_version "$current_link/usr/local/bin/chariox-kernel") || return 1
  if [ "$active_protocol" != "$expected_protocol" ]; then
    echo "active managed kernel protocol does not match the staged release" >&2
    return 1
  fi
  systemctl is-active --quiet "$service_name" || return 1
  node "$script_root/check-managed-kernel-health.mjs" \
    "$health_host" "$health_port" "$health_timeout_ms" \
    "$receipt_path" "$release_override_path" "$presence_root" \
    "$expected_protocol" "$expected_digest" "$not_before_ms"
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
  previous_slice_build_context=$(read_single_line "$transaction_root/previous-slice-build-context") || return 1
  validate_digest "$previous_digest" || {
    echo "managed kernel upgrade transaction has an invalid previous digest" >&2
    return 1
  }
  [ "$previous_target" = "releases/${previous_digest#sha256:}" ] || {
    echo "managed kernel upgrade transaction has an invalid previous target" >&2
    return 1
  }
  node "$script_root/managed-kernel-upgrade-state.mjs" validate-receipt \
    "$transaction_root/previous-receipt.json" "$previous_digest" \
    "$transaction_root/previous-release-override.json" || return 1
  target_digest=$(read_single_line "$transaction_root/target-digest") || return 1
  target_protocol=$(read_single_line "$transaction_root/target-protocol") || return 1
  previous_protocol=$(read_single_line "$transaction_root/previous-protocol") || return 1
  validate_digest "$target_digest" || return 1
  node "$script_root/managed-kernel-upgrade-state.mjs" validate-protocol-transition \
    "$chariox_root/$previous_target" "$previous_protocol" \
    "$releases_root/${target_digest#sha256:}" "$target_protocol" || return 1
  if ! systemctl stop "$service_name"; then
    echo "managed kernel rollback could not stop the kernel service" >&2
    return 1
  fi
  atomic_receipt "$transaction_root/previous-receipt.json" || return 1
  previous_override_present=$(read_single_line "$transaction_root/previous-release-override-present") || return 1
  case "$previous_override_present" in
    yes) atomic_release_override "$transaction_root/previous-release-override.json" || return 1 ;;
    no) remove_release_override || return 1 ;;
    *) echo "managed kernel upgrade transaction has an invalid release override marker" >&2; return 1 ;;
  esac
  atomic_symlink "$previous_target" "$current_link" || return 1
  atomic_symlink "$previous_slice_build_context" "$slice_build_context_link" || return 1
  verify_slice_build_context_facade "$previous_slice_build_context" || return 1
  systemctl daemon-reload || return 1
  health_not_before_ms=$(node -e 'process.stdout.write(String(Date.now()))') || return 1
  systemctl start "$service_name" || return 1
  active_previous_protocol=$(protocol_version "$current_link/usr/local/bin/chariox-kernel") || return 1
  [ "$active_previous_protocol" = "$previous_protocol" ] || return 1
  check_health "$active_previous_protocol" "$previous_digest" "$health_not_before_ms" || return 1
  node "$script_root/managed-kernel-upgrade-state.mjs" validate-receipt-match \
    "$receipt_path" "$transaction_root/previous-receipt.json" "$previous_digest" \
    "$release_override_path" "$transaction_root/previous-release-override.json" || return 1
  write_phase rolled_back || return 1
  tombstone_transaction || return 1
  transaction_active=0
  rolling_back=0
}

recover_terminal_transaction() {
  if [ ! -e "$terminal_transaction" ]; then
    return 0
  fi
  if [ -L "$terminal_transaction" ] || [ ! -d "$terminal_transaction" ]; then
    echo "managed kernel upgrade terminal transaction path is obstructed" >&2
    return 1
  fi
  terminal_phase=$(read_single_line "$terminal_transaction/phase") || return 1
  case "$terminal_phase" in
    committed)
      terminal_digest=$(read_single_line "$terminal_transaction/target-digest") || return 1
      terminal_current=$(read_single_line "$terminal_transaction/target-current") || return 1
      terminal_slice_build_context=$(read_single_line "$terminal_transaction/target-slice-build-context") || return 1
      terminal_receipt=$terminal_transaction/target-receipt.json
      terminal_override=$terminal_transaction/target-release-override.json
      ;;
    rolled_back)
      terminal_digest=$(read_single_line "$terminal_transaction/previous-digest") || return 1
      terminal_current=$(read_single_line "$terminal_transaction/previous-current") || return 1
      terminal_slice_build_context=$(read_single_line "$terminal_transaction/previous-slice-build-context") || return 1
      terminal_receipt=$terminal_transaction/previous-receipt.json
      terminal_override=$terminal_transaction/previous-release-override.json
      ;;
    *)
      echo "managed kernel upgrade terminal transaction phase is invalid" >&2
      return 1
      ;;
  esac
  validate_digest "$terminal_digest" || return 1
  [ "$terminal_current" = "releases/${terminal_digest#sha256:}" ] || return 1
  [ "$(readlink "$current_link")" = "$terminal_current" ] || return 1
  [ "$(readlink "$slice_build_context_link")" = "$terminal_slice_build_context" ] || return 1
  verify_slice_build_context_facade "$terminal_slice_build_context" || return 1
  node "$script_root/managed-kernel-upgrade-state.mjs" validate-receipt-match \
    "$receipt_path" "$terminal_receipt" "$terminal_digest" \
    "$release_override_path" "$terminal_override" || return 1
  discard_terminal_transaction
}

recover_transaction() {
  recover_terminal_transaction || return 1
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
    target_slice_build_context=$(read_single_line "$transaction_root/target-slice-build-context") || return 1
    [ "$target_slice_build_context" = "$signed_slice_build_context_target" ] || return 1
    verify_signed_slice_build_context_facade || return 1
    node "$script_root/managed-kernel-upgrade-state.mjs" validate-receipt-match \
      "$receipt_path" "$transaction_root/target-receipt.json" "$target_digest" \
      "$release_override_path" "$transaction_root/target-release-override.json"
    tombstone_transaction
    return 0
  fi
  if [ "$phase" = rolled_back ]; then
    previous_digest=$(read_single_line "$transaction_root/previous-digest") || return 1
    previous_current=$(read_single_line "$transaction_root/previous-current") || return 1
    validate_digest "$previous_digest" || return 1
    [ "$previous_current" = "releases/${previous_digest#sha256:}" ] || return 1
    [ "$(readlink "$current_link")" = "$previous_current" ] || return 1
    previous_slice_build_context=$(read_single_line "$transaction_root/previous-slice-build-context") || return 1
    verify_slice_build_context_facade "$previous_slice_build_context" || return 1
    node "$script_root/managed-kernel-upgrade-state.mjs" validate-receipt-match \
      "$receipt_path" "$transaction_root/previous-receipt.json" "$previous_digest" \
      "$release_override_path" "$transaction_root/previous-release-override.json" || return 1
    tombstone_transaction || return 1
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
require_root_owned_private_regular_file "$trusted_public_key" "trusted release public key"
require_root_owned_ancestor_chain "$trusted_public_key" "trusted release public key"
mkdir "$staging_root/image"
(umask 000; cp -RP "$image_root/." "$staging_root/image/")
cp "$trusted_public_key" "$staging_root/trusted-public-key"
image_root=$staging_root/image
trusted_public_key=$staging_root/trusted-public-key
require_regular_file "$trusted_public_key"

require_root_owned_directory "$chariox_root"
require_root_owned_ancestor_chain "$chariox_root" "managed kernel upgrade authority"
require_root_owned_directory "$releases_root"
require_private_regular_file "$receipt_path" "managed bootstrap receipt"
require_safe_ancestor_chain "$receipt_path" "managed bootstrap receipt"

upgrade_lock=${CHARIOX_MANAGED_UPGRADE_LOCK:-/run/lock/chariox-managed-image-install.lock}
exec 9>"$upgrade_lock"
flock 9
recover_transaction

if [ ! -L "$current_link" ]; then
  echo "registered managed kernel current release link is missing" >&2
  exit 1
fi
if [ ! -L "$slice_build_context_link" ]; then
  echo "registered managed kernel slice build context facade is missing" >&2
  exit 1
fi
previous_slice_build_context=$(readlink "$slice_build_context_link")
verify_slice_build_context_facade "$previous_slice_build_context"
current_target=$(readlink "$current_link")
expected_current_target=releases/${expected_current_digest#sha256:}
if [ "$current_target" != "$expected_current_target" ]; then
  echo "installed release does not match the expected current release" >&2
  exit 1
fi
require_private_regular_file "$receipt_path" "managed bootstrap receipt"
node "$script_root/managed-kernel-upgrade-state.mjs" validate-receipt \
  "$receipt_path" "$expected_current_digest" "$release_override_path"
require_root_owned_directory "$releases_root/${expected_current_digest#sha256:}"
node "$script_root/verify-image-release.mjs" \
  "$releases_root/${expected_current_digest#sha256:}" "$expected_current_digest" "$trusted_public_key"
node "$script_root/verify-image-release.mjs" "$image_root" "$expected_new_digest" "$trusted_public_key"

current_protocol=$(protocol_version "$current_link/usr/local/bin/chariox-kernel")
target_protocol=$(protocol_version "$image_root/usr/local/bin/chariox-kernel")
node "$script_root/managed-kernel-upgrade-state.mjs" validate-protocol-transition \
  "$current_link" "$current_protocol" "$image_root" "$target_protocol"

release_name=${expected_new_digest#sha256:}
published_release=$releases_root/$release_name
if [ -e "$published_release" ] || [ -L "$published_release" ]; then
  require_directory "$published_release"
  require_root_owned_directory "$published_release"
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
  node "$script_root/managed-kernel-upgrade-state.mjs" sync-tree "$pending_release"
  mv "$pending_release" "$published_release"
  node "$script_root/managed-kernel-upgrade-state.mjs" sync-directory "$releases_root"
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
if [ -e "$release_override_path" ] || [ -L "$release_override_path" ]; then
  require_private_regular_file "$release_override_path" "managed release override"
  require_safe_ancestor_chain "$release_override_path" "managed release override"
  cp -P "$release_override_path" "$pending_transaction/previous-release-override.json"
  printf '%s\n' yes > "$pending_transaction/previous-release-override-present"
else
  printf '%s\n' no > "$pending_transaction/previous-release-override-present"
fi
node "$script_root/managed-kernel-upgrade-state.mjs" prepare-receipt \
  "$receipt_path" "$expected_current_digest" "$expected_new_digest" \
  "$pending_transaction/target-receipt.json" "$release_override_path" \
  "$pending_transaction/target-release-override.json"
printf '%s\n' "$current_target" > "$pending_transaction/previous-current"
printf '%s\n' "$expected_current_digest" > "$pending_transaction/previous-digest"
printf '%s\n' "$current_protocol" > "$pending_transaction/previous-protocol"
printf '%s\n' "$previous_slice_build_context" > "$pending_transaction/previous-slice-build-context"
printf '%s\n' "releases/$release_name" > "$pending_transaction/target-current"
printf '%s\n' "$expected_new_digest" > "$pending_transaction/target-digest"
printf '%s\n' "$target_protocol" > "$pending_transaction/target-protocol"
printf '%s\n' "$signed_slice_build_context_target" > "$pending_transaction/target-slice-build-context"
printf '%s\n' prepared > "$pending_transaction/phase"
chmod 0600 "$pending_transaction"/*
node "$script_root/managed-kernel-upgrade-state.mjs" sync-tree "$pending_transaction"
node "$script_root/managed-kernel-upgrade-state.mjs" publish-transaction \
  "$pending_transaction" "$transaction_root"
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
if ! atomic_receipt "$transaction_root/target-receipt.json"; then
  activation_failed=1
elif [ -f "$transaction_root/target-release-override.json" ]; then
  atomic_release_override "$transaction_root/target-release-override.json" || activation_failed=1
else
  remove_release_override || activation_failed=1
fi
if [ "${activation_failed:-0}" -eq 0 ]; then
  atomic_symlink "releases/$release_name" "$current_link" || activation_failed=1
fi
if [ "${activation_failed:-0}" -eq 0 ]; then
  atomic_symlink "$signed_slice_build_context_target" "$slice_build_context_link" \
    || activation_failed=1
fi
if [ "${activation_failed:-0}" -eq 0 ]; then
  verify_signed_slice_build_context_facade || activation_failed=1
fi
if [ "${activation_failed:-0}" -ne 0 ]; then
  if rollback_transaction; then
    echo "managed kernel activation failed; restored previous managed kernel release" >&2
  else
    echo "managed kernel activation failed; rollback remains pending" >&2
  fi
  exit 1
fi
write_phase activated
if ! systemctl daemon-reload \
  || ! health_not_before_ms=$(node -e 'process.stdout.write(String(Date.now()))') \
  || ! systemctl start "$service_name" \
  || ! check_health "$target_protocol" "$expected_new_digest" "$health_not_before_ms"; then
  if rollback_transaction; then
    echo "managed kernel health check failed; restored previous managed kernel release" >&2
  else
    echo "managed kernel health check failed; rollback remains pending" >&2
  fi
  exit 1
fi
if ! node "$script_root/managed-kernel-upgrade-state.mjs" validate-receipt-match \
  "$receipt_path" "$transaction_root/target-receipt.json" "$expected_new_digest" \
  "$release_override_path" "$transaction_root/target-release-override.json" \
  || ! write_phase committed; then
  if rollback_transaction; then
    echo "managed kernel final receipt validation failed; restored previous managed kernel release" >&2
  else
    echo "managed kernel final receipt validation failed; rollback remains pending" >&2
  fi
  exit 1
fi
tombstone_transaction
transaction_active=0
printf 'managed kernel upgraded to %s\n' "$expected_new_digest"
