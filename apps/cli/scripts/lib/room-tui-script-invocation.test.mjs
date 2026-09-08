import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { promisify } from "node:util"
import test from "node:test"

import { roomTuiScriptInvocation } from "./room-tui-script-invocation.mjs"

test("uses the existing BSD script syntax outside Linux", () => {
  assert.deepEqual(roomTuiScriptInvocation("bun", ["cli.js", "--session", "room 1"], "darwin"), {
    command: "script",
    args: ["-q", "/dev/null", "bun", "cli.js", "--session", "room 1"],
  })
})

test("uses util-linux script command syntax with shell-safe arguments", () => {
  assert.deepEqual(roomTuiScriptInvocation("bun", ["cli.js", "room's $HOME", "$(false)"], "linux"), {
    command: "script",
    args: ["-q", "-e", "-c", "'bun' 'cli.js' 'room'\\''s $HOME' '$(false)'", "/dev/null"],
  })
  assert.throws(() => roomTuiScriptInvocation("bun", ["bad\0arg"], "linux"), /without NUL bytes/)
})

test("util-linux invocation runs argv literally in a real pseudo-terminal", { skip: process.platform !== "linux" }, async () => {
  const marker = "room's $HOME $(false)"
  const invocation = roomTuiScriptInvocation(process.execPath, ["-e", "process.stdout.write(process.argv[1])", marker])
  const { stdout, stderr } = await promisify(execFile)(invocation.command, invocation.args, { timeout: 5000 })
  assert.equal(stdout.replaceAll("\r", "").trim(), marker)
  assert.equal(stderr, "")
})
