import assert from "node:assert/strict";

// Keep production helper options intact; the fixture adds its cleanup label
// and disables extra swap. Production already sets network, CPU, memory/PIDs.
export function restoreDockerShim(label) {
  assert.match(label, /^io\.chariox\.chromium-drill=[a-f0-9]{24}$/);
  return `#!/usr/bin/env bash
set -euo pipefail
if [[ "$1" == create ]]; then
  shift
  exec /usr/bin/docker create --memory-swap 512m --label '${label}' "$@"
fi
if [[ "$1" == volume && "$2" == create ]]; then
  shift 2
  exec /usr/bin/docker volume create --label '${label}' "$@"
fi
exec /usr/bin/docker "$@"
`;
}

function section(source, startMarker, endMarker) {
  const start = source.indexOf(startMarker);
  const end = source.indexOf(endMarker, start + startMarker.length);
  assert.ok(start >= 0 && end > start, `production restore section missing: ${startMarker}`);
  return source.slice(start, end);
}

// Test-only adapter to the same production initial-home functions used by
// restore-state. The minimal browser fixture has no worker kernel/toolchains,
// so it cannot execute the full restore-state image/runtime transaction.
export function restoreScript(provisioner) {
  assert.match(provisioner, /^prepare_home_volume\(\) \{$/m, "production prepare_home_volume function is missing");
  return [
    "set -Eeuo pipefail",
    'SLICE_NAME="$CHARIOX_SLICE_NAME"',
    'SLICE_HOME_VOLUME="$CHARIOX_SLICE_HOME_VOLUME"',
    'SLICE_SAVED_HOME_ARCHIVE="$CHARIOX_SLICE_SAVED_HOME_ARCHIVE"',
    'SLICE_IMAGE="$CHARIOX_SLICE_DOCKER_IMAGE"',
    // The production guarded command runner and digest live beside the
    // provisioner. Quotas stay off and the archive streams through stdin, as
    // for an unprotected restore without a broker-pinned archive directory.
    'SCRIPT_DIR="$CHARIOX_SLICE_SCRIPT_DIR"',
    'SLICE_SAVED_HOME_ARCHIVE_DIR=""',
    'SLICE_ID="chromium-drill"',
    'SLICE_OWNER_KERNEL_ID="chromium-drill"',
    'SLICE_OWNER_MACHINE_ID="chromium-drill"',
    'SLICE_DISK_LAYER_MB=""',
    'SLICE_DISK_HOME_MB=""',
    'log() { printf "[fixture-restore] %s\\n" "$*" >&2; }',
    'fail() { log "$*"; exit 1; }',
    section(provisioner, "hash_stdin() {", "runtime_source_revision() {"),
    section(provisioner, "disk_quota_enabled() {", "apply_layer_disk_quota() {"),
    section(provisioner, "run_guarded_command() {", "usage() {"),
    section(provisioner, "volume_inspect_reports_not_found() {", "machine_id_hex() {"),
    "prepare_home_volume",
  ].join("\n");
}
