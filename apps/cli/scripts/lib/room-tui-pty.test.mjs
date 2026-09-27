import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import test from "node:test"
import { roomTuiPtyInvocation } from "./room-tui-pty.mjs"

test("Room TUI keeps BSD script argument passing on macOS", () => {
  assert.deepEqual(roomTuiPtyInvocation(["bun", "/path with spaces/cli.js", "--session", "one"], "darwin"), {
    command: "script", args: ["-q", "/dev/null", "bun", "/path with spaces/cli.js", "--session", "one"],
  })
})

test("Room TUI quotes Linux command arguments without shell expansion", () => {
  const values = ["path with spaces", "O'Brien", "$(printf unintended)", "$EXAMPLE", "", "a\nb"]
  const invocation = roomTuiPtyInvocation([process.execPath, "-e", "console.log(JSON.stringify(process.argv.slice(1)))", "--", ...values], "linux")
  assert.deepEqual(invocation.args.slice(0, 3), ["--quiet", "--return", "--command"])
  assert.equal(invocation.args.at(-1), "/dev/null")
  const child = spawnSync("sh", ["-c", invocation.args[3]], { encoding: "utf8", timeout: 5_000 })
  assert.equal(child.status, 0, child.stderr)
  assert.deepEqual(JSON.parse(child.stdout), values)
})

test("Room TUI launches an actual PTY on the current platform", () => {
  const invocation = roomTuiPtyInvocation([process.execPath, "-e", "console.log('ROOM_TUI_PTY_READY:' + Boolean(process.stdout.isTTY))"])
  const child = spawnSync(invocation.command, invocation.args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], timeout: 5_000 })
  assert.equal(child.status, 0, child.stderr)
  assert.match(child.stdout, /ROOM_TUI_PTY_READY:true/)
})

test("Room TUI rejects unsupported platforms and invalid command bytes", () => {
  assert.throws(() => roomTuiPtyInvocation(["bun"], "win32"), /unsupported/)
  assert.throws(() => roomTuiPtyInvocation(["bun", "bad\0arg"], "linux"), /valid TUI command/)
})
