import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync, lstatSync } from "node:fs"
import { tmpdir } from "node:os"
import path from "node:path"
import test from "node:test"

const screen = readFileSync(new URL("./docker/slice-screen.sh", import.meta.url), "utf8")
const audio = screen.slice(screen.indexOf("stop_desktop_audio() {"), screen.indexOf("chromium_has_restorable_session()"))
const runtime = "0123456789abcdef0123456789abcdef-runtime"

function fixture(run) {
  const root = mkdtempSync(path.join(tmpdir(), "chariox-screen-audio-"))
  try {
    const config = path.join(root, "config", "pulse")
    mkdirSync(config, { recursive: true })
    const target = path.join(root, "external-runtime")
    mkdirSync(target)
    writeFileSync(path.join(target, "socket-sentinel"), "preserve")
    symlinkSync(target, path.join(config, runtime))
    return run({ root, config, target })
  } finally { rmSync(root, { recursive: true, force: true }) }
}

function stop(root, running = false) {
  return spawnSync("bash", ["-c", `
set -euo pipefail
stop_process_pattern() { printf '%s\\n' stopped >> "$HOME/order"; }
process_running() { ${running ? "return 0" : "return 1"}; }
log() { :; }
${audio}
stop_desktop_audio
`], { env: { ...process.env, HOME: root, XDG_CONFIG_HOME: path.join(root, "config") }, encoding: "utf8" })
}

test("desktop shutdown removes PulseAudio runtime links without following them", () => fixture(({ root, config, target }) => {
  writeFileSync(path.join(config, "cookie"), "audio-settings")
  const directory = "ffffffffffffffffffffffffffffffff-runtime"
  mkdirSync(path.join(config, directory))
  symlinkSync(target, path.join(config, "other-runtime"))
  const result = stop(root)
  assert.equal(result.status, 0, result.stderr)
  assert.throws(() => lstatSync(path.join(config, runtime)), { code: "ENOENT" })
  assert.equal(readFileSync(path.join(target, "socket-sentinel"), "utf8"), "preserve")
  assert.equal(readFileSync(path.join(config, "cookie"), "utf8"), "audio-settings")
  assert.ok(lstatSync(path.join(config, directory)).isDirectory())
  assert.ok(lstatSync(path.join(config, "other-runtime")).isSymbolicLink())
  assert.equal(readFileSync(path.join(root, "order"), "utf8"), "stopped\n")
}))

test("a still-running audio writer prevents runtime-link cleanup", () => fixture(({ root, config }) => {
  assert.notEqual(stop(root, true).status, 0)
  assert.ok(lstatSync(path.join(config, runtime)).isSymbolicLink())
}))

test("desktop stop shuts down audio after Chromium and before the display", () => {
  const body = screen.slice(screen.indexOf("stop_desktop() {"), screen.indexOf("screenshot() {"))
  assert.ok(body.indexOf('stop_process_pattern "/usr/lib/chromium/chromium"') < body.indexOf("  stop_desktop_audio\n"))
  assert.ok(body.indexOf("  stop_desktop_audio\n") < body.indexOf('stop_process_pattern "Xvfb'))
})


test("the image gives all audio clients a non-home runtime path", () => {
  const dockerfile = readFileSync(new URL("./docker/Dockerfile", import.meta.url), "utf8")
  assert.match(dockerfile, /^ENV PULSE_RUNTIME_PATH=\/tmp\/chariox-pulse-runtime$/m)
})
