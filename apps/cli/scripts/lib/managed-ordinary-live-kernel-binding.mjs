import { inspectPrivilegedProcMetadata } from "./managed-ordinary-proc-metadata.mjs"
import { managedOrdinaryKernelConnection } from "./managed-ordinary-kernel-endpoint.mjs"
import { createHash } from "node:crypto"
import { readFile, readdir, readlink, realpath } from "node:fs/promises"
import net from "node:net"
import { isAbsolute, join, resolve } from "node:path"
import { tmpdir } from "node:os"

export const LIVE_KERNEL_BINDING_SCHEMA = "chariox.managed-ordinary-live-kernel-binding/v1"

const DIGEST = /^sha256:[0-9a-f]{64}$/i
const KERNEL_ID = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/
const BOOT_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
const PROC_ROOT = "/proc"
const MAX_IPC_FRAME_BYTES = 1024 * 1024
const IPC_TIMEOUT_MS = 10_000

const NODE_FILESYSTEM = Object.freeze({ readFile, readdir, readlink, realpath })

export class LiveKernelBindingError extends Error {
  constructor(code, message, options = {}) {
    super(message, options)
    this.name = "LiveKernelBindingError"
    this.code = code
  }
}

function bindingError(code, message, cause) {
  return new LiveKernelBindingError(code, message, cause ? { cause } : undefined)
}

function sha256(bytes) {
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`
}

function requiredIdentifier(value, label) {
  if (typeof value !== "string" || !KERNEL_ID.test(value)) {
    throw bindingError("kernel_capture_identity_missing", `capture evidence is missing a valid ${label}`)
  }
  return value
}

function parseCaptureIdentity(environment, expectedBoundary) {
  const raw = environment?.CHARIOX_PARITY_CAPTURE_EVIDENCE_JSON
  if (typeof raw !== "string" || raw.length === 0) {
    throw bindingError("kernel_capture_identity_missing", "capture evidence is missing the observed kernel identity")
  }

  let capture
  try {
    capture = JSON.parse(raw)
  } catch (error) {
    throw bindingError("kernel_capture_identity_invalid", "capture evidence is invalid JSON", error)
  }
  if (!capture || typeof capture !== "object" || Array.isArray(capture) || capture.observed !== true) {
    throw bindingError("kernel_capture_identity_unobserved", "capture evidence does not report an observed kernel identity")
  }
  if (capture.boundary !== expectedBoundary || capture.inside_provider_turn !== true || capture.independent !== true) {
    throw bindingError("kernel_capture_boundary_mismatch", "kernel identity evidence does not match the observed capture boundary")
  }

  const identity = capture.kernel_identity
  if (!identity || typeof identity !== "object" || Array.isArray(identity) || identity.observed !== true) {
    throw bindingError("kernel_capture_identity_unobserved", "capture evidence is missing observed kernel identity details")
  }
  if (!["local-unix-ipc", "kernel-public-api", "relay"].includes(identity.transport)) {
    throw bindingError("kernel_capture_transport_mismatch", "capture kernel identity was not observed over a supported product transport")
  }
  return {
    boundary: capture.boundary,
    kernel_id: requiredIdentifier(identity.kernel_id, "kernel ID"),
    machine_id: requiredIdentifier(identity.machine_id, "machine ID"),
    transport: identity.transport,
  }
}

function localSocketPath(environment, kernelId) {
  const configured = environment?.CHARIOX_DAEMON_SOCKET
  if (typeof configured === "string" && configured.trim()) {
    if (!isAbsolute(configured)) {
      throw bindingError("kernel_socket_path_invalid", "configured local daemon socket path must be absolute")
    }
    return resolve(configured)
  }

  let runtimeRoot
  if (typeof environment?.CHARIOX_HOME === "string" && environment.CHARIOX_HOME.trim()) {
    runtimeRoot = join(resolve(environment.CHARIOX_HOME), "run")
  } else if (typeof environment?.XDG_RUNTIME_DIR === "string" && environment.XDG_RUNTIME_DIR.trim()) {
    runtimeRoot = join(resolve(environment.XDG_RUNTIME_DIR), "chariox")
  } else if (typeof environment?.HOME === "string" && environment.HOME.trim()) {
    runtimeRoot = join(resolve(environment.HOME), ".chariox", "run")
  } else {
    runtimeRoot = join(tmpdir(), "chariox")
  }
  return join(runtimeRoot, `${kernelId}.sock`)
}

function parseProcStat(text, pid) {
  const line = String(text).trim()
  const open = line.indexOf("(")
  const close = line.lastIndexOf(")")
  if (open <= 0 || close <= open || line.slice(0, open).trim() !== String(pid)) {
    throw bindingError("kernel_process_identity_invalid", "running kernel process stat identity is malformed")
  }
  const fields = line.slice(close + 1).trim().split(/\s+/)
  const state = fields[0]
  const startTimeTicks = fields[19]
  if (!state || state === "Z" || state === "X" || !/^\d+$/.test(startTimeTicks ?? "")) {
    throw bindingError("kernel_process_exited", "running kernel process is no longer live")
  }
  return startTimeTicks
}

function socketPathFromProcNetUnix(line) {
  const match = /^\s*\S+:\s+\S+\s+\S+\s+(\S+)\s+(\S+)\s+(\S+)\s+(\d+)\s+(.+?)\s*$/.exec(line)
  return match ? { flags: match[1], type: match[2], state: match[3], inode: match[4], path: match[5] } : null
}

async function findUnixSocketInode(filesystem, socketPath) {
  let contents
  try {
    contents = await filesystem.readFile(join(PROC_ROOT, "net/unix"), "utf8")
  } catch (error) {
    throw bindingError("kernel_socket_unreadable", "Linux Unix socket table cannot be read", error)
  }
  const matches = String(contents).split("\n")
    .map(socketPathFromProcNetUnix)
    .filter((entry) => entry?.path === socketPath
      && entry.type === "0001"
      && entry.state === "01"
      && (Number.parseInt(entry.flags, 16) & 0x10000) !== 0)
  if (matches.length !== 1) {
    throw bindingError("kernel_socket_identity_mismatch", "configured local daemon socket is not uniquely present in the Linux Unix socket table")
  }
  return matches[0].inode
}

async function findSocketOwnerPid(filesystem, inode, privilegedInspector) {
  let entries
  try {
    entries = await filesystem.readdir(PROC_ROOT)
  } catch (error) {
    throw bindingError("kernel_process_table_unreadable", "Linux process table cannot be read", error)
  }

  const target = `socket:[${inode}]`
  const owners = new Set()
  let accessDenied = false
  for (const entry of entries) {
    const pid = String(entry)
    if (!/^\d+$/.test(pid)) continue
    let fds
    try {
      fds = await filesystem.readdir(join(PROC_ROOT, pid, "fd"))
    } catch (error) {
      if (["EACCES", "EPERM"].includes(error?.code)) accessDenied = true
      continue
    }
    for (const fd of fds) {
      try {
        if (await filesystem.readlink(join(PROC_ROOT, pid, "fd", String(fd))) === target) owners.add(pid)
      } catch {
        // Processes can exit while /proc is being enumerated. The socket must
        // still resolve to one readable owner for this capture to pass.
      }
    }
  }
  if (owners.size === 0 && accessDenied && privilegedInspector) {
    const inspected = await privilegedInspector("socket-owner", inode)
    if (!Array.isArray(inspected?.owners) || inspected.owners.some(pid => !Number.isSafeInteger(pid) || pid < 1)) {
      throw bindingError("kernel_socket_owner_mismatch", "privileged socket-owner metadata is invalid")
    }
    for (const pid of inspected.owners) owners.add(String(pid))
  }
  if (owners.size !== 1) {
    throw bindingError("kernel_socket_owner_mismatch", "local daemon socket does not have exactly one observable process owner")
  }
  return Number([...owners][0])
}

export async function inspectLinuxKernelProcess({
  filesystem = NODE_FILESYSTEM,
  socketPath,
  privilegedInspector = inspectPrivilegedProcMetadata,
  expectedExecutablePath,
  expectedExecutableDigest,
}) {
  const socketInode = await findUnixSocketInode(filesystem, socketPath)
  const pid = await findSocketOwnerPid(filesystem, socketInode, privilegedInspector)
  return inspectKernelProcessIdentity(filesystem, pid, socketInode, expectedExecutablePath, expectedExecutableDigest, privilegedInspector)
}

async function inspectKernelProcessIdentity(filesystem, pid, socketInode, expectedExecutablePath, expectedExecutableDigest, privilegedInspector) {
  const procPath = join(PROC_ROOT, String(pid))

  let bootId
  let stat
  let executableLink
  let executableBytes
  let executableDigest
  try {
    [bootId, stat, executableLink, executableBytes] = await Promise.all([
      filesystem.readFile(join(PROC_ROOT, "sys/kernel/random/boot_id"), "utf8"),
      filesystem.readFile(join(procPath, "stat"), "utf8"),
      filesystem.readlink(join(procPath, "exe")),
      filesystem.readFile(join(procPath, "exe")),
    ])
  } catch (error) {
    if (!["EACCES", "EPERM"].includes(error?.code) || !privilegedInspector) {
      throw bindingError("kernel_process_unreadable", "running kernel process identity or executable cannot be inspected", error)
    }
    try {
      const metadata = await privilegedInspector("process", pid)
      if (metadata?.pid !== pid || !DIGEST.test(metadata.executableDigest ?? "")) throw new Error("invalid process metadata")
      bootId = metadata.bootId
      stat = metadata.stat
      executableLink = metadata.executableLink
      executableDigest = metadata.executableDigest.toLowerCase()
    } catch (cause) {
      throw bindingError("kernel_process_unreadable", "privileged read-only process inspection failed", cause)
    }
  }

  const normalizedBootId = String(bootId).trim().toLowerCase()
  if (!BOOT_ID.test(normalizedBootId)) {
    throw bindingError("kernel_process_identity_invalid", "Linux boot ID is invalid")
  }
  const startTimeTicks = parseProcStat(stat, pid)
  if (String(executableLink).endsWith(" (deleted)")) {
    throw bindingError("kernel_executable_deleted", "running kernel executable has been deleted")
  }
  if (!isAbsolute(executableLink) || resolve(executableLink) !== expectedExecutablePath) {
    throw bindingError("kernel_executable_path_mismatch", "running kernel executable path is not the verified signed artifact")
  }
  executableDigest ??= sha256(executableBytes)
  if (executableDigest !== expectedExecutableDigest) {
    throw bindingError("kernel_executable_digest_mismatch", "running kernel executable bytes do not match the expected executable")
  }

  return {
    pid,
    linux_boot_id: normalizedBootId,
    start_time_ticks: startTimeTicks,
    socket_inode: socketInode,
    executable_path: executableLink,
    executable_sha256: executableDigest,
  }
}

function sameProcess(left, right) {
  return left.pid === right.pid
    && left.linux_boot_id === right.linux_boot_id
    && left.start_time_ticks === right.start_time_ticks
    && left.socket_inode === right.socket_inode
    && left.executable_path === right.executable_path
    && left.executable_sha256 === right.executable_sha256
}

function parseRelayStatusEnvelope(value) {
  const status = value?.response?.RelayStatus?.status
  if (value?.error != null || !status
    || typeof status.daemon_id !== "string" || !KERNEL_ID.test(status.daemon_id)
    || typeof status.machine_id !== "string" || !KERNEL_ID.test(status.machine_id)) {
    throw bindingError("kernel_ipc_identity_invalid", "local daemon IPC did not return a valid kernel and machine identity")
  }
  return { kernel_id: status.daemon_id, machine_id: status.machine_id }
}

export async function readLocalKernelIdentity(socketPath, {
  connect = net.createConnection,
  timeoutMs = IPC_TIMEOUT_MS,
  timerApi = globalThis,
} = {}) {
  return await new Promise((resolvePromise, rejectPromise) => {
    let socket
    let buffer = Buffer.alloc(0)
    let expectedLength = null
    let parsedIdentity = null
    let sawEnd = false
    let settled = false
    let deadlineHandle
    let deadlineScheduled = false
    let deadlineCleared = false
    let socketDestroyed = false

    const destroySocket = () => {
      if (!socket || socketDestroyed) return
      socketDestroyed = true
      socket.destroy()
    }
    const finish = (error, value) => {
      if (settled) return
      settled = true
      if (deadlineScheduled && !deadlineCleared) {
        deadlineCleared = true
        timerApi.clearTimeout(deadlineHandle)
      }
      destroySocket()
      if (error) {
        rejectPromise(error)
      } else {
        resolvePromise(value)
      }
    }

    deadlineScheduled = true
    deadlineHandle = timerApi.setTimeout(
      () => finish(bindingError("kernel_ipc_timeout", "local daemon IPC identity request exceeded its total deadline")),
      timeoutMs,
    )

    try {
      socket = connect({ path: socketPath })
    } catch (error) {
      finish(bindingError("kernel_ipc_unavailable", "local daemon IPC could not be opened", error))
      return
    }
    socket.on("error", (error) => finish(bindingError("kernel_ipc_unavailable", "local daemon IPC identity request failed", error)))
    socket.on("connect", () => {
      if (settled) return
      const payload = Buffer.from(JSON.stringify({ RelayStatus: null }), "utf8")
      const frame = Buffer.allocUnsafe(4 + payload.length)
      frame.writeUInt32BE(payload.length, 0)
      payload.copy(frame, 4)
      try {
        socket.end(frame)
      } catch (error) {
        finish(bindingError("kernel_ipc_unavailable", "local daemon IPC request could not be written", error))
      }
    })
    socket.on("data", (chunk) => {
      if (settled) return
      if (buffer.length + chunk.length > MAX_IPC_FRAME_BYTES + 4) {
        finish(bindingError("kernel_ipc_frame_invalid", "local daemon IPC response exceeds the reviewed frame limit"))
        return
      }
      buffer = Buffer.concat([buffer, chunk])
      if (expectedLength === null && buffer.length >= 4) {
        expectedLength = buffer.readUInt32BE(0)
        if (expectedLength > MAX_IPC_FRAME_BYTES) {
          finish(bindingError("kernel_ipc_frame_invalid", "local daemon IPC response exceeds the reviewed frame limit"))
          return
        }
      }
      if (expectedLength === null || buffer.length < expectedLength + 4) return
      if (buffer.length !== expectedLength + 4) {
        finish(bindingError("kernel_ipc_frame_invalid", "local daemon IPC response contains trailing frame data"))
        return
      }
      try {
        const envelope = JSON.parse(buffer.subarray(4).toString("utf8"))
        parsedIdentity = parseRelayStatusEnvelope(envelope)
      } catch (error) {
        finish(error instanceof LiveKernelBindingError
          ? error
          : bindingError("kernel_ipc_response_invalid", "local daemon IPC response is invalid JSON", error))
      }
    })
    socket.on("end", () => {
      sawEnd = true
      if (expectedLength === null || buffer.length < expectedLength + 4) {
        finish(bindingError("kernel_ipc_response_truncated", "local daemon IPC identity response was truncated"))
      } else if (buffer.length > expectedLength + 4) {
        finish(bindingError("kernel_ipc_frame_invalid", "local daemon IPC response contains trailing frame data"))
      } else {
        finish(null, parsedIdentity)
      }
    })
    socket.on("close", () => {
      if (!settled && !sawEnd) {
        finish(bindingError("kernel_ipc_response_closed", "local daemon IPC closed before a complete response EOF"))
      }
    })
  })
}

async function observeKernel(filesystem, options) {
  const before = await inspectLinuxKernelProcess({ filesystem, ...options })
  const identity = await readLocalKernelIdentity(options.socketPath)
  const after = await inspectLinuxKernelProcess({ filesystem, ...options })
  if (!sameProcess(before, after)) {
    throw bindingError("kernel_process_changed", "local daemon process changed while its IPC identity was observed")
  }
  if (identity.kernel_id !== options.captureIdentity.kernel_id
    || identity.machine_id !== options.captureIdentity.machine_id) {
    throw bindingError("kernel_capture_identity_mismatch", "local daemon IPC identity does not match the observed capture identity")
  }
  return { identity, process: after }
}

async function requireKernelAncestor(filesystem, collectorPid, kernelPid) {
  if (!Number.isSafeInteger(collectorPid) || collectorPid < 1 || collectorPid === kernelPid) {
    throw bindingError("kernel_process_ancestry_mismatch", "capture must run inside a provider child")
  }
  const seen = new Set()
  let pid = collectorPid
  for (let depth = 0; depth < 128; depth += 1) {
    if (pid === kernelPid) return
    if (pid <= 1 || seen.has(pid)) break
    seen.add(pid)
    const text = String(await filesystem.readFile(join(PROC_ROOT, String(pid), "stat"), "utf8"))
    parseProcStat(text, pid)
    const parent = Number(text.slice(text.lastIndexOf(")") + 2).trim().split(/\s+/)[1])
    if (!Number.isSafeInteger(parent) || parent < 0) break
    pid = parent
  }
  throw bindingError("kernel_process_ancestry_mismatch", "authenticated kernel is not a collector process ancestor")
}

async function loopbackListenerInode(filesystem, endpoint) {
  const url = new URL(endpoint)
  const port = Number(url.port || 80).toString(16).padStart(4, "0").toUpperCase()
  const ipv6 = url.hostname === "[::1]"
  const address = ipv6 ? "00000000000000000000000001000000" : "0100007F"
  const table = String(await filesystem.readFile(join(PROC_ROOT, ipv6 ? "net/tcp6" : "net/tcp"), "utf8"))
  const inodes = table.split("\n").map(line => line.trim().split(/\s+/))
    .filter(fields => fields[1]?.toUpperCase() === `${address}:${port}` && fields[3] === "0A")
    .map(fields => fields[9])
  if (inodes.length !== 1 || !/^\d+$/.test(inodes[0])) {
    throw bindingError("kernel_socket_identity_mismatch", "loopback kernel listener is not uniquely present in the Linux TCP table")
  }
  return inodes[0]
}

async function observeProductKernel(filesystem, processApi, options, readKernelStatus) {
  const status = await readKernelStatus()
  if (status?.daemon_id !== options.captureIdentity.kernel_id || status?.machine_id !== options.captureIdentity.machine_id) {
    throw bindingError("kernel_capture_identity_mismatch", "product response belongs to a foreign capture kernel")
  }
  const native = status.runtime_process_identity
  if (!native || !Number.isSafeInteger(native.pid) || native.pid < 1
    || !BOOT_ID.test(native.linux_boot_id ?? "") || !/^\d+$/.test(native.start_time_ticks ?? "")) {
    throw bindingError("kernel_native_process_identity_missing", "product kernel must report its native Linux process identity")
  }
  const inode = options.captureIdentity.transport === "kernel-public-api"
    ? await loopbackListenerInode(filesystem, options.socketPath) : null
  if (inode !== null && await findSocketOwnerPid(filesystem, inode, options.privilegedInspector) !== native.pid) {
    throw bindingError("kernel_socket_owner_mismatch", "product endpoint listener is not owned by the responding kernel")
  }
  await requireKernelAncestor(filesystem, processApi.pid, native.pid)
  const observed = await inspectKernelProcessIdentity(filesystem, native.pid, inode, options.expectedExecutablePath, options.expectedExecutableDigest, options.privilegedInspector)
  if (observed.linux_boot_id !== native.linux_boot_id.toLowerCase() || observed.start_time_ticks !== native.start_time_ticks) {
    throw bindingError("kernel_native_process_identity_mismatch", "authenticated product process identity does not match the independent Linux observation")
  }
  return { identity: { kernel_id: status.daemon_id, machine_id: status.machine_id }, process: observed }
}

export async function startManagedOrdinaryLiveKernelBinding({
  filesystem = NODE_FILESYSTEM,
  processApi = process,
  privilegedInspector = inspectPrivilegedProcMetadata,
  selectedKernelPath,
  expectedArtifactDigest,
  expectedBoundary,
  readKernelStatus,
}) {
  if (processApi.platform !== "linux") {
    throw bindingError("kernel_binding_unsupported_platform", "live kernel process binding requires Linux")
  }
  if (!DIGEST.test(expectedArtifactDigest ?? "")) {
    throw bindingError("kernel_artifact_digest_invalid", "verified kernel artifact digest is invalid")
  }
  if (typeof selectedKernelPath !== "string" || !isAbsolute(selectedKernelPath)) {
    throw bindingError("kernel_artifact_path_invalid", "verified kernel artifact path must be absolute")
  }

  const captureIdentity = parseCaptureIdentity(processApi.env, expectedBoundary)
  const connection = managedOrdinaryKernelConnection(processApi.env, () => localSocketPath(processApi.env, captureIdentity.kernel_id))
  if (connection.transport !== captureIdentity.transport) throw bindingError("kernel_capture_transport_mismatch", "capture and product endpoint transports differ")
  const socketPath = connection.endpoint
  if (connection.transport !== "local-unix-ipc" && typeof readKernelStatus !== "function") {
    throw bindingError("kernel_product_client_missing", "live product binding requires the authenticated provider-turn client")
  }
  let expectedKernelPath
  try {
    expectedKernelPath = await filesystem.realpath(selectedKernelPath)
  } catch (error) {
    throw bindingError("kernel_artifact_unreadable", "verified kernel artifact path cannot be resolved", error)
  }

  const options = {
    privilegedInspector,
    socketPath,
    expectedExecutablePath: expectedKernelPath,
    expectedExecutableDigest: expectedArtifactDigest.toLowerCase(),
    captureIdentity,
  }
  const observe = () => connection.transport === "local-unix-ipc"
    ? observeKernel(filesystem, options)
    : observeProductKernel(filesystem, processApi, options, readKernelStatus)
  const initial = await observe()

  return {
    async finish() {
      const finalCaptureIdentity = parseCaptureIdentity(processApi.env, expectedBoundary)
      if (JSON.stringify(finalCaptureIdentity) !== JSON.stringify(captureIdentity)) {
        throw bindingError("kernel_capture_identity_changed", "observed capture kernel identity changed during collection")
      }
      const final = await observe()
      if (!sameProcess(initial.process, final.process)) {
        throw bindingError("kernel_process_changed", "running kernel process identity changed during collection")
      }
      if (initial.identity.kernel_id !== final.identity.kernel_id
        || initial.identity.machine_id !== final.identity.machine_id) {
        throw bindingError("kernel_capture_identity_changed", "local daemon identity changed during collection")
      }
      return {
        schema: LIVE_KERNEL_BINDING_SCHEMA,
        observed: true,
        capture_boundary: captureIdentity.boundary,
        capture_identity: captureIdentity,
        observed_kernel: {
          kernel_id: final.identity.kernel_id,
          machine_id: final.identity.machine_id,
        },
        transport: {
          kind: connection.transport,
          ...(connection.transport === "local-unix-ipc"
            ? { socket_path_sha256: sha256(Buffer.from(resolve(socketPath), "utf8")) }
            : { endpoint_sha256: sha256(Buffer.from(socketPath, "utf8")) }),
        },
        kernel_process: {
          pid: final.process.pid,
          linux_boot_id: final.process.linux_boot_id,
          start_time_ticks: final.process.start_time_ticks,
          executable_path: final.process.executable_path,
          executable_sha256: final.process.executable_sha256,
        },
        verified_artifact: {
          path: expectedKernelPath,
          sha256: expectedArtifactDigest,
        },
        stable_across_capture: true,
      }
    },
  }
}
