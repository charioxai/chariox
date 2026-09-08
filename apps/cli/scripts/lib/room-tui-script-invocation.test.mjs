import assert from "node:assert/strict"
import { spawn } from "node:child_process"
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

test("runner invocation preserves argv in a real pseudo-terminal with ignored stdin", { skip: !["linux", "darwin"].includes(process.platform) }, async () => {
  const marker = "room's $HOME $(false)"
  const invocation = roomTuiScriptInvocation(process.execPath, ["-e", "process.stdout.write(process.argv[1])", marker])
  const { stdout, stderr } = await new Promise((resolve, reject) => {
    const child = spawn(invocation.command, invocation.args, {
      timeout: 5000,
      stdio: ["ignore", "pipe", "pipe"],
    })
    let stdout = ""
    let stderr = ""
    child.stdout.on("data", chunk => { stdout += chunk })
    child.stderr.on("data", chunk => { stderr += chunk })
    child.once("error", reject)
    child.once("close", (code, signal) => {
      if (code !== 0) reject(new Error(`PTY exited ${code}/${signal}: ${stderr}`))
      else resolve({ stdout, stderr })
    })
  })
  // BSD script may echo its EOF keystroke when stdin is /dev/null.
  assert.equal(stdout.replace(/^\^D\x08\x08/, "").replaceAll("\r", "").trim(), marker)
  assert.equal(stderr, "")
})
