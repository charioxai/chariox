import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { EventEmitter, once } from "node:events"
import { readFile as readRealFile, realpath as realRealpath, unlink } from "node:fs/promises"
import { createServer } from "node:net"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { after, before, test } from "node:test"

import {
  LIVE_KERNEL_BINDING_SCHEMA,
  inspectLinuxKernelProcess,
  readLocalKernelIdentity,
  startManagedOrdinaryLiveKernelBinding,
} from "./managed-ordinary-live-kernel-binding.mjs"

const PROCESS_ID = 21987
const SOCKET_INODE = "390187"
const SOCKET_PATH = join(tmpdir(), `chariox-live-kernel-binding-${process.pid}.sock`)
const KERNEL_ID = "kernel-current"
const MACHINE_ID = "machine-current"
const KERNEL_PATH = "/verified/release/usr/local/bin/chariox-kernel"
const KERNEL_BYTES = Buffer.from("verified current kernel bytes\n")
const KERNEL_DIGEST = `sha256:${createHash("sha256").update(KERNEL_BYTES).digest("hex")}`
const OLD_KERNEL_BYTES = Buffer.from("previous running kernel bytes\n")
const EXPECTED_BOUNDARY = "official-provider-turn"
const RELAY_STATUS_REQUEST_FRAME = Buffer.from([
  0x00, 0x00, 0x00, 0x14,
  ...Buffer.from('{"RelayStatus":null}', "utf8"),
])
const RELAY_STATUS_RESPONSE = {
  response: {
    RelayStatus: {
      status: { daemon_id: KERNEL_ID, machine_id: MACHINE_ID },
    },
  },
  error: null,
}

function frame(value) {
  const bytes = Buffer.from(JSON.stringify(value), "utf8")
  const header = Buffer.allocUnsafe(4)
  header.writeUInt32BE(bytes.length, 0)
  return Buffer.concat([header, bytes])
}

let ipcServer
const receivedRequests = []

class FakeSocket extends EventEmitter {
  constructor() {
    super()
    this.request = null
    this.destroyCalls = 0
    this.destroyed = false
  }

  end(bytes) {
    this.request = Buffer.from(bytes)
  }

  destroy() {
    this.destroyCalls += 1
    this.destroyed = true
    this.emit("close", false)
    return this
  }
}

function manualTimers() {
  let now = 0
  let nextId = 1
  let scheduled = 0
  let cleared = 0
  const pending = new Map()
  return {
    api: {
      setTimeout(callback, delayMs) {
        const id = nextId++
        scheduled += 1
        pending.set(id, { callback, deadline: now + delayMs })
        return id
      },
      clearTimeout(id) {
        cleared += 1
        pending.delete(id)
      },
    },
    advanceBy(delayMs) {
      now += delayMs
      for (const [id, timer] of [...pending]) {
        if (timer.deadline > now) continue
        pending.delete(id)
        timer.callback()
      }
    },
    get scheduled() { return scheduled },
    get cleared() { return cleared },
    get pendingCount() { return pending.size },
  }
}

function fakeIdentityRequest({ timeoutMs = 100 } = {}) {
  const socket = new FakeSocket()
  const timers = manualTimers()
  const result = readLocalKernelIdentity(SOCKET_PATH, {
    connect: () => socket,
    timeoutMs,
    timerApi: timers.api,
  })
  socket.emit("connect")
  return { socket, timers, result }
}

before(async () => {
  await unlink(SOCKET_PATH).catch((error) => {
    if (error?.code !== "ENOENT") throw error
  })
  ipcServer = createServer((socket) => {
    let request = Buffer.alloc(0)
    socket.on("data", (chunk) => {
      request = Buffer.concat([request, chunk])
      if (request.length < 4) return
      const length = request.readUInt32BE(0)
      if (request.length < length + 4) return
      const received = Buffer.from(request.subarray(0, length + 4))
      receivedRequests.push(received)
      if (request.length !== length + 4 || !received.equals(RELAY_STATUS_REQUEST_FRAME)) {
        socket.destroy()
        return
      }
      socket.end(frame(RELAY_STATUS_RESPONSE))
    })
  })
  ipcServer.listen(SOCKET_PATH)
  await once(ipcServer, "listening")
})

after(async () => {
  if (ipcServer?.listening) {
    await new Promise((resolve, reject) => ipcServer.close((error) => error ? reject(error) : resolve()))
  }
  await unlink(SOCKET_PATH).catch((error) => {
    if (error?.code !== "ENOENT") throw error
  })
})

function procStat(pid, startTime, state) {
  const fields = [state, "1", ...Array(17).fill("0"), String(startTime)]
  return `${pid} (chariox kernel test fixture) ${fields.join(" ")}\n`
}

function captureEnvironment(kernelId = KERNEL_ID, machineId = MACHINE_ID) {
  return {
    CHARIOX_DAEMON_SOCKET: SOCKET_PATH,
    CHARIOX_PARITY_CAPTURE_EVIDENCE_JSON: JSON.stringify({
      observed: true,
      boundary: EXPECTED_BOUNDARY,
      inside_provider_turn: true,
      independent: true,
      kernel_identity: {
        observed: true,
        kernel_id: kernelId,
        machine_id: machineId,
        transport: "local-unix-ipc",
      },
    }),
  }
}

function fakeFilesystem(overrides = {}) {
  const state = {
    startTime: 7712345,
    processState: "S",
    executableBytes: KERNEL_BYTES,
    executableLink: KERNEL_PATH,
    executableUnreadable: false,
    ...overrides,
  }
  return {
    state,
    async realpath(file) {
      if (file === KERNEL_PATH) return KERNEL_PATH
      throw Object.assign(new Error(`unavailable real path ${file}`), { code: "ENOENT" })
    },
    async readFile(file) {
      if (file === "/proc/net/unix") {
        return `Num RefCount Protocol Flags Type St Inode Path\n0000000000000000: 00000002 00000000 00010000 0001 01 ${SOCKET_INODE} ${SOCKET_PATH}\n`
      }
      if (file === "/proc/sys/kernel/random/boot_id") return "7a2b9ea4-cd3d-4bc2-9b36-46cba658217a\n"
      if (file === `/proc/${PROCESS_ID}/stat`) return procStat(PROCESS_ID, state.startTime, state.processState)
      if (file === `/proc/${PROCESS_ID}/exe`) {
        if (state.executableUnreadable) throw Object.assign(new Error("permission denied"), { code: "EACCES" })
        return state.executableBytes
      }
      throw Object.assign(new Error(`unavailable file ${file}`), { code: "ENOENT" })
    },
    async readdir(directory) {
      if (directory === "/proc") return [String(PROCESS_ID)]
      if (directory === `/proc/${PROCESS_ID}/fd`) return ["7"]
      throw Object.assign(new Error(`unavailable directory ${directory}`), { code: "ENOENT" })
    },
    async readlink(file) {
      if (file === `/proc/${PROCESS_ID}/exe`) return state.executableLink
      if (file === `/proc/${PROCESS_ID}/fd/7`) return `socket:[${SOCKET_INODE}]`
      throw Object.assign(new Error(`unavailable link ${file}`), { code: "ENOENT" })
    },
  }
}

function binding(filesystem, environment = captureEnvironment()) {
  return startManagedOrdinaryLiveKernelBinding({
    filesystem,
    processApi: { platform: "linux", env: environment },
    selectedKernelPath: KERNEL_PATH,
    expectedArtifactDigest: KERNEL_DIGEST,
    expectedBoundary: EXPECTED_BOUNDARY,
  })
}

test("current running kernel IPC identity and /proc executable bind to the verified artifact", async () => {
  const requestStart = receivedRequests.length
  const filesystem = fakeFilesystem()
  const liveBinding = await binding(filesystem)
  const evidence = await liveBinding.finish()

  assert.equal(evidence.schema, LIVE_KERNEL_BINDING_SCHEMA)
  assert.equal(evidence.observed, true)
  assert.deepEqual(evidence.capture_identity, {
    boundary: EXPECTED_BOUNDARY,
    kernel_id: KERNEL_ID,
    machine_id: MACHINE_ID,
    transport: "local-unix-ipc",
  })
  assert.deepEqual(evidence.observed_kernel, { kernel_id: KERNEL_ID, machine_id: MACHINE_ID })
  assert.equal(evidence.transport.kind, "local-unix-ipc")
  assert.equal(evidence.kernel_process.pid, PROCESS_ID)
  assert.equal(evidence.kernel_process.start_time_ticks, String(filesystem.state.startTime))
  assert.equal(evidence.kernel_process.executable_sha256, KERNEL_DIGEST)
  assert.deepEqual(evidence.verified_artifact, { path: KERNEL_PATH, sha256: KERNEL_DIGEST })
  assert.equal(evidence.stable_across_capture, true)
  assert.deepEqual(receivedRequests.slice(requestStart), [RELAY_STATUS_REQUEST_FRAME, RELAY_STATUS_REQUEST_FRAME])
})

test("local IPC request matches Rust's length-prefixed RelayStatus unit-struct request", async () => {
  const { socket, timers, result } = fakeIdentityRequest()
  socket.emit("data", frame(RELAY_STATUS_RESPONSE))
  socket.emit("end")

  assert.deepEqual(await result, { kernel_id: KERNEL_ID, machine_id: MACHINE_ID })
  assert.deepEqual(socket.request, RELAY_STATUS_REQUEST_FRAME)
  assert.deepEqual(RELAY_STATUS_REQUEST_FRAME.subarray(4), Buffer.from('{"RelayStatus":null}', "utf8"))
  assert.equal(timers.scheduled, 1)
  assert.equal(timers.cleared, 1)
  assert.equal(timers.pendingCount, 0)
  assert.equal(socket.destroyCalls, 1)
})

test("local IPC response accepts split header and body fragments", async () => {
  const { socket, timers, result } = fakeIdentityRequest()
  const response = frame(RELAY_STATUS_RESPONSE)
  socket.emit("data", response.subarray(0, 2))
  socket.emit("data", response.subarray(2, 4))
  socket.emit("data", response.subarray(4, 13))
  socket.emit("data", response.subarray(13))
  socket.emit("end")

  assert.deepEqual(await result, { kernel_id: KERNEL_ID, machine_id: MACHINE_ID })
  assert.deepEqual(socket.request, RELAY_STATUS_REQUEST_FRAME)
  assert.equal(timers.cleared, 1)
  assert.equal(socket.destroyCalls, 1)
})

test("truncated response at EOF rejects and cleans up once", async () => {
  const { socket, timers, result } = fakeIdentityRequest()
  const response = frame(RELAY_STATUS_RESPONSE)
  socket.emit("data", response.subarray(0, 4))
  socket.emit("data", response.subarray(4, 9))
  socket.emit("end")

  await assert.rejects(result, (error) => error.code === "kernel_ipc_response_truncated")
  assert.equal(timers.cleared, 1)
  assert.equal(socket.destroyCalls, 1)
})

test("abrupt IPC close rejects and cleans up once", async () => {
  const { socket, timers, result } = fakeIdentityRequest()
  socket.emit("data", Buffer.from([0, 0]))
  socket.emit("close", false)

  await assert.rejects(result, (error) => error.code === "kernel_ipc_response_closed")
  assert.equal(timers.cleared, 1)
  assert.equal(socket.destroyCalls, 1)
})

test("IPC socket error rejects and cleans up once", async () => {
  const { socket, timers, result } = fakeIdentityRequest()
  socket.emit("error", new Error("fixture transport error"))

  await assert.rejects(result, (error) => error.code === "kernel_ipc_unavailable")
  assert.equal(timers.cleared, 1)
  assert.equal(socket.destroyCalls, 1)
})

test("oversized IPC response frame rejects before reading its body", async () => {
  const { socket, timers, result } = fakeIdentityRequest()
  const header = Buffer.alloc(4)
  header.writeUInt32BE(1024 * 1024 + 1, 0)
  socket.emit("data", header)

  await assert.rejects(result, (error) => error.code === "kernel_ipc_frame_invalid")
  assert.equal(timers.cleared, 1)
  assert.equal(socket.destroyCalls, 1)
})

test("slow fragmented IPC response cannot extend the absolute deadline", async () => {
  const { socket, timers, result } = fakeIdentityRequest({ timeoutMs: 17 })
  const response = frame(RELAY_STATUS_RESPONSE)
  for (const byte of response) {
    if (socket.destroyed) break
    socket.emit("data", Buffer.from([byte]))
    timers.advanceBy(5)
  }

  await assert.rejects(result, (error) => error.code === "kernel_ipc_timeout")
  assert.deepEqual(socket.request, RELAY_STATUS_REQUEST_FRAME)
  assert.equal(timers.scheduled, 1)
  assert.equal(timers.cleared, 1)
  assert.equal(timers.pendingCount, 0)
  assert.equal(socket.destroyCalls, 1)
})

test("trailing response bytes are rejected before clean EOF", async () => {
  const { socket, timers, result } = fakeIdentityRequest()
  socket.emit("data", frame(RELAY_STATUS_RESPONSE))
  socket.emit("data", Buffer.from([0]))

  await assert.rejects(result, (error) => error.code === "kernel_ipc_frame_invalid")
  assert.equal(timers.cleared, 1)
  assert.equal(socket.destroyCalls, 1)
})

test("old running executable is rejected when the selected signed artifact is new", async () => {
  const filesystem = fakeFilesystem({ executableBytes: OLD_KERNEL_BYTES })
  await assert.rejects(binding(filesystem), (error) => error.code === "kernel_executable_digest_mismatch")
})

test("PID reuse during capture is rejected by the /proc start-time recheck", async () => {
  const filesystem = fakeFilesystem()
  const liveBinding = await binding(filesystem)
  filesystem.state.startTime += 1
  await assert.rejects(liveBinding.finish(), (error) => error.code === "kernel_process_changed")
})

test("an exited kernel process during capture is rejected", async () => {
  const filesystem = fakeFilesystem()
  const liveBinding = await binding(filesystem)
  filesystem.state.processState = "Z"
  await assert.rejects(liveBinding.finish(), (error) => error.code === "kernel_process_exited")
})

test("a deleted running executable is rejected even when its bytes remain readable", async () => {
  const filesystem = fakeFilesystem({ executableLink: `${KERNEL_PATH} (deleted)` })
  await assert.rejects(binding(filesystem), (error) => error.code === "kernel_executable_deleted")
})

test("an unreadable running executable is rejected", async () => {
  const filesystem = fakeFilesystem({ executableUnreadable: true })
  await assert.rejects(binding(filesystem), (error) => error.code === "kernel_process_unreadable")
})

test("a caller asserted kernel ID cannot replace the live IPC identity", async () => {
  const filesystem = fakeFilesystem()
  await assert.rejects(binding(filesystem, captureEnvironment("kernel-asserted", MACHINE_ID)), (error) => error.code === "kernel_capture_identity_mismatch")
})

test("Linux /proc socket owner and executable digest match the fixture process (unit conformance only)", {
  skip: process.platform !== "linux",
}, async () => {
  const socketPath = join(tmpdir(), `chariox-proc-owner-${process.pid}-${Date.now()}.sock`)
  const server = createServer()
  await unlink(socketPath).catch((error) => {
    if (error?.code !== "ENOENT") throw error
  })
  server.listen(socketPath)
  await once(server, "listening")
  try {
    const executablePath = await realRealpath(process.execPath)
    const executableBytes = await readRealFile(executablePath)
    const expectedDigest = `sha256:${createHash("sha256").update(executableBytes).digest("hex")}`
    const processIdentity = await inspectLinuxKernelProcess({
      socketPath,
      expectedExecutablePath: executablePath,
      expectedExecutableDigest: expectedDigest,
    })

    assert.equal(processIdentity.pid, process.pid)
    assert.equal(processIdentity.executable_path, executablePath)
    assert.equal(processIdentity.executable_sha256, expectedDigest)
    assert.match(processIdentity.linux_boot_id, /^[0-9a-f-]{36}$/i)
    assert.match(processIdentity.start_time_ticks, /^\d+$/)
  } finally {
    await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()))
    await unlink(socketPath).catch((error) => {
      if (error?.code !== "ENOENT") throw error
    })
  }
})


function productHarness() {
  const filesystem = fakeFilesystem()
  const originalReadFile = filesystem.readFile
  filesystem.readFile = async (path, ...args) => {
    if (path === "/proc/net/tcp") return `  0: 0100007F:${Number(43118).toString(16).toUpperCase()} 00000000:0000 0A 00000000:00000000 00:00000000 00000000 999 0 ${SOCKET_INODE}\n`
    if (path === "/proc/450/stat") return "450 (collector) S 201 " + [...Array(17).fill("0"), "123"].join(" ")
    if (path === "/proc/201/stat") return "201 (provider) S " + PROCESS_ID + " " + [...Array(17).fill("0"), "456"].join(" ")
    return originalReadFile(path, ...args)
  }
  const environment = captureEnvironment()
  delete environment.CHARIOX_DAEMON_SOCKET
  environment.CHARIOX_KERNEL_URL = "ws://127.0.0.1:43118"
  const capture = JSON.parse(environment.CHARIOX_PARITY_CAPTURE_EVIDENCE_JSON)
  capture.kernel_identity.transport = "kernel-public-api"
  environment.CHARIOX_PARITY_CAPTURE_EVIDENCE_JSON = JSON.stringify(capture)
  const native = { pid: PROCESS_ID, linux_boot_id: "7a2b9ea4-cd3d-4bc2-9b36-46cba658217a", start_time_ticks: String(filesystem.state.startTime) }
  const status = { daemon_id: KERNEL_ID, machine_id: MACHINE_ID, runtime_process_identity: native }
  let observations = 0
  const options = {
    filesystem,
    processApi: { platform: "linux", pid: 450, env: environment },
    selectedKernelPath: KERNEL_PATH,
    expectedArtifactDigest: KERNEL_DIGEST,
    expectedBoundary: EXPECTED_BOUNDARY,
    readKernelStatus: async () => { observations += 1; return status },
  }
  return { filesystem, environment, native, status, options, get observations() { return observations } }
}

test("MP-10 product identity binds the Linux boot, live PID and signed provider ancestor", async () => {
  const h = productHarness()
  const liveBinding = await startManagedOrdinaryLiveKernelBinding(h.options)
  const evidence = await liveBinding.finish()
  assert.equal(evidence.transport.kind, "kernel-public-api")
  assert.equal(evidence.kernel_process.pid, PROCESS_ID)
  assert.equal(evidence.kernel_process.executable_sha256, KERNEL_DIGEST)
  assert.equal(evidence.stable_across_capture, true)
  assert.equal(h.observations, 2)
})

test("MP-10 product route fails closed for legacy or foreign native identities", async t => {
  for (const [name, change, code] of [
    ["legacy status", h => { delete h.status.runtime_process_identity }, "kernel_native_process_identity_missing"],
    ["foreign boot", h => { h.native.linux_boot_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee" }, "kernel_native_process_identity_mismatch"],
    ["PID reuse", h => { h.native.start_time_ticks = "8888888" }, "kernel_native_process_identity_mismatch"],
    ["foreign kernel", h => { h.status.daemon_id = "kernel-foreign" }, "kernel_capture_identity_mismatch"],
    ["outside provider chain", h => { h.options.processApi.pid = PROCESS_ID }, "kernel_process_ancestry_mismatch"],
    ["old executable", h => { h.filesystem.state.executableBytes = OLD_KERNEL_BYTES }, "kernel_executable_digest_mismatch"],
  ]) await t.test(name, async () => {
    const h = productHarness(); change(h)
    await assert.rejects(startManagedOrdinaryLiveKernelBinding(h.options), error => error.code === code)
  })
})

test("MP-10 authenticated relay still requires independent local signed-process observation", async () => {
  const h = productHarness()
  h.environment.CHARIOX_KERNEL_URL = "wss://relay.example.test/runtime"
  h.environment.CHARIOX_PARITY_PROJECT_SETUP_RELAY_TOKEN = "fixture-relay-secret"
  const capture = JSON.parse(h.environment.CHARIOX_PARITY_CAPTURE_EVIDENCE_JSON)
  capture.kernel_identity.transport = "relay"
  h.environment.CHARIOX_PARITY_CAPTURE_EVIDENCE_JSON = JSON.stringify(capture)
  const liveBinding = await startManagedOrdinaryLiveKernelBinding(h.options)
  const proof = await liveBinding.finish()
  assert.equal(proof.transport.kind, "relay")
  assert.equal(JSON.stringify(proof).includes("fixture-relay-secret"), false)
  h.native.linux_boot_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
  await assert.rejects(startManagedOrdinaryLiveKernelBinding(h.options), error => error.code === "kernel_native_process_identity_mismatch")
})

test("MP-10 rejects native PID changes after the initial product observation", async () => {
  const h = productHarness()
  const liveBinding = await startManagedOrdinaryLiveKernelBinding(h.options)
  h.filesystem.state.startTime += 1
  h.native.start_time_ticks = String(h.filesystem.state.startTime)
  await assert.rejects(liveBinding.finish(), error => error.code === "kernel_process_changed")
})


test("MP-10 rejects a spoofed local product status from a different TCP listener owner", async () => {
  const h = productHarness()
  const readlink = h.filesystem.readlink
  h.filesystem.readlink = async path => path.endsWith("/fd/7") ? "socket:[foreign-inode]" : readlink(path)
  await assert.rejects(startManagedOrdinaryLiveKernelBinding(h.options), error => error.code === "kernel_socket_owner_mismatch")
})
