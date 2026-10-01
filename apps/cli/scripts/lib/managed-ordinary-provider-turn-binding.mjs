import { managedOrdinaryKernelConnection } from "./managed-ordinary-kernel-endpoint.mjs"
import { createHash } from "node:crypto"
import { readFile, readlink } from "node:fs/promises"
import { basename, isAbsolute, join, resolve } from "node:path"
import { tmpdir } from "node:os"

export const CAPTURE_PROVENANCE_SCHEMA = "chariox.managed-ordinary-capture-provenance/v1"

const BOOT_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
const IDENTIFIER = /^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$/
const PROC_ROOT = "/proc"
const MAX_PARENT_DEPTH = 128
const NODE_FILESYSTEM = Object.freeze({ readFile, readlink })

export class ProviderTurnBindingError extends Error {
  constructor(code, message, options = {}) {
    super(message, options)
    this.name = "ProviderTurnBindingError"
    this.code = code
  }
}

function bindingError(code, message, cause) {
  return new ProviderTurnBindingError(code, message, cause ? { cause } : undefined)
}

function sha256(value) {
  return `sha256:${createHash("sha256").update(value).digest("hex")}`
}

function stableJson(value) {
  if (Array.isArray(value)) return value.map(stableJson)
  if (!value || typeof value !== "object") return value
  return Object.fromEntries(Object.keys(value).sort().map((key) => [key, stableJson(value[key])]))
}

function fingerprintJson(value) {
  return sha256(Buffer.from(JSON.stringify(stableJson(value)), "utf8"))
}

function parseCaptureLocator(environment) {
  const raw = environment?.CHARIOX_PARITY_CAPTURE_EVIDENCE_JSON
  if (typeof raw !== "string" || !raw.trim()) return null
  try {
    const evidence = JSON.parse(raw)
    const kernelId = evidence?.kernel_identity?.kernel_id
    return typeof kernelId === "string" && IDENTIFIER.test(kernelId) ? kernelId : null
  } catch {
    // Capture evidence is not authority. It is consulted only as a fallback
    // locator for the local socket when the provider environment omitted it.
    return null
  }
}

function legacyKernelSocketPath(environment = {}) {
  const configured = environment.CHARIOX_DAEMON_SOCKET
  if (typeof configured === "string" && configured.trim()) {
    if (!isAbsolute(configured)) {
      throw bindingError("kernel_socket_path_invalid", "configured local daemon socket path must be absolute")
    }
    return resolve(configured)
  }

  const kernelId = parseCaptureLocator(environment)
  if (!kernelId) {
    throw bindingError("kernel_socket_path_missing", "local daemon socket cannot be located without an absolute socket path or kernel locator")
  }

  let runtimeRoot
  if (typeof environment.CHARIOX_HOME === "string" && environment.CHARIOX_HOME.trim()) {
    runtimeRoot = join(resolve(environment.CHARIOX_HOME), "run")
  } else if (typeof environment.XDG_RUNTIME_DIR === "string" && environment.XDG_RUNTIME_DIR.trim()) {
    runtimeRoot = join(resolve(environment.XDG_RUNTIME_DIR), "chariox")
  } else if (typeof environment.HOME === "string" && environment.HOME.trim()) {
    runtimeRoot = join(resolve(environment.HOME), ".chariox", "run")
  } else {
    runtimeRoot = join(tmpdir(), "chariox")
  }
  return join(runtimeRoot, `${kernelId}.sock`)
}

export function resolveKernelSocketPath(environment = {}) {
  return managedOrdinaryKernelConnection(environment, () => legacyKernelSocketPath(environment)).endpoint
}

function parseProcStat(pid, text) {
  const line = String(text).trim()
  const open = line.indexOf("(")
  const close = line.lastIndexOf(")")
  if (open <= 0 || close <= open || line.slice(0, open).trim() !== String(pid)) {
    throw bindingError("provider_process_identity_invalid", "provider process stat identity is malformed")
  }
  const fields = line.slice(close + 1).trim().split(/\s+/)
  const state = fields[0]
  const parentPid = Number(fields[1])
  const startTimeTicks = fields[19]
  if (!state || state === "Z" || state === "X") {
    throw bindingError("provider_process_exited", "kernel-owned provider process is no longer live")
  }
  if (!Number.isSafeInteger(parentPid) || parentPid < 0 || !/^\d+$/.test(startTimeTicks ?? "")) {
    throw bindingError("provider_process_identity_invalid", "provider process stat fields are invalid")
  }
  return { state, parentPid, startTimeTicks }
}

async function readProcessStat(filesystem, pid) {
  try {
    return parseProcStat(pid, await filesystem.readFile(join(PROC_ROOT, String(pid), "stat"), "utf8"))
  } catch (error) {
    if (error instanceof ProviderTurnBindingError) throw error
    throw bindingError("provider_process_unreadable", "Linux provider process identity cannot be read", error)
  }
}

async function readProcessAncestry(filesystem, processId) {
  if (!Number.isSafeInteger(processId) || processId < 1) {
    throw bindingError("collector_process_identity_invalid", "collector process ID is invalid")
  }

  const ancestry = new Map()
  let pid = processId
  for (let depth = 0; depth < MAX_PARENT_DEPTH; depth += 1) {
    if (ancestry.has(pid)) {
      throw bindingError("provider_process_ancestry_invalid", "Linux process ancestry contains a cycle")
    }
    const stat = await readProcessStat(filesystem, pid)
    ancestry.set(pid, stat)
    if (pid === 1 || stat.parentPid === 0 || stat.parentPid === pid) return ancestry
    pid = stat.parentPid
  }
  throw bindingError("provider_process_ancestry_too_deep", "Linux process ancestry exceeds the reviewed depth bound")
}

function responseVariant(value, variant, operation) {
  const result = value?.[variant]
  if (!result || value.error != null) {
    throw bindingError("kernel_ipc_response_invalid", `local daemon ${operation} did not return ${variant}`)
  }
  return result
}

function requiredId(value, label) {
  if (typeof value !== "string" || !IDENTIFIER.test(value)) {
    throw bindingError("kernel_identity_invalid", `kernel response is missing a valid ${label}`)
  }
  return value
}

function processLaunchFingerprint(run, processInfo, processIdentity, ancestryDepth) {
  const program = run.pty_program
  const args = run.pty_args
  const workingDirectory = run.working_directory
  if (typeof program !== "string" || !program.trim() || !Array.isArray(args)
    || args.some((arg) => typeof arg !== "string")
    || (workingDirectory != null && typeof workingDirectory !== "string")) {
    throw bindingError("provider_launch_context_invalid", "kernel provider run has invalid launch context")
  }
  if (processInfo.process_label !== run.process_label || processInfo.provider !== run.provider) {
    throw bindingError("provider_process_run_mismatch", "kernel provider process label or provider does not match its run")
  }

  return {
    pid: processIdentity.pid,
    linux_boot_id: processIdentity.linuxBootId,
    start_time_ticks: processIdentity.startTimeTicks,
    ancestry_depth: ancestryDepth,
    executable_basename: basename(processIdentity.executablePath),
    executable_path_sha256: sha256(Buffer.from(processIdentity.executablePath, "utf8")),
    executable_sha256: processIdentity.executableDigest,
    command_line_sha256: processIdentity.commandLineDigest,
    current_working_directory_sha256: processIdentity.cwdDigest,
    launch_program_basename: basename(program),
    launch_program_sha256: sha256(Buffer.from(program, "utf8")),
    launch_arguments_sha256: fingerprintJson(args),
    launch_working_directory_sha256: workingDirectory == null
      ? null
      : sha256(Buffer.from(workingDirectory, "utf8")),
  }
}

async function inspectProviderProcess(filesystem, pid, ancestryStat) {
  const procPath = join(PROC_ROOT, String(pid))
  let bootId
  let stat
  let executablePath
  let executableBytes
  let commandLine
  let currentWorkingDirectory
  try {
    [bootId, stat, executablePath, executableBytes, commandLine, currentWorkingDirectory] = await Promise.all([
      filesystem.readFile(join(PROC_ROOT, "sys/kernel/random/boot_id"), "utf8"),
      filesystem.readFile(join(procPath, "stat"), "utf8"),
      filesystem.readlink(join(procPath, "exe")),
      filesystem.readFile(join(procPath, "exe")),
      filesystem.readFile(join(procPath, "cmdline")),
      filesystem.readlink(join(procPath, "cwd")),
    ])
  } catch (error) {
    throw bindingError("provider_process_unreadable", "kernel-owned provider executable or launch context cannot be inspected", error)
  }

  const processStat = parseProcStat(pid, stat)
  if (processStat.startTimeTicks !== ancestryStat.startTimeTicks) {
    throw bindingError("provider_process_changed", "provider PID changed while its process ancestry was inspected")
  }
  const linuxBootId = String(bootId).trim().toLowerCase()
  if (!BOOT_ID.test(linuxBootId)) {
    throw bindingError("provider_process_identity_invalid", "Linux boot ID is invalid")
  }
  const link = String(executablePath)
  if (link.endsWith(" (deleted)")) {
    throw bindingError("provider_executable_deleted", "kernel-owned provider executable has been deleted")
  }
  if (!isAbsolute(link) || !Buffer.from(commandLine).length || !isAbsolute(String(currentWorkingDirectory))) {
    throw bindingError("provider_launch_context_invalid", "provider executable, command line, or working directory is invalid")
  }

  return {
    pid,
    linuxBootId,
    startTimeTicks: processStat.startTimeTicks,
    executablePath: link,
    executableDigest: sha256(Buffer.from(executableBytes)),
    commandLineDigest: sha256(Buffer.from(commandLine)),
    cwdDigest: sha256(Buffer.from(String(currentWorkingDirectory), "utf8")),
  }
}

function sameProcessIdentity(left, right) {
  return left.pid === right.pid
    && left.linux_boot_id === right.linux_boot_id
    && left.start_time_ticks === right.start_time_ticks
    && left.executable_basename === right.executable_basename
    && left.executable_path_sha256 === right.executable_path_sha256
    && left.executable_sha256 === right.executable_sha256
    && left.command_line_sha256 === right.command_line_sha256
    && left.current_working_directory_sha256 === right.current_working_directory_sha256
    && left.launch_program_basename === right.launch_program_basename
    && left.launch_program_sha256 === right.launch_program_sha256
    && left.launch_arguments_sha256 === right.launch_arguments_sha256
    && left.launch_working_directory_sha256 === right.launch_working_directory_sha256
}

function sameTurnIdentity(left, right) {
  return left.kernel_identity.kernel_id === right.kernel_identity.kernel_id
    && left.kernel_identity.machine_id === right.kernel_identity.machine_id
    && left.kernel_identity.transport === right.kernel_identity.transport
    && left.provider.provider_run_id === right.provider.provider_run_id
    && left.provider.name === right.provider.name
    && left.session_id === right.session_id
    && left.agent_id === right.agent_id
    && left.attachment_id === right.attachment_id
    && left.prompt_id === right.prompt_id
    && left.prompt_origin === right.prompt_origin
    && left.prompt_status === right.prompt_status
    && left.provider.status === right.provider.status
    && left.provider.process_status === right.provider.process_status
    && left.process.pid === right.process.pid
    && left.process.linux_boot_id === right.process.linux_boot_id
    && left.process.start_time_ticks === right.process.start_time_ticks
    && left.process.ancestry_depth === right.process.ancestry_depth
    && left.process.executable_basename === right.process.executable_basename
    && left.process.executable_path_sha256 === right.process.executable_path_sha256
    && left.process.executable_sha256 === right.process.executable_sha256
    && left.process.command_line_sha256 === right.process.command_line_sha256
    && left.process.current_working_directory_sha256 === right.process.current_working_directory_sha256
    && left.process.launch_program_basename === right.process.launch_program_basename
    && left.process.launch_program_sha256 === right.process.launch_program_sha256
    && left.process.launch_arguments_sha256 === right.process.launch_arguments_sha256
    && left.process.launch_working_directory_sha256 === right.process.launch_working_directory_sha256
}

async function defaultDependencies() {
  const [ipcModule, requestModule] = await Promise.all([
    import(new URL("../../dist/ipc.js", import.meta.url)),
    import(new URL("../../dist/ipc-requests.js", import.meta.url)),
  ])
  if (typeof ipcModule.LocalIpcClient !== "function") {
    throw bindingError("kernel_ipc_client_missing", "built local IPC client is unavailable")
  }
  return {
    clientFactory: (endpoint, options) => new ipcModule.LocalIpcClient(endpoint, options),
    requestBuilders: {
      relayStatusRequest: requestModule.relayStatusRequest,
      getProviderRunRequest: requestModule.getProviderRunRequest,
      getSessionStateRequest: requestModule.getSessionStateRequest,
      listProviderProcessesRequest: requestModule.listProviderProcessesRequest,
    },
  }
}

async function resolveDependencies(options) {
  if (options.clientFactory && options.requestBuilders) {
    return { clientFactory: options.clientFactory, requestBuilders: options.requestBuilders }
  }
  const defaults = await defaultDependencies()
  return {
    clientFactory: options.clientFactory ?? defaults.clientFactory,
    requestBuilders: options.requestBuilders ?? defaults.requestBuilders,
  }
}

function validateDependencies({ clientFactory, requestBuilders }) {
  if (typeof clientFactory !== "function") {
    throw new TypeError("clientFactory must be a function")
  }
  for (const name of ["relayStatusRequest", "getProviderRunRequest", "getSessionStateRequest", "listProviderProcessesRequest"]) {
    if (typeof requestBuilders?.[name] !== "function") {
      throw new TypeError(`requestBuilders.${name} must be a function`)
    }
  }
}

async function observeBinding({ filesystem, processApi, expectedProvider, clientFactory, requestBuilders, connection }) {
  const client = clientFactory(connection.endpoint, connection.clientOptions)
  try {
    const relayStatus = responseVariant(
      await client.send(requestBuilders.relayStatusRequest()),
      "RelayStatus",
      "RelayStatus",
    )
    const daemonId = requiredId(relayStatus.status?.daemon_id, "kernel ID")
    const machineId = requiredId(relayStatus.status?.machine_id, "machine ID")
    if (connection.transport === "relay" && daemonId !== connection.clientOptions.targetDaemonId) {
      throw bindingError("kernel_identity_invalid", "authenticated product route returned a foreign target kernel")
    }
    const kernelIdentity = {
      kernel_id: daemonId,
      machine_id: machineId,
      transport: connection.transport,
    }

    const [processEnvelope, ancestry] = await Promise.all([
      client.send(requestBuilders.listProviderProcessesRequest()),
      readProcessAncestry(filesystem, processApi.pid),
    ])
    const processList = responseVariant(processEnvelope, "ProviderProcessesListed", "ListProviderProcesses")
    if (!Array.isArray(processList.processes)) {
      throw bindingError("provider_process_list_invalid", "kernel provider process list is malformed")
    }
    const candidates = processList.processes.filter((processInfo) => Number.isSafeInteger(processInfo.pid)
      && ancestry.has(processInfo.pid)
      && processInfo.status === "active"
      && processInfo.provider === expectedProvider
      && Array.isArray(processInfo.owner_provider_run_ids)
      && processInfo.owner_provider_run_ids.length > 0)
    if (candidates.length !== 1) {
      throw bindingError("provider_process_owner_mismatch", "collector is not descended from exactly one active kernel-owned provider process")
    }

    const processInfo = candidates[0]
    const providerPid = processInfo.pid
    const ancestryDepth = [...ancestry.keys()].indexOf(providerPid)
    if (!Array.isArray(processInfo.owner_session_ids) || processInfo.owner_session_ids.length === 0) {
      throw bindingError("provider_process_owner_mismatch", "kernel provider process has no owned session")
    }

    const matchingTurns = []
    for (const providerRunId of processInfo.owner_provider_run_ids) {
      const runEnvelope = await client.send(requestBuilders.getProviderRunRequest(providerRunId))
      const runResult = responseVariant(runEnvelope, "ProviderRun", "GetProviderRun")
      const run = runResult.provider_run
      if (!run || run.id !== providerRunId || run.provider !== expectedProvider || run.state !== "Running") continue
      if (typeof run.session_id !== "string" || typeof run.agent_instance_id !== "string") continue
      if (!processInfo.owner_session_ids.includes(run.session_id)) continue
      if (!Array.isArray(run.pty_args) || typeof run.pty_program !== "string" || !run.pty_program.trim()) continue

      const sessionEnvelope = await client.send(requestBuilders.getSessionStateRequest(run.session_id))
      const sessionResult = responseVariant(sessionEnvelope, "SessionState", "GetSessionState")
      const session = sessionResult.session
      if (!session || session.id !== run.session_id || !Array.isArray(session.agents)
        || !session.agents.some((agent) => agent.id === run.agent_instance_id)) continue
      const activeTurn = sessionResult.agent_activity?.[run.agent_instance_id]?.active_turn
      if (!activeTurn || activeTurn.provider_run_id !== run.id
        || typeof activeTurn.prompt_id !== "string" || !activeTurn.prompt_id
        || typeof activeTurn.source_attachment_id !== "string" || !activeTurn.source_attachment_id
        || activeTurn.prompt_origin !== "chariox"
        || activeTurn.status !== "running"
        || !["awaiting_first_output", "streaming"].includes(activeTurn.phase)) continue

      matchingTurns.push({ run, session, activeTurn })
    }
    if (matchingTurns.length !== 1) {
      throw bindingError("provider_turn_identity_mismatch", "kernel process does not own exactly one matching active official provider turn")
    }

    const { run, session, activeTurn } = matchingTurns[0]
    const processIdentity = await inspectProviderProcess(filesystem, providerPid, ancestry.get(providerPid))
    const processProof = processLaunchFingerprint(run, processInfo, processIdentity, ancestryDepth)
    const proof = {
      schema: CAPTURE_PROVENANCE_SCHEMA,
      boundary: "official-provider-turn",
      observed: true,
      kernel_identity: kernelIdentity,
      session_id: requiredId(session.id, "session ID"),
      agent_id: requiredId(run.agent_instance_id, "agent ID"),
      attachment_id: requiredId(activeTurn.source_attachment_id, "source attachment ID"),
      prompt_id: requiredId(activeTurn.prompt_id, "prompt ID"),
      prompt_origin: activeTurn.prompt_origin,
      prompt_status: activeTurn.status,
      prompt_phase_start: activeTurn.phase,
      provider: {
        provider_run_id: requiredId(run.id, "provider run ID"),
        name: requiredId(run.provider, "provider name"),
        status: run.state,
        process_status: processInfo.status,
      },
      process: processProof,
    }
    return { proof, processIdentity }
  } catch (error) {
    throw error instanceof ProviderTurnBindingError
      ? error
      : bindingError("provider_turn_observation_failed", "kernel provider-turn provenance could not be observed", error)
  } finally {
    await client.close?.()
  }
}

export async function startManagedOrdinaryProviderTurnBinding({
  filesystem = NODE_FILESYSTEM,
  processApi = process,
  expectedProvider,
  expectedBoundary = "official-provider-turn",
  clientFactory,
  requestBuilders,
} = {}) {
  if (processApi.platform !== "linux") {
    throw bindingError("provider_turn_unsupported_platform", "provider-turn process binding requires Linux")
  }
  if (expectedBoundary !== "official-provider-turn") {
    throw bindingError("provider_turn_boundary_unsupported", "this verifier accepts only kernel-proven official provider turns")
  }
  if (typeof expectedProvider !== "string" || !expectedProvider.trim()) {
    throw bindingError("provider_identity_missing", "expected provider name is required")
  }
  const connection = managedOrdinaryKernelConnection(processApi.env, () => legacyKernelSocketPath(processApi.env))
  const socketPath = connection.endpoint
  const dependencies = await resolveDependencies({ clientFactory, requestBuilders })
  validateDependencies(dependencies)

  let initial
  try {
    initial = await observeBinding({
      filesystem,
      processApi,
      expectedProvider,
      ...dependencies,
      connection,
    })
  } catch (error) {
    throw error instanceof ProviderTurnBindingError
      ? error
      : bindingError("provider_turn_observation_failed", "kernel provider-turn provenance could not be observed", error)
  }

  let finished = false
  return {
    socketPath,
    kernelIdentity: initial.proof.kernel_identity,
    async readKernelStatus() {
      const client = dependencies.clientFactory(connection.endpoint, connection.clientOptions)
      try {
        return responseVariant(await client.send(dependencies.requestBuilders.relayStatusRequest()), "RelayStatus", "RelayStatus").status
      } finally { await client.close?.() }
    },
    initialProof: initial.proof,
    async finish() {
      if (finished) {
        throw bindingError("provider_turn_binding_finished", "provider-turn binding has already been finished")
      }
      finished = true
      let final
      try {
        final = await observeBinding({
          filesystem,
          processApi,
          expectedProvider,
          ...dependencies,
          connection,
        })
      } catch (error) {
        throw error instanceof ProviderTurnBindingError
          ? error
          : bindingError("provider_turn_observation_failed", "kernel provider-turn provenance could not be rechecked", error)
      }
      if (!sameTurnIdentity(initial.proof, final.proof)
        || !sameProcessIdentity(initial.proof.process, final.proof.process)) {
        throw bindingError("provider_turn_changed", "kernel provider, prompt, or process identity changed during capture")
      }
      return {
        ...initial.proof,
        prompt_phase_end: final.proof.prompt_phase_start,
      }
    },
  }
}
