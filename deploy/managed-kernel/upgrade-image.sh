#!/bin/sh
set -eu

if [ "$(id -u)" -ne 0 ]; then
  echo "upgrade-image.sh must run as root" >&2
  exit 1
fi
apps_rollback_override=
if [ "${1:-}" = --allow-apps-rollback ]; then
  apps_rollback_override=--allow-apps-rollback
  shift
fi
if [ "$#" -ne 4 ] && [ "$#" -ne 5 ]; then
  echo "usage: CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1|shared_host upgrade-image.sh [--allow-apps-rollback] <managed-kernel-rootfs> <expected-current-release-digest> <expected-new-release-digest> <current-trusted-public-key> [next-trusted-public-key]" >&2
  exit 1
fi

image_root=$1
expected_current_digest=$2
expected_new_digest=$3
trusted_public_key=$4
next_trusted_public_key=${5:-$4}
# MP-07: restart recovery must not depend on a deleted download or image.
recover_only=${CHARIOX_MANAGED_UPGRADE_RECOVER_ONLY:-0}
case "$recover_only" in 0|1) ;; *) echo "invalid managed upgrade recovery mode" >&2; exit 1 ;; esac
install_root=${CHARIOX_MANAGED_UPGRADE_ROOT:-}
state_root=$install_root/var/lib/chariox
managed_home=$install_root/home/chariox
managed_state=$managed_home/.chariox
legacy_home=$state_root/home
script_root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
. "$script_root/managed-kernel-builder-pin-transaction.sh"
. "$script_root/managed-kernel-recovery-release.sh"
. "$script_root/managed-app-storage.sh"
managed_provider_topology=${CHARIOX_MANAGED_PROVIDER_TOPOLOGY-}
case "$managed_provider_topology" in
  path1|shared_host) ;;
  '')
    echo "CHARIOX_MANAGED_PROVIDER_TOPOLOGY must be explicitly set to path1 or shared_host" >&2
    exit 1
    ;;
  *)
    echo "CHARIOX_MANAGED_PROVIDER_TOPOLOGY must be path1 or shared_host" >&2
    exit 1
    ;;
esac
if [ "$managed_provider_topology" = path1 ]; then
  service_name=chariox-path1-managed-bootstrap.service
else
  service_name=chariox-managed-bootstrap.service
fi
chariox_root=$install_root/usr/lib/chariox
releases_root=$chariox_root/releases
current_link=$chariox_root/current
slice_build_context_link=$chariox_root/slice-build-context
signed_slice_build_context_target=current/usr/lib/chariox/slice-build-context
default_managed_receipt=$install_root/var/lib/chariox/managed/bootstrap-receipt.json
default_worker_receipt=$install_root/var/lib/chariox/disposable-worker/bootstrap-receipt.json
receipt_path=${CHARIOX_MANAGED_UPGRADE_RECEIPT:-$default_managed_receipt}
receipt_path_explicit=0
if [ -n "${CHARIOX_MANAGED_UPGRADE_RECEIPT:-}" ]; then
  receipt_path_explicit=1
fi
transaction_root=$chariox_root/.managed-kernel-upgrade
terminal_transaction=$chariox_root/.managed-kernel-upgrade.terminal
update_result_path=$chariox_root/.managed-kernel-upgrade-result
managed_release_update_id=${CHARIOX_MANAGED_RELEASE_UPDATE_ID:-}
health_host=${CHARIOX_MANAGED_UPGRADE_HEALTH_HOST:-127.0.0.1}
health_port=${CHARIOX_MANAGED_UPGRADE_HEALTH_PORT:-43118}
health_timeout_ms=${CHARIOX_MANAGED_UPGRADE_HEALTH_TIMEOUT_MS:-120000}
# Both topologies run with CHARIOX_HOME=$managed_state; a legacy home is migrated before any start.
presence_root=$managed_state/kernels/active
staging_root=$(mktemp -d "${TMPDIR:-/tmp}/chariox-managed-upgrade.XXXXXX")
chmod 0700 "$staging_root"
pending_release=
pending_transaction=
transaction_active=0
rolling_back=0
chariox_uid=$(id -u chariox)
chariox_gid=$(id -g chariox)

cleanup() {
  if [ -n "$pending_release" ] && [ -d "$pending_release" ]; then
    rm -rf -- "$pending_release"
  fi
  if [ -n "$pending_transaction" ] && [ -d "$pending_transaction" ]; then
    rm -rf -- "$pending_transaction"
  fi
  rm -rf -- "$staging_root"
}

# MP-07/MP-10: a helper may exit under set -e, bypassing its caller's
# explicit failure branch. Recover while staged trust inputs still exist.
finish() {
  finish_status=$?
  trap - EXIT
  if [ "$finish_status" -ne 0 ] && [ "$transaction_active" -eq 1 ] \
    && [ "$rolling_back" -eq 0 ]; then
    # Isolate helpers that use exit on an invalid authority. Preserve the
    # durable journal on failed recovery; never turn a failure into success.
    record_diagnostic_phase update_unexpected_exit
    if ! (recover_transaction); then
      echo "managed kernel exit recovery remains pending" >&2
    fi
  fi
  cleanup
  exit "$finish_status"
}

path_exists() {
  [ -e "$1" ] || [ -L "$1" ]
}

select_receipt_path() {
  if [ "$receipt_path_explicit" -eq 1 ]; then
    receipt_path=$CHARIOX_MANAGED_UPGRADE_RECEIPT
  else
    selected_receipt=
    for candidate in \
      "$default_managed_receipt" \
      "$default_worker_receipt" \
      "$legacy_home/managed/bootstrap-receipt.json" \
      "$legacy_home/disposable-worker/bootstrap-receipt.json" \
      "$legacy_home/.chariox/managed/bootstrap-receipt.json" \
      "$legacy_home/.chariox/disposable-worker/bootstrap-receipt.json" \
      "$managed_home/managed/bootstrap-receipt.json" \
      "$managed_home/disposable-worker/bootstrap-receipt.json" \
      "$managed_home/.chariox/managed/bootstrap-receipt.json" \
      "$managed_home/.chariox/disposable-worker/bootstrap-receipt.json"
    do
      if path_exists "$candidate"; then
        if [ -n "$selected_receipt" ]; then
          echo "both managed and allocation worker receipts exist; select the receipt explicitly" >&2
          exit 1
        fi
        selected_receipt=$candidate
      fi
    done
    receipt_path=${selected_receipt:-$default_managed_receipt}
  fi
  release_override_path=${CHARIOX_MANAGED_UPGRADE_RELEASE_OVERRIDE:-${receipt_path%/*}/release-override.json}
  grant_binding_path=${receipt_path%/*}/bootstrap-grant-binding.json
}

select_supervisor_service() {
  service_name=$(node "$script_root/managed-kernel-upgrade-state.mjs" \
    supervisor-service "$receipt_path" "$release_override_path") || return 1
  if [ "$managed_provider_topology" = path1 ] \
    && [ "$service_name" = chariox-managed-bootstrap.service ]; then
    service_name=chariox-path1-managed-bootstrap.service
  fi
}

assert_path1_service_overrides() {
  [ "$managed_provider_topology" = path1 ] || return 0
  path1_preflight_failure=0
  path1_drop_in_failure=0
  for unit in chariox-path1-managed-bootstrap.service chariox-disposable-worker-bootstrap.service; do
    need_daemon_reload=$(systemctl show --property=NeedDaemonReload --value "$unit") || {
      echo "could not inspect systemd reload state for Path-1 service $unit" >&2
      path1_preflight_failure=1
      path1_drop_in_failure=1
      continue
    }
    case "$need_daemon_reload" in
      no) ;;
      yes)
        echo "Path-1 service $unit needs systemd daemon-reload; refusing upgrade before service mutation" >&2
        path1_preflight_failure=1
        path1_drop_in_failure=1
        ;;
      *)
        echo "could not verify systemd reload state for Path-1 service $unit" >&2
        path1_preflight_failure=1
        path1_drop_in_failure=1
        ;;
    esac
    drop_in_paths=$(systemctl show --property=DropInPaths --value "$unit") || {
      echo "could not inspect effective systemd drop-ins for $unit" >&2
      path1_preflight_failure=1
      path1_drop_in_failure=1
      continue
    }
    # MP-07/MP-10/MP-11: admit only the exact root-owned observation setting
    # installed by the signed campaign installer; reject every other override.
    if [ -n "$drop_in_paths" ] && ! node "$script_root/path1-campaign-diagnostics-policy.mjs" \
      "$install_root" "$unit" "$drop_in_paths"; then
      path1_drop_in_failure=1
      path1_preflight_failure=1
      echo "Path-1 service $unit has systemd drop-ins: $drop_in_paths" >&2
    fi
  done
  [ "$path1_preflight_failure" -eq 0 ]
}

verify_selected_release() {
  selected_builder_public_key=${4:-${trusted_builder_public_key:-}}
  if [ "$managed_provider_topology" = path1 ]; then
    node "$script_root/verify-image-release.mjs" "$1" "$2" "$3" path1 "$selected_builder_public_key" || return 1
  elif [ "$service_name" = chariox-disposable-worker-bootstrap.service ]; then
    node "$script_root/verify-image-release.mjs" "$1" "$2" "$3" || return 1
  else
    node "$script_root/verify-image-release.mjs" "$1" "$2" "$3" "$managed_provider_topology" || return 1
  fi
  node "$script_root/managed-kernel-upgrade-state.mjs" verify-immutable-release-tree "$1" 0
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

# Schema 2 remains verifiable for current-release checks and legacy rollback.
require_cloud_update_target() {
  [ -n "$managed_release_update_id" ] || return 0
  node --input-type=module - "$1/usr/lib/chariox/release-manifest.json" <<'NODE'
import { readFile } from "node:fs/promises"
const manifest = JSON.parse(await readFile(process.argv[2], "utf8"))
if (manifest.schemaVersion !== 3 || manifest.managedUpdateEvidenceVersion !== 1) {
  console.error("Cloud managed release update target requires signed update evidence capability 1 (manifest schema 3)")
  process.exit(1)
}
NODE
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

# Disable while current still resolves the unit; systemd cannot disable a
# dangling unit after the pre-Apps release has replaced current.
prepare_managed_app_release_switch() {
  [ ! -f "$1/usr/libexec/chariox-app-storage" ] || return 0
  app_unit_link=$install_root/etc/systemd/system/chariox-app-storage.service
  if path_exists "$app_unit_link"; then
    [ -L "$app_unit_link" ] && [ "$(readlink "$app_unit_link")" = "../../../usr/lib/chariox/current/etc/systemd/system/chariox-app-storage.service" ] || {
      echo "managed App storage release link is obstructed" >&2; return 1;
    }
    # A dangling own link means a prior rollback already disabled the helper
    # before switching current; recovery only needs to remove that link.
    if [ -e "$app_unit_link" ]; then
      systemctl disable --now chariox-app-storage.service || return 1
    fi
  fi
}

sync_managed_app_storage() {
  app_package_link=$install_root/usr/local/bin/chariox-app-package
  app_helper_link=$install_root/usr/libexec/chariox-app-storage
  app_unit_link=$install_root/etc/systemd/system/chariox-app-storage.service
  if [ -f "$current_link/usr/local/bin/chariox-app-package" ]; then
    enroll_managed_app_storage "$install_root" "$managed_provider_topology" || return 1
    install -d -o root -g root -m 0755 "$install_root/usr/libexec" "$install_root/usr/local/bin" "$install_root/etc/systemd/system" || return 1
    publish_managed_app_link "../../../usr/lib/chariox/current/usr/local/bin/chariox-app-package" "$app_package_link" || return 1
    publish_managed_app_link "../lib/chariox/current/usr/libexec/chariox-app-storage" "$app_helper_link" || return 1
    publish_managed_app_link "../../../usr/lib/chariox/current/etc/systemd/system/chariox-app-storage.service" "$app_unit_link" || return 1
  else
    # Retain enrollment and App data for a later Apps upgrade, but remove only
    # our own release links. Never replace/remove an unrelated host install.
    for app_path in "$app_package_link" "$app_helper_link" "$app_unit_link"; do
      if path_exists "$app_path"; then
        case "$app_path" in
          "$app_package_link") app_target="../../../usr/lib/chariox/current/usr/local/bin/chariox-app-package" ;;
          "$app_helper_link") app_target="../lib/chariox/current/usr/libexec/chariox-app-storage" ;;
          "$app_unit_link") app_target="../../../usr/lib/chariox/current/etc/systemd/system/chariox-app-storage.service" ;;
        esac
        [ -L "$app_path" ] && [ "$(readlink "$app_path")" = "$app_target" ] || {
          echo "managed App storage release link is obstructed" >&2; return 1;
        }
      fi
    done
    rm -f -- "$app_package_link" "$app_helper_link" "$app_unit_link" || return 1
    if [ -f "$install_root/etc/chariox/app-storage.json" ]; then
      echo "Pre-Apps release: App storage is disabled; enrollment and App data are preserved for a later upgrade." >&2
    fi
  fi
}

start_managed_app_storage() {
  [ -f "$current_link/usr/libexec/chariox-app-storage" ] || return 0
  systemctl enable chariox-app-storage.service || return 1
  systemctl restart chariox-app-storage.service || return 1
  systemctl is-active --quiet chariox-app-storage.service || return 1
}

sync_path1_data_volume_unit_links() {
  [ "$managed_provider_topology" = path1 ] || return 0
  data_service=$install_root/etc/systemd/system/chariox-data-volume-admission.service
  rootless_dropin=$install_root/etc/systemd/system/chariox-rootless-docker.service.d/50-chariox-data-volume.conf
  allocator_dropin=$install_root/etc/systemd/system/chariox-slice-disk-quota-allocator.service.d/50-chariox-data-volume.conf
  if [ -f "$current_link/etc/systemd/system/chariox-data-volume-admission.service" ] \
    && [ -f "$current_link/etc/systemd/system/chariox-rootless-docker.service.d/50-chariox-data-volume.conf" ] \
    && [ -f "$current_link/etc/systemd/system/chariox-slice-disk-quota-allocator.service.d/50-chariox-data-volume.conf" ]; then
    install -d -o root -g root -m 0755 \
      "$(dirname "$rootless_dropin")" \
      "$(dirname "$allocator_dropin")"
    atomic_symlink "../../../usr/lib/chariox/current/etc/systemd/system/chariox-data-volume-admission.service" "$data_service" \
      || return 1
    atomic_symlink "../../../../usr/lib/chariox/current/etc/systemd/system/chariox-rootless-docker.service.d/50-chariox-data-volume.conf" "$rootless_dropin" \
      || return 1
    atomic_symlink "../../../../usr/lib/chariox/current/etc/systemd/system/chariox-slice-disk-quota-allocator.service.d/50-chariox-data-volume.conf" "$allocator_dropin" \
      || return 1
    return 0
  fi
  echo "Path-1 release is missing required data-volume admission artifacts" >&2
  return 1
}

stop_path1_runtime_services() {
  [ "$managed_provider_topology" = path1 ] || return 0
  path1_docker_uid=$(id -u chariox-docker) || return 1
  case "$path1_docker_uid" in ''|0|*[!0-9]*) return 1 ;; esac
  systemctl stop chariox-rootless-docker.service || return 1
  systemctl stop "user@$path1_docker_uid.service" || return 1
  systemctl stop chariox-slice-disk-quota-allocator.service || return 1
  systemctl stop chariox-data-volume-admission.service || return 1
}

start_path1_runtime_services() {
  [ "$managed_provider_topology" = path1 ] || return 0
  path1_docker_uid=$(id -u chariox-docker) || return 1
  case "$path1_docker_uid" in ''|0|*[!0-9]*) return 1 ;; esac
  # Force inactive units even when a caller reaches this helper with Docker
  # already running, then run the non-persistent admission check before either
  # service can start. Their Requires dependency repeats the check per start.
  stop_path1_runtime_services || return 1
  systemctl start chariox-data-volume-admission.service || return 1
  systemctl start chariox-slice-disk-quota-allocator.service || return 1
  systemctl start chariox-rootless-docker.service || return 1
  systemctl is-active --quiet chariox-slice-disk-quota-allocator.service || return 1
  systemctl is-active --quiet chariox-rootless-docker.service || return 1
  systemctl is-active --quiet "user@$path1_docker_uid.service" || return 1
  mountpoint --quiet /var/lib/chariox-docker/data || return 1
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

# MP-07/MP-10/MP-11: observation failure never changes release settlement.
record_diagnostic_phase() {
  if [ -n "${CHARIOX_RUNTIME_DIAGNOSTICS_DIR:-}" ]; then
    python3 "$script_root/runtime-diagnostics.py" event \
      --directory "$CHARIOX_RUNTIME_DIAGNOSTICS_DIR" --event "$1" >/dev/null 2>&1 || :
  fi
}

write_phase() {
  node "$script_root/managed-kernel-upgrade-state.mjs" atomic-text "$1" "$transaction_root/phase" || return $?
  record_diagnostic_phase "$1"
}

plan_home_migration() {
  node "$script_root/managed-kernel-home-migration.mjs" plan \
    "$pending_transaction/home-migration.json" \
    "$state_root" "$legacy_home" "$managed_home" "$managed_state" \
    "$chariox_uid" "$chariox_gid"
}

resume_home_migration() {
  if [ ! -f "$transaction_root/home-migration.json" ]; then
    if path_exists "$legacy_home"; then
      echo "managed kernel upgrade transaction is missing its required home migration journal" >&2
      return 1
    fi
    return 0
  fi
  node "$script_root/managed-kernel-home-migration.mjs" apply \
    "$transaction_root/home-migration.json" \
    "$state_root" "$legacy_home" "$managed_home" "$managed_state" \
    "$chariox_uid" "$chariox_gid" || return 1
  previous_service_name=$service_name
  select_receipt_path
  require_private_regular_file "$receipt_path" "managed bootstrap receipt"
  require_safe_ancestor_chain "$receipt_path" "managed bootstrap receipt"
  select_supervisor_service || return 1
  if [ "$service_name" != "$previous_service_name" ]; then
    echo "managed kernel service identity changed during home migration" >&2
    return 1
  fi
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

# Keep public terminal evidence after the private recovery journal is removed.
# The journal owns the update identity, including during restart recovery.
publish_update_result() {
  result_journal=$1
  [ -f "$result_journal/update-result-identity" ] || return 0
  node --input-type=module - "$result_journal" "$update_result_path" <<'NODE'
import { open, readFile, rename, unlink } from "node:fs/promises"
import { dirname } from "node:path"
const [journal, destination] = process.argv.slice(2)
const identity = await readFile(`${journal}/update-result-identity`, "utf8")
const fields = identity.trimEnd().split("\n")
const phase = (await readFile(`${journal}/phase`, "utf8")).trimEnd()
if (fields.length !== 7 || fields[0] !== "1"
  || !/^managed_release_update_[a-f0-9-]{36}$/.test(fields[1])
  || fields.slice(2, 4).some((value) => !/^sha256:[a-f0-9]{64}$/.test(value))
  || fields.slice(4).some((value) => !/^[a-z0-9][a-z0-9._:-]{0,127}$/.test(value))
  || !["committed", "rolled_back"].includes(phase)) {
  throw new Error("managed release update result identity is invalid")
}
const temporary = `${destination}.new`
await unlink(temporary).catch((error) => { if (error.code !== "ENOENT") throw error })
const handle = await open(temporary, "wx", 0o644)
try {
  await handle.chmod(0o644)
  await handle.writeFile(`${fields.join("\n")}\n${phase}\n`)
  await handle.sync()
} finally {
  await handle.close()
}
await rename(temporary, destination)
const parent = await open(dirname(destination), "r")
try { await parent.sync() } finally { await parent.close() }
NODE
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
  if [ "$(read_single_line "$transaction_root/phase")" = committed ]; then
    echo "refusing to roll back a committed managed release update" >&2
    return 1
  fi
  rolling_back=1
  validate_builder_pin_journal "$transaction_root" || return 1
  verify_recovery_release "$transaction_root" previous || return 1
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
  if [ "$managed_provider_topology" = path1 ]; then
    for previous_data_volume_artifact in \
      etc/systemd/system/chariox-data-volume-admission.service \
      etc/systemd/system/chariox-rootless-docker.service.d/50-chariox-data-volume.conf \
      etc/systemd/system/chariox-slice-disk-quota-allocator.service.d/50-chariox-data-volume.conf; do
      if [ ! -f "$chariox_root/$previous_target/$previous_data_volume_artifact" ]; then
        echo "refusing to roll Path-1 back to a release without data-volume admission" >&2
        return 1
      fi
    done
  fi
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
  if ! systemctl stop "$service_name" || ! stop_path1_runtime_services; then
    echo "managed kernel rollback could not stop the kernel or Path-1 storage services" >&2
    return 1
  fi
  activate_builder_pin "$transaction_root" previous || return 1
  resume_home_migration || return 1
  atomic_receipt "$transaction_root/previous-receipt.json" || return 1
  # The new supervisor may have rebound the grant to a schema the previous one rejects.
  if [ -f "$transaction_root/previous-grant-binding.json" ]; then
    node "$script_root/managed-kernel-upgrade-state.mjs" atomic-sidecar \
      "$transaction_root/previous-grant-binding.json" "$grant_binding_path" "$receipt_path" || return 1
  fi
  previous_override_present=$(read_single_line "$transaction_root/previous-release-override-present") || return 1
  case "$previous_override_present" in
    yes) atomic_release_override "$transaction_root/previous-release-override.json" || return 1 ;;
    no) remove_release_override || return 1 ;;
    *) echo "managed kernel upgrade transaction has an invalid release override marker" >&2; return 1 ;;
  esac
  prepare_managed_app_release_switch "$chariox_root/$previous_target" || return 1
  atomic_symlink "$previous_target" "$current_link" || return 1
  sync_path1_data_volume_unit_links || return 1
  sync_managed_app_storage || return 1
  atomic_symlink "$previous_slice_build_context" "$slice_build_context_link" || return 1
  verify_slice_build_context_facade "$previous_slice_build_context" || return 1
  validate_active_builder_pin "$transaction_root" previous "$previous_target" || return 1
  systemctl daemon-reload || return 1
  assert_path1_service_overrides || return 1
  start_path1_runtime_services || return 1
  start_managed_app_storage || return 1
  health_not_before_ms=$(node -e 'process.stdout.write(String(Date.now()))') || return 1
  systemctl start "$service_name" || return 1
  active_previous_protocol=$(protocol_version "$current_link/usr/local/bin/chariox-kernel") || return 1
  [ "$active_previous_protocol" = "$previous_protocol" ] || return 1
  check_health "$active_previous_protocol" "$previous_digest" "$health_not_before_ms" || return 1
  node "$script_root/managed-kernel-upgrade-state.mjs" validate-receipt-match \
    "$receipt_path" "$transaction_root/previous-receipt.json" "$previous_digest" \
    "$release_override_path" "$transaction_root/previous-release-override.json" || return 1
  write_phase rolled_back || return 1
  publish_update_result "$transaction_root" || return 1
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
  case "$terminal_phase" in
    committed)
      verify_recovery_release "$terminal_transaction" target || return 1
      validate_active_builder_pin "$terminal_transaction" target "$terminal_current" || return 1
      ;;
    rolled_back)
      verify_recovery_release "$terminal_transaction" previous || return 1
      validate_active_builder_pin "$terminal_transaction" previous "$terminal_current" || return 1
      ;;
  esac
  node "$script_root/managed-kernel-upgrade-state.mjs" validate-receipt-match \
    "$receipt_path" "$terminal_receipt" "$terminal_digest" \
    "$release_override_path" "$terminal_override" || return 1
  publish_update_result "$terminal_transaction" || return 1
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
    verify_recovery_release "$transaction_root" target || return 1
    validate_active_builder_pin "$transaction_root" target "$target_current" || return 1
    node "$script_root/managed-kernel-upgrade-state.mjs" validate-receipt-match \
      "$receipt_path" "$transaction_root/target-receipt.json" "$target_digest" \
      "$release_override_path" "$transaction_root/target-release-override.json" || return 1
    publish_update_result "$transaction_root" || return 1
    tombstone_transaction || return 1
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
    verify_recovery_release "$transaction_root" previous || return 1
    validate_active_builder_pin "$transaction_root" previous "$previous_current" || return 1
    node "$script_root/managed-kernel-upgrade-state.mjs" validate-receipt-match \
      "$receipt_path" "$transaction_root/previous-receipt.json" "$previous_digest" \
      "$release_override_path" "$transaction_root/previous-release-override.json" || return 1
    publish_update_result "$transaction_root" || return 1
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
    # A committed journal admits the target permanently. Recovery can still
    # publish its result and remove the journal after this process exits.
    if [ "$(read_single_line "$transaction_root/phase")" != committed ]; then
      rollback_transaction
    fi
  fi
  cleanup
  case "$$" in ''|0|1|*[!0-9]*) exit 1 ;; esac
  kill -s "$signal" "$$"
  exit 1
}

trap finish EXIT
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
if [ "$managed_provider_topology" = path1 ]; then
  trusted_builder_public_key=${CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY:-}
  next_trusted_builder_public_key=${CHARIOX_NEXT_TRUSTED_BUILDER_PUBLIC_KEY:-$trusted_builder_public_key}
  trusted_builder_runtime_key=$install_root/etc/chariox/trusted-builder-public-key
  if [ -z "$trusted_builder_public_key" ]; then
    echo "Path-1 upgrade requires CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY outside the image" >&2
    exit 1
  fi
fi
if [ "$recover_only" -eq 0 ] && { [ -L "$image_root" ] || [ ! -d "$image_root" ]; }; then
  echo "managed kernel image root must be a directory, not a symlink" >&2
  exit 1
fi
if [ "$managed_provider_topology" = path1 ]; then
  image_canonical=
  if [ "$recover_only" -eq 0 ]; then image_canonical=$(realpath "$image_root"); fi
  for builder_input in "$trusted_builder_public_key" "$next_trusted_builder_public_key"; do
    require_root_owned_private_regular_file "$builder_input" "trusted builder public key"
    require_root_owned_ancestor_chain "$builder_input" "trusted builder public key"
    builder_key_canonical=$(realpath "$builder_input")
    case "$recover_only:$builder_key_canonical" in
      "0:$image_canonical"|"0:$image_canonical"/*)
        echo "trusted builder public key must be supplied outside the image" >&2
        exit 1
        ;;
    esac
  done
fi
require_root_owned_private_regular_file "$trusted_public_key" "trusted release public key"
require_root_owned_ancestor_chain "$trusted_public_key" "trusted release public key"
require_root_owned_private_regular_file "$next_trusted_public_key" "next trusted release public key"
require_root_owned_ancestor_chain "$next_trusted_public_key" "next trusted release public key"
if [ "$recover_only" -eq 0 ]; then
  mkdir "$staging_root/image"
  (umask 000; cp -RP "$image_root/." "$staging_root/image/")
fi
cp "$trusted_public_key" "$staging_root/trusted-public-key"
cp "$next_trusted_public_key" "$staging_root/next-trusted-public-key"
if [ "$managed_provider_topology" = path1 ]; then
  cp "$trusted_builder_public_key" "$staging_root/trusted-builder-public-key"
  cp "$next_trusted_builder_public_key" "$staging_root/next-trusted-builder-public-key"
  trusted_builder_public_key=$staging_root/trusted-builder-public-key
  next_trusted_builder_public_key=$staging_root/next-trusted-builder-public-key
fi
image_root=$staging_root/image
trusted_public_key=$staging_root/trusted-public-key
next_trusted_public_key=$staging_root/next-trusted-public-key
require_regular_file "$trusted_public_key"
require_regular_file "$next_trusted_public_key"

upgrade_lock=${CHARIOX_MANAGED_UPGRADE_LOCK:-/run/lock/chariox-managed-image-install.lock}
exec 9>"$upgrade_lock"
flock 9
select_receipt_path
require_root_owned_directory "$chariox_root"
require_root_owned_ancestor_chain "$chariox_root" "managed kernel upgrade authority"
require_root_owned_directory "$releases_root"
if path_exists "$update_result_path"; then
  require_root_owned_private_regular_file "$update_result_path" "managed release update result"
fi
require_private_regular_file "$receipt_path" "managed bootstrap receipt"
require_safe_ancestor_chain "$receipt_path" "managed bootstrap receipt"
select_supervisor_service
assert_path1_service_overrides
if [ "$recover_only" -eq 1 ]; then
  for recovery_journal in "$transaction_root" "$terminal_transaction"; do
    if path_exists "$recovery_journal"; then
      require_root_owned_directory "$recovery_journal"
      node "$script_root/managed-kernel-upgrade-state.mjs" validate-update-recovery \
        "$recovery_journal" "$managed_release_update_id" "$expected_current_digest" "$expected_new_digest"
    fi
  done
fi
recover_transaction
# A replay settles the original journal and publishes its terminal evidence;
# it must not silently start the failed target again under the same update ID.
if [ "$recover_only" -eq 1 ]; then exit 0; fi
select_receipt_path
require_private_regular_file "$receipt_path" "managed bootstrap receipt"
require_safe_ancestor_chain "$receipt_path" "managed bootstrap receipt"
select_supervisor_service

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
verify_selected_release \
  "$releases_root/${expected_current_digest#sha256:}" "$expected_current_digest" "$trusted_public_key"
verify_selected_release "$image_root" "$expected_new_digest" "$next_trusted_public_key" "${next_trusted_builder_public_key:-}"
require_cloud_update_target "$image_root"
if [ "$managed_provider_topology" = path1 ]; then
  require_root_owned_directory "$install_root/etc"
  if path_exists "$install_root/etc/chariox"; then
    require_root_owned_directory "$install_root/etc/chariox"
  else
    install -d -o root -g root -m 0755 "$install_root/etc/chariox"
  fi
  require_root_owned_ancestor_chain "$trusted_builder_runtime_key" "trusted builder runtime key"
  if path_exists "$trusted_builder_runtime_key"; then
    require_root_owned_private_regular_file "$trusted_builder_runtime_key" "trusted builder runtime key"
    if ! cmp -s "$trusted_builder_public_key" "$trusted_builder_runtime_key"; then
      echo "installed trusted builder key differs from the independent input" >&2
      exit 1
    fi
  fi
  install -o root -g root -m 0644 "$trusted_builder_public_key" "$trusted_builder_runtime_key"
  node "$script_root/managed-kernel-upgrade-state.mjs" sync-tree "$install_root/etc/chariox"
fi

current_protocol=$(protocol_version "$current_link/usr/local/bin/chariox-kernel")
target_protocol=$(protocol_version "$image_root/usr/local/bin/chariox-kernel")
node "$script_root/managed-kernel-upgrade-state.mjs" validate-protocol-transition \
  "$current_link" "$current_protocol" "$image_root" "$target_protocol"
if [ "$current_protocol" -ge 410 ] && [ "$target_protocol" -lt 410 ]; then
  apps_state=$(python3 "$script_root/apps-rollback-state.py" "$install_root")
  if [ "$apps_state" != absent ]; then
    if [ "$apps_rollback_override" != --allow-apps-rollback ]; then
      echo "rollback across the Apps boundary with App state is blocked; --allow-apps-rollback is required" >&2
      exit 1
    fi
    echo "WARNING: Apps rollback override enabled. The pre-Apps kernel cannot use App state; App state survival and later recovery are unproven. Preserve a backup before continuing." >&2
  fi
fi

release_name=${expected_new_digest#sha256:}
published_release=$releases_root/$release_name
if [ -e "$published_release" ] || [ -L "$published_release" ]; then
  require_directory "$published_release"
  require_root_owned_directory "$published_release"
  verify_selected_release "$published_release" "$expected_new_digest" "$next_trusted_public_key" "${next_trusted_builder_public_key:-}"
else
  pending_release=$(mktemp -d "$releases_root/.new-$release_name.XXXXXX")
  chmod 0755 "$pending_release"
  install -d -o root -g root -m 0755 \
    "$pending_release/usr/local/bin" \
    "$pending_release/usr/lib/chariox" \
    "$pending_release/etc/systemd/system" \
    "$pending_release/etc/systemd/system/chariox-rootless-docker.service.d" \
    "$pending_release/etc/systemd/system/chariox-slice-disk-quota-allocator.service.d"
  install -o root -g root -m 0755 "$image_root/usr/local/bin/chariox-kernel" "$pending_release/usr/local/bin/chariox-kernel"
  install -o root -g root -m 0755 "$image_root/usr/local/bin/chariox-managed-bootstrap" "$pending_release/usr/local/bin/chariox-managed-bootstrap"
  # The verified image carries the whole App set or, if built before Apps, none of it.
  if path_exists "$image_root/usr/local/bin/chariox-app-package"; then
    install -d -o root -g root -m 0755 "$pending_release/usr/libexec"
    install -o root -g root -m 0755 "$image_root/usr/local/bin/chariox-app-package" "$pending_release/usr/local/bin/chariox-app-package"
    install -o root -g root -m 0755 "$image_root/usr/libexec/chariox-app-storage" "$pending_release/usr/libexec/chariox-app-storage"
    install -o root -g root -m 0644 "$image_root/etc/systemd/system/chariox-app-storage.service" "$pending_release/etc/systemd/system/chariox-app-storage.service"
  fi
  for release_file in release-manifest.json release-manifest.sig release-public-key build-attestation.json build-attestation.sig builder-public-key; do
    install -o root -g root -m 0644 "$image_root/usr/lib/chariox/$release_file" "$pending_release/usr/lib/chariox/$release_file"
  done
  for unit in chariox-managed-bootstrap.service chariox-rootless-docker.service chariox-slice-broker.service; do
    install -o root -g root -m 0644 "$image_root/etc/systemd/system/$unit" "$pending_release/etc/systemd/system/$unit"
  done
  if [ "$managed_provider_topology" = path1 ]; then
    install -o root -g root -m 0644 "$image_root/etc/systemd/system/chariox-data-volume-admission.service" "$pending_release/etc/systemd/system/chariox-data-volume-admission.service"
    install -o root -g root -m 0644 "$image_root/etc/systemd/system/chariox-rootless-docker.service.d/50-chariox-data-volume.conf" "$pending_release/etc/systemd/system/chariox-rootless-docker.service.d/50-chariox-data-volume.conf"
    install -o root -g root -m 0644 "$image_root/etc/systemd/system/chariox-slice-disk-quota-allocator.service.d/50-chariox-data-volume.conf" "$pending_release/etc/systemd/system/chariox-slice-disk-quota-allocator.service.d/50-chariox-data-volume.conf"
  fi
  path1_unit=chariox-path1-managed-bootstrap.service
  if [ -f "$image_root/etc/systemd/system/$path1_unit" ]; then
    install -o root -g root -m 0644 "$image_root/etc/systemd/system/$path1_unit" "$pending_release/etc/systemd/system/$path1_unit"
  fi
  worker_unit=chariox-disposable-worker-bootstrap.service
  if [ -f "$image_root/etc/systemd/system/$worker_unit" ]; then
    install -o root -g root -m 0644 "$image_root/etc/systemd/system/$worker_unit" "$pending_release/etc/systemd/system/$worker_unit"
  fi
  (umask 000; cp -RP "$image_root/usr/lib/chariox/slice-build-context" "$pending_release/usr/lib/chariox/slice-build-context")
  verify_selected_release "$pending_release" "$expected_new_digest" "$next_trusted_public_key" "${next_trusted_builder_public_key:-}"
  node "$script_root/managed-kernel-upgrade-state.mjs" sync-tree "$pending_release"
  mv "$pending_release" "$published_release"
  node "$script_root/managed-kernel-upgrade-state.mjs" sync-directory "$releases_root"
  pending_release=
fi

# Provision before supervisor namespace setup, including upgrades from PrivateTmp hosts.
. "$script_root/docker-admission-install.sh"
install_docker_admission_artifacts "$published_release" "$install_root" || {
  echo "failed to provision host-wide Docker admission locks" >&2
  exit 1
}

pending_transaction=$chariox_root/.managed-kernel-upgrade.pending
if [ -e "$pending_transaction" ] || [ -L "$pending_transaction" ]; then
  if [ -L "$pending_transaction" ] || [ ! -d "$pending_transaction" ]; then
    echo "managed kernel upgrade pending transaction path is obstructed" >&2
    exit 1
  fi
  rm -rf -- "$pending_transaction"
fi
install -d -o root -g root -m 0700 "$pending_transaction"
journal_builder_pins "$pending_transaction"
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
if [ -e "$grant_binding_path" ] || [ -L "$grant_binding_path" ]; then
  require_private_regular_file "$grant_binding_path" "managed bootstrap grant binding"
  cp -P "$grant_binding_path" "$pending_transaction/previous-grant-binding.json"
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
if [ -n "$managed_release_update_id" ]; then
  node --input-type=module - "$managed_release_update_id" "$expected_current_digest" \
    "$expected_new_digest" "$pending_transaction/previous-receipt.json" \
    > "$pending_transaction/update-result-identity" <<'NODE'
import { readFile } from "node:fs/promises"
const [id, from, target, receiptPath] = process.argv.slice(2)
const receipt = JSON.parse(await readFile(receiptPath, "utf8"))
const identity = [receipt.environmentId, receipt.machineId, receipt.kernelId]
if (!/^managed_release_update_[a-f0-9-]{36}$/.test(id)
  || identity.some((value) => typeof value !== "string" || !/^[a-z0-9][a-z0-9._:-]{0,127}$/.test(value))) {
  throw new Error("managed release update identity is invalid")
}
process.stdout.write(["1", id, from, target, ...identity].join("\n") + "\n")
NODE
fi
plan_home_migration
chmod 0600 "$pending_transaction"/*
node "$script_root/managed-kernel-upgrade-state.mjs" sync-tree "$pending_transaction"
node "$script_root/managed-kernel-upgrade-state.mjs" publish-transaction \
  "$pending_transaction" "$transaction_root"
pending_transaction=
transaction_active=1
record_diagnostic_phase prepared

if ! systemctl stop "$service_name" || ! stop_path1_runtime_services; then
  if rollback_transaction; then
    echo "managed kernel or Path-1 storage services could not be stopped; restored previous managed kernel release" >&2
  else
    echo "managed kernel or Path-1 storage services could not be stopped; rollback remains pending" >&2
  fi
  exit 1
fi
write_phase stopped
record_diagnostic_phase activation_builder_pin_start
if ! activate_builder_pin "$transaction_root" target; then
  if rollback_transaction; then
    echo "managed builder pin activation failed; restored previous managed kernel release" >&2
  else
    echo "managed builder pin activation failed; rollback remains pending" >&2
  fi
  exit 1
fi
record_diagnostic_phase activation_home_migration_start
if ! resume_home_migration; then
  if rollback_transaction; then
    echo "managed kernel home migration failed; restored previous managed kernel release" >&2
  else
    echo "managed kernel home migration failed; rollback remains pending" >&2
  fi
  exit 1
fi
record_diagnostic_phase activation_receipt_start
if ! atomic_receipt "$transaction_root/target-receipt.json"; then
  activation_failed=1
elif [ -f "$transaction_root/target-release-override.json" ]; then
  record_diagnostic_phase activation_release_override_start
  atomic_release_override "$transaction_root/target-release-override.json" || activation_failed=1
else
  record_diagnostic_phase activation_release_override_start
  remove_release_override || activation_failed=1
fi
if [ "${activation_failed:-0}" -eq 0 ]; then
  record_diagnostic_phase activation_app_prepare_start
  prepare_managed_app_release_switch "$published_release" || activation_failed=1
  if [ "${activation_failed:-0}" -eq 0 ]; then
    record_diagnostic_phase activation_current_link_start
    atomic_symlink "releases/$release_name" "$current_link" || activation_failed=1
  fi
fi
if [ "${activation_failed:-0}" -eq 0 ]; then
  record_diagnostic_phase activation_data_volume_links_start
  sync_path1_data_volume_unit_links || activation_failed=1
fi
if [ "${activation_failed:-0}" -eq 0 ]; then
  record_diagnostic_phase activation_app_storage_start
  sync_managed_app_storage || activation_failed=1
fi
if [ "${activation_failed:-0}" -eq 0 ]; then
  record_diagnostic_phase activation_slice_facade_start
  atomic_symlink "$signed_slice_build_context_target" "$slice_build_context_link" \
    || activation_failed=1
fi
if [ "${activation_failed:-0}" -eq 0 ]; then
  record_diagnostic_phase activation_slice_facade_check_start
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
  || ! assert_path1_service_overrides \
  || ! start_path1_runtime_services \
  || ! start_managed_app_storage \
  || ! health_not_before_ms=$(node -e 'process.stdout.write(String(Date.now()))') \
  || ! systemctl start "$service_name" \
  || ! check_health "$target_protocol" "$expected_new_digest" "$health_not_before_ms"; then
  if [ "${path1_drop_in_failure:-0}" -eq 1 ]; then
    if rollback_transaction; then
      echo "Path-1 systemd drop-ins blocked activation; restored previous managed kernel release" >&2
    else
      echo "Path-1 systemd drop-ins blocked activation; rollback remains pending; verify the managed kernel service state before retry" >&2
    fi
  elif rollback_transaction; then
    echo "managed kernel health check failed; restored previous managed kernel release" >&2
  else
    echo "managed kernel health check failed; rollback remains pending" >&2
  fi
  exit 1
fi
if ! node "$script_root/managed-kernel-upgrade-state.mjs" validate-receipt-match \
  "$receipt_path" "$transaction_root/target-receipt.json" "$expected_new_digest" \
  "$release_override_path" "$transaction_root/target-release-override.json" \
  || ! validate_active_builder_pin "$transaction_root" target "releases/$release_name" \
  || ! write_phase committed; then
  if rollback_transaction; then
    echo "managed kernel final receipt validation failed; restored previous managed kernel release" >&2
  else
    echo "managed kernel final receipt validation failed; rollback remains pending" >&2
  fi
  exit 1
fi
publish_update_result "$transaction_root"
tombstone_transaction
transaction_active=0
printf 'managed kernel upgraded to %s\n' "$expected_new_digest"
