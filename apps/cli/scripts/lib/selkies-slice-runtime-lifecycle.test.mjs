import assert from "node:assert/strict"
import fs from "node:fs"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../../..")
const dockerfilePath = path.join(repoRoot, "apps/kernel/slice-linux-docker/docker/Dockerfile")
const screenPath = path.join(repoRoot, "apps/kernel/slice-linux-docker/docker/slice-screen.sh")
const dockerfile = fs.readFileSync(dockerfilePath, "utf8")
const screen = fs.readFileSync(screenPath, "utf8")

const selkiesVersion = "0.0.0.dev0"
const selkiesRevision = "3f87241fcd6abc44e205b22f6596e78ef4946670"
const selkiesSourceSha256 = "9aee9dcaf6e6617194b84a667fd7fe441b2cf85718abf63e109f29db18c90d51"

function section(source, startMarker, endMarker) {
  const start = source.indexOf(startMarker)
  const end = source.indexOf(endMarker, start + startMarker.length)
  assert.notEqual(start, -1, `missing section start: ${startMarker}`)
  assert.notEqual(end, -1, `missing section end: ${endMarker}`)
  return source.slice(start, end)
}

test("Selkies is source-pinned with verifiable license and native-wheel metadata", () => {
  for (const needle of [
    `ARG SELKIES_VERSION=${selkiesVersion}`,
    `ARG SELKIES_REVISION=${selkiesRevision}`,
    `ARG SELKIES_SOURCE_SHA256=${selkiesSourceSha256}`,
    "https://github.com/selkies-project/selkies/archive/$SELKIES_REVISION.tar.gz",
    "https://github.com/selkies-project/selkies/commit/${CHARIOX_SELKIES_REVISION}",
    "source_archive_sha256=$SELKIES_SOURCE_SHA256",
    "license=MPL-2.0",
    "license_url=https://mozilla.org/MPL/2.0/",
    "COPY --from=selkies-builder /out/selkies-LICENSE.txt",
    "/usr/share/doc/chariox-selkies/LICENSE",
    "/usr/share/doc/chariox-selkies/SOURCE-METADATA",
    'LABEL io.chariox.selkies-license="MPL-2.0"',
    'LABEL org.opencontainers.image.licenses="MPL-2.0"',
    "pixelflux_version=2.1.0",
    "pixelflux_source_revision=6974a8c16a14dc0040d5ed937d633a6ceb4c926d",
    "pixelflux_source_url=https://github.com/selkies-project/pixelflux/commit/6974a8c16a14dc0040d5ed937d633a6ceb4c926d",
    "pcmflux_version=2.1.0",
    "pcmflux_source_revision=a8fac8d8ea89117a1c2480bdbefbb3b8b717d956",
    "pcmflux_source_url=https://github.com/selkies-project/pcmflux/commit/a8fac8d8ea89117a1c2480bdbefbb3b8b717d956",
    "--only-binary=:all:",
    "--require-hashes",
    "pip check",
  ]) {
    assert.ok(dockerfile.includes(needle), `missing Dockerfile contract: ${needle}`)
  }

  for (const needle of [
    "https://github.com/selkies-project/pixelflux/releases/download/6974a8c/pixelflux-2.1.0-cp311-cp311-manylinux_2_28_x86_64.whl",
    "97a1b6b289373373b8a32699bade666d758848be11fda470f5e49a5d50512756",
    "https://github.com/selkies-project/pixelflux/releases/download/6974a8c/pixelflux-2.1.0-cp311-cp311-manylinux_2_28_aarch64.whl",
    "b86783c2fb7eb14b105f123b8182be7144afb9bd4d2c2af181b5731b5e966342",
    "https://github.com/selkies-project/pcmflux/releases/download/a8fac8d/pcmflux-2.1.0-cp311-cp311-manylinux_2_28_x86_64.whl",
    "5fc902b41888eaf8185f891709c995198152fd7febfd158d130cba18bd07508c",
    "https://github.com/selkies-project/pcmflux/releases/download/a8fac8d/pcmflux-2.1.0-cp311-cp311-manylinux_2_28_aarch64.whl",
    "d34fb03f25ad0a977bbcb19d8aee061b5125b8ee8be96be2a8398bd6097681ef",
  ]) {
    assert.ok(dockerfile.includes(needle), `missing native-wheel contract: ${needle}`)
  }

  assert.doesNotMatch(dockerfile, /selkies-project\/selkies\/(?:archive|commit)\/(?:main|master)/)
})

test("backend selection is explicit, fail-closed, and keeps novnc as production default", () => {
  assert.match(screen, /DISPLAY_BACKEND="\$\{CHARIOX_SLICE_DISPLAY_BACKEND:-novnc\}"/)
  assert.match(screen, /case "\$DISPLAY_BACKEND" in\s+novnc\|selkies\) ;;[\s\S]*unknown display backend[\s\S]*exit 2/)
  assert.doesNotMatch(screen, /CHARIOX_SLICE_DISPLAY_BACKEND:-selkies/, "Selkies must not become the default")
  assert.match(screen, /SELKIES_PORT="\$\{CHARIOX_SLICE_SELKIES_PORT:-\$NOVNC_PORT\}"/)
  assert.match(screen, /SELKIES_HEALTH_TIMEOUT[\s\S]*bounded range/)
  assert.match(screen, /CHARIOX_SLICE_DISPLAY_BACKEND.*novnc/)
})

test("Selkies start, health, and stop have ordered bounded lifecycle gates", () => {
  const start = section(screen, "selkies_start() {", "\nstop_process_pattern() {")
  const stop = section(screen, "selkies_stop_pid() {", "\nselkies_start() {")
  const health = section(screen, "selkies_health() {", "\nselkies_write_pid_state() {")

  assert.ok(start.indexOf("selkies_require_binary") < start.indexOf("selkies_port_is_free"))
  assert.ok(start.indexOf('nohup "$SELKIES_BIN"') < start.indexOf("selkies_write_pid_state"))
  const afterPidState = start.slice(start.indexOf("selkies_write_pid_state"))
  assert.ok(afterPidState.indexOf("selkies_process_owned") < afterPidState.indexOf("if selkies_health; then"))
  assert.match(start, /health_attempts=\$\(\(SELKIES_HEALTH_TIMEOUT \* 10\)\)/)
  assert.match(start, /for attempt in \$\(seq 1 "\$health_attempts"\)/)
  assert.match(start, /--addr=0\.0\.0\.0/)
  assert.doesNotMatch(start, /--addr=127\.0\.0\.1/)
  assert.match(start, /--port="\$SELKIES_PORT"/)
  assert.match(start, /--mode=websockets/)

  assert.match(health, /curl --fail --silent --show-error --max-time 1/)
  assert.match(health, /http:\/\/127\.0\.0\.1:\$SELKIES_PORT\/api\/health/)
  assert.match(health, /grep -Fxq 'OK'/)

  assert.ok(stop.indexOf("kill -TERM") < stop.indexOf("kill -KILL"))
  assert.match(stop, /for attempt in \$\(seq 1 50\)/)
  assert.match(stop, /for attempt in \$\(seq 1 20\)/)
  assert.match(stop, /selkies_stop\(\) \{[\s\S]*selkies_stop_pid/)
  assert.ok(stop.indexOf("selkies_process_owned") < stop.indexOf('rm -f "$SELKIES_PID_FILE"'))
  assert.match(stop, /if \[\[ -L "\$SELKIES_PID_FILE" \]\]/)
})

test("Selkies failures clean up before start success can be reported", () => {
  const start = section(screen, "selkies_start() {", "\nstop_process_pattern() {")
  const cleanup = section(screen, "cleanup_failed_start() {", "\ntrap cleanup_failed_start EXIT")

  assert.match(start, /Selkies exited before becoming healthy[\s\S]*selkies_stop \|\| true/)
  assert.match(start, /Selkies health check timed out[\s\S]*selkies_stop \|\| true/)
  assert.match(start, /unable to record Selkies PID state[\s\S]*selkies_stop_pid/)
  assert.match(screen, /START_IN_PROGRESS=1/)
  assert.match(screen, /START_IN_PROGRESS=0/)
  assert.match(cleanup, /START_IN_PROGRESS.*== 1/)
  assert.match(cleanup, /DISPLAY_BACKEND.*selkies/)
  assert.match(cleanup, /set \+e[\s\S]*stop_desktop/)
  assert.match(screen, /trap cleanup_failed_start EXIT/)
})

test("stale PID state, missing binaries, and port conflicts fail closed", () => {
  assert.match(screen, /\[\[ -f "\$SELKIES_PID_FILE" && ! -L "\$SELKIES_PID_FILE" \]\]/)
  assert.match(screen, /line_count.*== 2/)
  assert.match(screen, /SELKIES_START_TICKS.*\^\[0-9\]\+\$/)
  assert.match(screen, /\/proc\/\$pid\/cmdline/)
  assert.match(screen, /command_line.*SELKIES_BIN/)
  assert.match(screen, /\[\[ -x "\$SELKIES_BIN" \]\]/)
  assert.match(screen, /command -v curl/)
  assert.match(screen, /command -v lsof/)
  assert.match(screen, /lsof -nP -iTCP:"\$SELKIES_PORT" -sTCP:LISTEN/)
  assert.match(screen, /Selkies PID state is stale or malformed/)
  assert.match(screen, /Selkies port is already in use/)
  assert.match(screen, /Selkies binary is missing or not executable/)
})

test("novnc remains the unchanged explicit fallback and is mutually exclusive with Selkies", () => {
  const launch = section(screen, "  nohup openbox >", "  nohup chromium \\")
  const selkiesBranch = launch.indexOf('if [[ "$DISPLAY_BACKEND" == selkies ]]; then')
  const novncBranch = launch.indexOf("  else", selkiesBranch)
  assert.ok(selkiesBranch >= 0)
  assert.ok(novncBranch > selkiesBranch)

  const selkiesLaunch = launch.slice(selkiesBranch, novncBranch)
  const novncLaunch = launch.slice(novncBranch)
  assert.doesNotMatch(selkiesLaunch, /x11vnc|websockify/)
  assert.match(novncLaunch, /nohup x11vnc -display "\$DISPLAY_ID" -localhost -nopw -forever -shared -rfbport "\$VNC_PORT"/)
  assert.match(novncLaunch, /nohup websockify --web=\/usr\/share\/novnc\/ "0\.0\.0\.0:\$NOVNC_PORT" "127\.0\.0\.1:\$VNC_PORT"/)
  assert.match(dockerfile, /EXPOSE [^\n]*6080/)
  assert.match(screen, /viewer=http:\/\/127\.0\.0\.1:%s\/vnc\.html\?host=127\.0\.0\.1&port=%s&autoconnect=true&resize=scale/)
  assert.match(screen, /screen_missing_components\(\)[\s\S]*DISPLAY_BACKEND.*== selkies[\s\S]*x11vnc[\s\S]*novnc_running/)
})
