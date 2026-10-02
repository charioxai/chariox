import assert from "node:assert/strict";

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
    'log() { printf "[fixture-restore] %s\\n" "$*" >&2; }',
    'fail() { log "$*"; exit 1; }',
    section(provisioner, "hash_stdin() {", "runtime_source_revision() {"),
    section(provisioner, "run_with_timeout() {", "run_with_file_stdin_timeout() {"),
    section(provisioner, "volume_inspect_reports_not_found() {", "machine_id_hex() {"),
    "prepare_home_volume",
  ].join("\n");
}
