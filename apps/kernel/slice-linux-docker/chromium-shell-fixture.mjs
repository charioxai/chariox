import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

export const screenSource = readFileSync(new URL("./docker/slice-screen.sh", import.meta.url), "utf8");
export const provisionSource = readFileSync(new URL("./provision-linux-docker-slice.sh", import.meta.url), "utf8");

// Load production function bodies, without running their script entrypoints or
// initialization. Every browser, display, Docker and process-control call below
// is a recording stub. Profile operations only touch this fixture's temp tree.
export function shellFunction(source, name) {
  const start = source.indexOf(`\n${name}() {\n`);
  assert.notEqual(start, -1, `missing ${name}`);
  const end = source.indexOf("\n}\n", start);
  assert.notEqual(end, -1, `unclosed ${name}`);
  return source.slice(start + 1, end + 3);
}

export function fixture() {
  const root = mkdtempSync(join(tmpdir(), "chariox-chromium-shell-"));
  const profile = join(root, "profile with spaces");
  const capture = join(root, "capture");
  for (const dir of [profile, capture, join(root, "logs"), join(profile, "Default")]) mkdirSync(dir, { recursive: true });
  return {
    root, profile, capture,
    run(source, environment = {}) {
      return spawnSync("bash", ["-c", `set -Eeuo pipefail\n${source}\nwait`], {
        cwd: root, encoding: "utf8", timeout: 3000, maxBuffer: 64 * 1024,
        env: { ...process.env, CHARIOX_SLICE_ROOT: root, CHARIOX_SLICE_CHROME_PROFILE: profile,
          FIXTURE_CAPTURE: capture, FIXTURE_UID: "1001", FIXTURE_DISPLAY: "1", FIXTURE_CHROMIUM: "0",
          FIXTURE_CDP_STATUS: "1", FIXTURE_LAUNCH_SURVIVES: "1", FIXTURE_READY_STATUS: "0", FIXTURE_RECREATE: "0", FIXTURE_UNCONFINED: "0",
          FIXTURE_INSPECT_FAILURE: "0", FIXTURE_IMAGE_ID: "fixture-image-id", ...environment },
      });
    },
    calls() {
      return Object.fromEntries(readdirSync(capture).map(name => [name, readFileSync(join(capture, name), "utf8").split("\0").slice(0, -1)]));
    },
    cleanup() { rmSync(root, { recursive: true, force: true }); },
  };
}

export function screenScript(operation) {
  const header = screenSource.slice(0, screenSource.indexOf("\nexport DISPLAY="));
  const definitions = screenSource.slice(screenSource.indexOf("\nlog() {"), screenSource.indexOf('\ncase "${1:-status}"'));
  return `${header}\n${definitions}
id() { printf '%s\\n' "$FIXTURE_UID"; }
process_running() {
  case "$1" in
    chromium*) [[ "$FIXTURE_CHROMIUM" == 1 || ( -f "$FIXTURE_CAPTURE/chromium" && "$FIXTURE_LAUNCH_SURVIVES" == 1 ) ]] ;;
    Xvfb*) [[ "$FIXTURE_DISPLAY" == 1 ]] ;;
    *) return 1 ;;
  esac
}
xdpyinfo() { [[ "$FIXTURE_DISPLAY" == 1 ]]; }
novnc_running() { return 1; }
sleep() { command sleep 0.001; }
curl() { printf '%s\\0' "$@" > "$FIXTURE_CAPTURE/readiness"; return "$FIXTURE_READY_STATUS"; }
nohup() { printf '%s\\0' "$@" > "$FIXTURE_CAPTURE/$1"; }
timeout() { printf '%s\\0' "$@" > "$FIXTURE_CAPTURE/cdp"; return "$FIXTURE_CDP_STATUS"; }
pkill() { printf '%s\\0' "$@" >> "$FIXTURE_CAPTURE/pkill"; }
rm() { printf '%s\\0' "$@" >> "$FIXTURE_CAPTURE/rm"; }
stop_desktop() { printf '%s\\0' stopped > "$FIXTURE_CAPTURE/stop"; FIXTURE_CHROMIUM=0; FIXTURE_DISPLAY=0; }
stop_process_pattern() { printf '%s\\0' "$@" >> "$FIXTURE_CAPTURE/stop-pattern"; }
wait_for_display() { FIXTURE_DISPLAY=1; }
require_process() { :; }
status() { printf '%s\\n' 'fixture status'; }
focus_chromium() { printf '%s\\0' focused > "$FIXTURE_CAPTURE/focus"; }
${operation}`;
}

export function provisionScript(operation) {
  const definitions = ["hash_stdin", "log", "fail", "migration_container_id", "ensure_container"].map(name => shellFunction(provisionSource, name)).join("\n");
  return `${definitions}
SCRIPT_DIR="$FIXTURE_SCRIPT_DIR"
SLICE_NAME=fixture
SLICE_IMAGE=fixture-image
SLICE_RECREATE="$FIXTURE_RECREATE"
SLICE_WORKSPACE_MOUNT_MODE=rw
SLICE_ALLOW_UNCONFINED_SECCOMP="$FIXTURE_UNCONFINED"
SLICE_HOME_VOLUME=fixture-home
SLICE_SAVED_HOME_ARCHIVE=deliberately-stale-archive
SLICE_CHROMIUM_MIGRATION_ID=""
SLICE_WORKSPACE_SOURCE=/fixture-workspace
SLICE_WORKSPACE=/workspace
SLICE_DEVELOPMENT_MOUNT_COUNT=0
SLICE_DOCKER_MEMORY=""
SLICE_DOCKER_CPUS=""
SLICE_CODEX_PORT=43252
SLICE_OPENCODE_PORT=43140
SLICE_CODEX_PORT_RANGE=43260-43279
SLICE_OPENCODE_PORT_RANGE=43150-43169
SLICE_KERNEL_PORT=43119
SLICE_RELAY_PORT=43130
SLICE_NOVNC_PORT=6080
container_exists() { [[ "$FIXTURE_EXISTS" == 1 ]]; }
container_running() { return 0; }
docker() {
  printf '%s\\0' "$@" >> "$FIXTURE_CAPTURE/docker"
  case "$1 $2" in
    'container inspect')
      if [[ "$*" == *io.chariox.chromium-migration* ]]; then
        printf '%s\\n' "$FIXTURE_MIGRATION_METADATA"
      elif [[ "$*" == *io.chariox.chromium-seccomp* ]]; then
        if [[ "$FIXTURE_INSPECT_FAILURE" == 1 ]]; then return 1; fi
        printf '%s\\n' "$FIXTURE_POLICY"
      else printf '%s\\n' fixture-image-id; fi ;;
    'image inspect') printf '%s\\n' "$FIXTURE_IMAGE_ID" ;;
    'rm -f') FIXTURE_EXISTS=0 ;;
    'create --name') printf '%s\\0' "$@" > "$FIXTURE_CAPTURE/create"; FIXTURE_EXISTS=1 ;;
    'volume create'|'exec -u') : ;;
    *) printf 'unexpected Docker call: %s\\n' "$*" >&2; return 1 ;;
  esac
}
run_with_timeout() { shift; "$@"; }
restore_saved_home_volume() { printf '%s\\0' restore > "$FIXTURE_CAPTURE/restore"; }
configure_stable_machine_identity() { :; }
configure_chromium_browser_policy() { :; }
configure_slice_state_directory() { :; }
refresh_slice_support_files() { :; }
refresh_saved_state_runtime() { :; }
${operation}`;
}
