// Smoke-tests a compiled `chariox` release executable with neither Node nor Bun
// on its PATH, the way a release installation runs it:
//
//   node apps/cli/scripts/release-smoke.mjs EXECUTABLE VERSION
//
// MP-07 / MP-08 / MP-10 / MP-11: checks standalone CLI startup.
// Claude hooks are kernel-owned and tested in provider::claude::native_tui;
// they require Node on the provider execution machine.
import assert from "node:assert/strict"
import { execFile, execFileSync } from "node:child_process"
import { mkdtemp, realpath, rm } from "node:fs/promises"
import { createServer } from "node:http"
import { tmpdir } from "node:os"
import path from "node:path"
import { promisify } from "node:util"
import { WebSocketServer } from "ws"


const [executableArg, version] = process.argv.slice(2)
if (!executableArg || !version) throw new Error("usage: release-smoke.mjs EXECUTABLE VERSION")
const executable = await realpath(executableArg)
const scratch = await mkdtemp(path.join(await realpath(tmpdir()), "chariox-release-smoke-"))
// Only system tools; the release must not need Node or Bun.
const env = { PATH: scratch, HOME: scratch, CHARIOX_HOME: scratch, XDG_RUNTIME_DIR: scratch, LANG: "C.UTF-8" }
const run = (...args) => execFileSync(executable, args, { encoding: "utf8", env, timeout: 20_000 })

try {
  assert.equal(run("--version").trim(), `chariox ${version}`)
  assert.match(run("--help"), /^usage: /)
  await smokeAccessNoWait()

  process.stdout.write(`${JSON.stringify({ ok: true, executable, version })}\n`)
} finally {
  await rm(scratch, { recursive: true, force: true })
}

async function smokeAccessNoWait() {
  const socketPath = path.join(scratch, "kernel.sock")
  const server = createServer()
  const websocket = new WebSocketServer({ server })
  let received
  websocket.on("connection", (socket, request) => {
    assert.equal(request.url, "/kernel")
    assert.equal(request.headers.authorization, undefined)
    socket.once("message", bytes => {
      received = JSON.parse(String(bytes))
      socket.send(JSON.stringify({ type: "response", request_id: received.request_id, error: null,
        response: { KernelAccessStatus: { status: { state: "pending", holder_pid: 4242, holder_executable: "/usr/bin/codex" } } } }))
    })
  })
  await new Promise(resolve => server.listen(socketPath, resolve))
  try {
    const result = await promisify(execFile)(executable,
      ["access", "request", "--until-exit", "--no-wait", "--socket", socketPath],
      { encoding: "utf8", env, timeout: 15_000 }).catch(error => error)
    assert.equal(result.code, 6, result.stderr)
    assert.deepEqual(JSON.parse(result.stdout), { status: "pending", holder_pid: 4242,
      note: "approve in your Chariox client, then run chariox access status" })
    // The kernel owns the pending request and picks the holder: no pid, no waiter process.
    assert.deepEqual(received.request, { RequestKernelAccess: { term: "until_holder_exits", no_wait: true } })
  } finally {
    for (const socket of websocket.clients) socket.terminate()
    await new Promise(resolve => websocket.close(resolve))
    await new Promise(resolve => server.close(resolve))
  }
}
