import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { once } from "node:events"
import { unlink } from "node:fs/promises"
import { createServer } from "node:net"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { after, before, test } from "node:test"

import {
  LIVE_KERNEL_BINDING_SCHEMA,
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

function frame(value) {
  const bytes = Buffer.from(JSON.stringify(value), "utf8")
  const header = Buffer.allocUnsafe(4)
  header.writeUInt32BE(bytes.length, 0)
  return Buffer.concat([header, bytes])
}

let ipcServer

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
      socket.end(frame({
        response: {
          RelayStatus: {
            status: { daemon_id: KERNEL_ID, machine_id: MACHINE_ID },
          },
        },
        error: null,
      }))
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
