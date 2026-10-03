import assert from "node:assert/strict"
import { test } from "node:test"
import net from "node:net"
import { mkdtemp, rm, unlink } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { guardedControlSessionRequest, sendGuardedLocalSocketRequest } from "./local-socket-session.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

const command = { KeepDisposableWorkerRunning: { homeKernelId: "home", allocationId: "one" } }
const admission = { session: { version: 1 }, error: null, response: { RelayStatus: { status: {
  daemon_id: "home", machine_id: "machine", capabilities: ["disposable_worker_control_v1"],
} } } }
function frame(value: unknown) {
  const body = Buffer.from(JSON.stringify(value)), result = Buffer.alloc(body.length + 4)
  result.writeUInt32BE(body.length); body.copy(result, 4); return result
}
function requests(socket: net.Socket, receive: (value: unknown) => void) {
  let buffered = Buffer.alloc(0)
  socket.on("error", () => {})
  socket.on("data", chunk => {
    buffered = Buffer.concat([buffered, chunk])
    while (buffered.length >= 4 && buffered.length >= buffered.readUInt32BE(0) + 4) {
      const size = buffered.readUInt32BE(0)
      const value = JSON.parse(buffered.subarray(4, 4 + size).toString())
      buffered = buffered.subarray(4 + size); receive(value)
    }
  })
}
async function fixture(run: (socketPath: string, server: net.Server) => Promise<void>, connection: (socket: net.Socket) => void) {
  const dir = await mkdtemp(path.join(os.tmpdir(), "cx-session-")), socketPath = path.join(dir, "s")
  const server = net.createServer(connection)
  await new Promise<void>(resolve => server.listen(socketPath, resolve))
  try { await run(socketPath, server) } finally {
    await new Promise<void>(resolve => server.close(() => resolve()))
    await rm(dir, { recursive: true, force: true })
  }
}
test("protocol 367 snapshots the explicit opt-in transport envelope", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 416)
  assert.equal(JSON.stringify(guardedControlSessionRequest), '{"GuardedControlSession":{"version":1}}')
})
test("fragmented probe and response execute one command on the admitted socket", async () => {
  const seen: unknown[] = []; let connections = 0
  await fixture(async socketPath => {
    assert.deepEqual(await sendGuardedLocalSocketRequest(socketPath, command, 1000), { Done: null })
    assert.equal(connections, 1)
    assert.deepEqual(seen, [guardedControlSessionRequest, command])
  }, socket => {
    connections++
    requests(socket, value => {
      seen.push(value)
      const bytes = frame(seen.length === 1 ? admission : { response: { Done: null }, error: null })
      socket.write(bytes.subarray(0, 2))
      setImmediate(() => socket.write(bytes.subarray(2)))
    })
  })
})
for (const reply of [{ error: "unknown request", response: null }, { ...admission, session: undefined },
  { ...admission, session: { version: 2 } }]) {
  test(`incompatible peer rejects without command: ${JSON.stringify(reply)}`, async () => {
    let count = 0
    await fixture(async socketPath => {
      await assert.rejects(sendGuardedLocalSocketRequest(socketPath, command, 1000), /kernel does not support guarded Unix control sessions/)
      assert.equal(count, 1)
    }, socket => requests(socket, () => { count++; socket.end(frame(reply)) }))
  })
}
test("closed admitted instance cannot replay onto a replacement at the same pathname", async () => {
  let freshCommands = 0
  let admittedPath = ""
  let replacement: net.Server | undefined
  await fixture(async socketPath => {
    admittedPath = socketPath
    await assert.rejects(sendGuardedLocalSocketRequest(socketPath, command, 1000))
    assert.equal(freshCommands, 0)
    if (replacement) await new Promise<void>(resolve => replacement!.close(() => resolve()))
  }, socket => requests(socket, async () => {
    const socketPath = admittedPath
    await unlink(socketPath)
    replacement = net.createServer(next => requests(next, () => { freshCommands++ }))
    await new Promise<void>(resolve => replacement!.listen(socketPath, resolve))
    socket.destroy()
  }))
})
test("connection closing after probe never reconnects or reports success", async () => {
  let connections = 0
  await fixture(async socketPath => {
    await assert.rejects(sendGuardedLocalSocketRequest(socketPath, command, 1000))
    assert.equal(connections, 1)
  }, socket => { connections++; requests(socket, () => socket.end(frame(admission))) })
})

test("missing capabilities reject before the command frame", async () => {
  let frames = 0
  await fixture(async socketPath => {
    await assert.rejects(sendGuardedLocalSocketRequest(socketPath, command, 1000), /does not support/)
    assert.equal(frames, 1)
  }, socket => requests(socket, () => {
    frames++
    socket.write(frame({ ...admission, response: { RelayStatus: { status: {
      daemon_id: "home", machine_id: "machine", capabilities: [],
    } } } }))
  }))
})
test("guarded mutation errors retain their operation-specific diagnostic", async () => {
  let frames = 0
  await fixture(async socketPath => {
    await assert.rejects(sendGuardedLocalSocketRequest(socketPath, command, 1000), /allocation does not exist/)
    assert.equal(frames, 2)
  }, socket => requests(socket, () => {
    socket.write(frame(++frames === 1 ? admission : { response: null, error: "allocation does not exist" }))
  }))
})
test("protocol 367 probe errors retain the kernel diagnostic without sending a command", async () => {
  let frames = 0
  await fixture(async socketPath => {
    await assert.rejects(sendGuardedLocalSocketRequest(socketPath, command, 1000), /kernel error: response exceeded frame limit/)
    assert.equal(frames, 1)
  }, socket => requests(socket, () => {
    frames++
    socket.end(frame({ response: null, error: "response exceeded frame limit" }))
  }))
})
test("whole exchange timeout closes a silent admitted socket", async () => {
  await fixture(async socketPath => {
    await assert.rejects(sendGuardedLocalSocketRequest(socketPath, command, 20), /timed out/)
  }, socket => requests(socket, () => {}))
})
test("oversized response header rejects before allocation of its declared body", async () => {
  await fixture(async socketPath => {
    await assert.rejects(sendGuardedLocalSocketRequest(socketPath, command, 1000), /frame size/)
  }, socket => requests(socket, () => {
    const header = Buffer.alloc(4); header.writeUInt32BE(1024 * 1024 + 1); socket.write(header)
  }))
})
