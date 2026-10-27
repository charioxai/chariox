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
import { mkdtemp, readFile, realpath, rm } from "node:fs/promises"
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
  await smokeAccessWaiter()

  process.stdout.write(`${JSON.stringify({ ok: true, executable, version })}\n`)
} finally {
  await rm(scratch, { recursive: true, force: true })
}

async function smokeAccessWaiter() {
  const socketPath = path.join(scratch, "kernel.sock")
  const server = createServer()
  const websocket = new WebSocketServer({ server })
  let waiterPid
  let received
  let reply
  let requestReceived
  const requested = new Promise(resolve => { requestReceived = resolve })
  const disconnected = new Promise(resolve => {
    websocket.on("connection", (socket, request) => {
      assert.equal(request.url, "/kernel")
      assert.equal(request.headers.authorization, undefined)
      socket.once("message", bytes => {
        received = JSON.parse(String(bytes))
        reply = () => socket.send(JSON.stringify({ type: "response", request_id: received.request_id,
          response: { KernelAccessGranted: { grant: { holder_pid: process.pid } } }, error: null }))
        requestReceived()
      })
      socket.once("close", resolve)
    })
  })
  await new Promise(resolve => server.listen(socketPath, resolve))
  try {
    const result = await promisify(execFile)(executable,
      ["access", "request", "--until-exit", "--no-wait", "--holder-pid", String(process.pid), "--socket", socketPath],
      { encoding: "utf8", env, timeout: 15_000 }).catch(error => error)
    assert.equal(result.code, 6, result.stderr)
    assert.deepEqual(JSON.parse(result.stdout), { status: "pending", holder_pid: process.pid,
      note: "approve in your Chariox client, then run chariox access status" })
    waiterPid = Number(await readFile(path.join(scratch, `access-waiter-${process.pid}.pid`), "utf8"))
    assert.ok(Number.isSafeInteger(waiterPid) && waiterPid > 1)
    await within(requested, "access waiter did not send its request")
    assert.deepEqual(received.request, { RequestKernelAccess: { holder_pid: process.pid, term: "until_holder_exits" } })
    // The detached child must retain its connection after the launcher exits.
    process.kill(waiterPid, 0)
    reply()
    await within(disconnected, "access waiter did not finish after approval")
  } finally {
    if (Number.isSafeInteger(waiterPid) && waiterPid > 1) {
      try { process.kill(waiterPid, "SIGTERM") } catch (error) { if (error.code !== "ESRCH") throw error }
    }
    for (const socket of websocket.clients) socket.terminate()
    await new Promise(resolve => websocket.close(resolve))
    await new Promise(resolve => server.close(resolve))
  }
}

async function within(promise, message) {
  let timer
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), 5_000)
    })])
  } finally { clearTimeout(timer) }
}
