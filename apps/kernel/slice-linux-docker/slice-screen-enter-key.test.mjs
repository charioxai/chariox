import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from "node:fs"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const helper = fileURLToPath(new URL("docker/slice-screen.sh", import.meta.url))
const linuxOnly = { skip: process.platform === "linux" ? false : "native X11 helper requires GNU timeout" }

function key(name) {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-enter-key-"))
  try {
    for (const [name, body] of Object.entries({
      xdpyinfo: "exit 0",
      pgrep: "echo live",
      timeout: 'printf "%s\\n" "$@" > "$CHARIOX_SLICE_ROOT/command"; cat > "$CHARIOX_SLICE_ROOT/input"',
    })) {
      writeFileSync(path.join(root, name), `#!/bin/sh\n${body}\n`, { mode: 0o755 })
    }
    execFileSync("bash", [helper, "computer-key-stdin", "1"], {
      env: { ...process.env, PATH: `${root}:${process.env.PATH}`, CHARIOX_SLICE_ROOT: root,
        CHARIOX_SLICE_CHROME_PROFILE: path.join(root, "chrome") },
      input: name, stdio: "pipe",
    })
    assert.deepEqual(readFileSync(path.join(root, "command"), "utf8").trim().split("\n"), [
      "--foreground", "10s", "/opt/chariox-selkies/bin/python",
      path.join(path.dirname(helper), "slice-keyboard.py"), "key-repeat", "1",
    ])
    return readFileSync(path.join(root, "input"), "utf8")
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
}

for (const name of ["Enter", "ctrl+Enter", "KP_Enter", "Return", "ctrl++", "Enter+Enter"]) {
  test(`Room key ${name} reaches the physical keyboard executor unchanged`, linuxOnly, () => {
    assert.equal(key(name), name)
  })
}
