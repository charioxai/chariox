import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from "node:fs"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const helper = fileURLToPath(new URL("docker/slice-screen.sh", import.meta.url))
const linuxOnly = { skip: process.platform === "linux" ? false : "native X11 helper requires GNU timeout" }

function click(button, count) {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-pointer-click-"))
  try {
    for (const [name, body] of Object.entries({
      xdpyinfo: "exit 0",
      pgrep: "echo live",
      xdotool: 'printf "%s\\n" "$@" > "$CHARIOX_SLICE_ROOT/input"',
    })) {
      writeFileSync(path.join(root, name), `#!/bin/sh\n${body}\n`, { mode: 0o755 })
    }
    execFileSync("bash", [helper, "pointer-click", "120", "240", button, String(count)], {
      env: { ...process.env, PATH: `${root}:${process.env.PATH}`, CHARIOX_SLICE_ROOT: root,
        CHARIOX_SLICE_CHROME_PROFILE: path.join(root, "chrome") },
      stdio: "pipe",
    })
    return readFileSync(path.join(root, "input"), "utf8").trim().split("\n")
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
}

test("single pointer click dispatches both coordinates and button without repeat pacing", linuxOnly, () => {
  for (const [name, button] of [["left", "1"], ["middle", "2"], ["right", "3"]]) {
    assert.deepEqual(click(name, 1), ["mousemove", "120", "240", "click", "--repeat", "1", "--delay", "0", button])
  }
})

test("double pointer click keeps the 80ms interval", linuxOnly, () => {
  assert.deepEqual(click("left", 2), ["mousemove", "120", "240", "click", "--repeat", "2", "--delay", "80", "1"])
})

test("invalid pointer clicks fail before dispatch", linuxOnly, () => {
  assert.throws(() => click("other", 1), /pointer button must/)
  assert.throws(() => click("left", 3), /pointer click count must/)
})
