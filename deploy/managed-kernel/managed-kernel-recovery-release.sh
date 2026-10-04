#!/bin/sh
# MP-07/MP-11: recovery must verify the selected release again before execution
# or terminal settlement. Journaled/package keys are correlation, not authority.

verify_recovery_release() {
  recovery_journal=$1
  recovery_role=$2
  case "$recovery_role" in
    previous)
      recovery_release_key=$trusted_public_key
      recovery_builder_key=${trusted_builder_public_key:-}
      ;;
    target)
      recovery_release_key=$next_trusted_public_key
      recovery_builder_key=${next_trusted_builder_public_key:-}
      ;;
    *) return 1 ;;
  esac
  recovery_digest=$(read_single_line "$recovery_journal/$recovery_role-digest") || return 1
  validate_digest "$recovery_digest" || return 1
  recovery_release=$releases_root/${recovery_digest#sha256:}
  require_root_owned_directory "$recovery_release"
  require_root_owned_ancestor_chain "$recovery_release" "recovery release"
  verify_selected_release "$recovery_release" "$recovery_digest" \
    "$recovery_release_key" "$recovery_builder_key"
}
