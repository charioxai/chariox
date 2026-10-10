#!/bin/sh
# Sourced by upgrade-image.sh. Only public verification pins are journaled.

journal_builder_pins() {
  [ "$managed_provider_topology" = path1 ] || return 0
  cp "$trusted_builder_public_key" "$1/previous-builder-public-key" || return 1
  cp "$next_trusted_builder_public_key" "$1/target-builder-public-key" || return 1
  chmod 0600 "$1/previous-builder-public-key" "$1/target-builder-public-key"
}

validate_builder_pin_journal() {
  [ "$managed_provider_topology" = path1 ] || return 0
  require_root_owned_directory "$1"
  require_root_owned_ancestor_chain "$1" "builder pin transaction"
  for pin_role in previous target; do
    if ! path_exists "$1/$pin_role-builder-public-key"; then
      echo "Path-1 transaction has no builder pin journal; refusing ambiguous legacy recovery" >&2
      return 1
    fi
    require_root_owned_private_regular_file "$1/$pin_role-builder-public-key" "journaled builder public key"
    pin_digest=$(read_single_line "$1/$pin_role-digest") || return 1
    validate_digest "$pin_digest" || return 1
    pin_release=$releases_root/${pin_digest#sha256:}/usr/lib/chariox/builder-public-key
    require_root_owned_ancestor_chain "$pin_release" "journaled builder release"
    require_root_owned_private_regular_file "$pin_release" "journaled packaged builder public key"
    if ! cmp -s "$1/$pin_role-builder-public-key" "$pin_release"; then
      echo "journaled builder pin does not match its immutable release" >&2
      return 1
    fi
  done
}

validate_builder_runtime_key() {
  require_root_owned_ancestor_chain "$trusted_builder_runtime_key" "trusted builder runtime key"
  require_root_owned_private_regular_file "$trusted_builder_runtime_key" "trusted builder runtime key"
}

# MP-07/MP-11: public checkpoints only. No paths, key bytes or command output.
builder_pin_diagnostic() {
  if command -v record_diagnostic_phase >/dev/null 2>&1; then
    record_diagnostic_phase "$1" || :
  fi
}

# Authority checks use exit on invalid input. Keep them in a subshell so the
# caller still reaches its explicit rollback branch, including during recovery.
activate_builder_pin() (
  [ "$managed_provider_topology" = path1 ] || return 0
  trap 'builder_pin_status=$?; if [ "$builder_pin_status" -ne 0 ]; then builder_pin_diagnostic builder_pin_failed; fi' EXIT
  builder_pin_diagnostic builder_pin_journal_start
  validate_builder_pin_journal "$1" || return 1
  builder_pin_diagnostic builder_pin_journal_returned
  case "$2" in previous|target) ;; *) return 1 ;; esac
  builder_pin_diagnostic builder_pin_runtime_start
  validate_builder_runtime_key || return 1
  builder_pin_diagnostic builder_pin_runtime_returned
  # Refuse to overwrite an unrelated authority introduced during the transaction.
  builder_pin_diagnostic builder_pin_compare_start
  if ! cmp -s "$trusted_builder_runtime_key" "$1/previous-builder-public-key" \
    && ! cmp -s "$trusted_builder_runtime_key" "$1/target-builder-public-key"; then
    echo "runtime builder pin does not belong to the upgrade transaction" >&2
    return 1
  fi
  builder_pin_diagnostic builder_pin_compare_returned
  builder_pin_diagnostic builder_pin_atomic_start
  node "$script_root/managed-kernel-upgrade-state.mjs" atomic-file \
    "$1/$2-builder-public-key" "$trusted_builder_runtime_key" || return 1
  builder_pin_diagnostic builder_pin_atomic_returned
)

validate_active_builder_pin() {
  [ "$managed_provider_topology" = path1 ] || return 0
  validate_builder_pin_journal "$1" || return 1
  case "$2" in previous|target) ;; *) return 1 ;; esac
  case "$3" in releases/*) ;; *) return 1 ;; esac
  validate_builder_runtime_key || return 1
  require_root_owned_private_regular_file \
    "$chariox_root/$3/usr/lib/chariox/builder-public-key" "active packaged builder public key"
  if ! cmp -s "$trusted_builder_runtime_key" "$1/$2-builder-public-key" \
    || ! cmp -s "$trusted_builder_runtime_key" "$chariox_root/$3/usr/lib/chariox/builder-public-key"; then
    echo "active builder pin does not match its release transaction" >&2
    return 1
  fi
}
