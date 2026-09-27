import path from "node:path"

export const COORDINATED_LOAD_SCHEMA = "chariox.room_coordinated_load.config.v1"
export const COORDINATED_LOAD_MAX_SLICES = 16
export const COORDINATED_LOAD_MAX_DURATION_MS = 10 * 60 * 1_000
export const COORDINATED_LOAD_UNRUN_GATES = Object.freeze({
  safeMaximumAdmission: "not_proven: the kernel has no read-only admission-capacity query; this drill verifies approved prepared slices but does not issue an extra mutating CreateSlice probe to claim limit rejection",
  cloudWebRendering: "not_proven: Selkies stream clients do not prove Cloud Web frontend rendering",
  kernelRequestWallBound: "not_proven: LocalIpcClient uses a fixed 600-second request timeout and does not expose AbortSignal cancellation for a pending kernel request",
  activeEightHourSoak: "not_run: use the separately gated browser-computer:soak runner",
  idleTwentyFourHourSoak: "not_run: use the separately gated idle-authenticated-browser-soak runner",
  managedMachine: "not_run: this source runner only accepts same-host loopback endpoints",
})

export function parseCoordinatedLoadArgs(argv) {
  const flags = new Set()
  const values = new Map()
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (["--execute-live", "--validate-config", "--help", "-h"].includes(arg)) {
      if (flags.has(arg)) throw new Error(`duplicate argument: ${arg}`)
      flags.add(arg)
      continue
    }
    if (arg !== "--config") throw new Error("unknown argument")
    if (values.has(arg)) throw new Error("duplicate argument: --config")
    const value = argv[++index]
    if (!value || value.startsWith("--")) throw new Error("--config requires a path")
    values.set(arg, value)
  }
  if (flags.has("--execute-live") === flags.has("--validate-config")) {
    if (!flags.has("--help") && !flags.has("-h")) {
      throw new Error("select exactly one of --validate-config or --execute-live")
    }
  }
  if (!flags.has("--help") && !flags.has("-h") && !values.has("--config")) {
    throw new Error("--config is required")
  }
  return {
    mode: flags.has("--execute-live") ? "execute" : flags.has("--validate-config") ? "validate" : "help",
    configPath: values.get("--config") ?? null,
  }
}

export function validateCoordinatedLoadConfig(config, { repoRoot }) {
  object(config, "config")
  exactKeys(config, [
    "schema", "runId", "approval", "homeKernel", "relay", "headedSlices",
    "workflow", "workspace", "limits", "slowViewer", "evidenceRoot",
  ], "config")
  if (config.schema !== COORDINATED_LOAD_SCHEMA) throw new Error("unsupported coordinated load schema")
  const runId = text(config.runId, "runId", /^[a-z0-9][a-z0-9-]{7,47}$/)

  object(config.approval, "approval")
  exactKeys(config.approval, ["reference", "approvedMaxHeadedSlices"], "approval")
  text(config.approval.reference, "approval.reference", /^[A-Za-z0-9][A-Za-z0-9._:/-]{2,127}$/)
  const approvedMaxHeadedSlices = integer(
    config.approval.approvedMaxHeadedSlices,
    "approval.approvedMaxHeadedSlices",
    1,
    COORDINATED_LOAD_MAX_SLICES,
  )

  object(config.homeKernel, "homeKernel")
  exactKeys(config.homeKernel, ["url", "kernelId", "machineId"], "homeKernel")
  const homeKernel = {
    url: loopbackWebSocket(config.homeKernel.url, "homeKernel.url"),
    kernelId: text(config.homeKernel.kernelId, "homeKernel.kernelId"),
    machineId: text(config.homeKernel.machineId, "homeKernel.machineId"),
  }

  object(config.relay, "relay")
  exactKeys(config.relay, ["url", "targetDaemonId", "tokenEnv"], "relay")
  const relay = {
    url: loopbackWebSocket(config.relay.url, "relay.url"),
    targetDaemonId: text(config.relay.targetDaemonId, "relay.targetDaemonId"),
    tokenEnv: text(config.relay.tokenEnv, "relay.tokenEnv", /^CHARIOX_DRILL_H_RELAY_TOKEN$/),
  }
  if (relay.targetDaemonId !== homeKernel.kernelId) {
    throw new Error("relay.targetDaemonId must match homeKernel.kernelId")
  }

  if (!Array.isArray(config.headedSlices)
    || config.headedSlices.length !== approvedMaxHeadedSlices
    || config.headedSlices.length > COORDINATED_LOAD_MAX_SLICES) {
    throw new Error("headedSlices must exactly match the explicitly approved slice bound")
  }
  const headedSlices = config.headedSlices.map((slice, index) => {
    object(slice, `headedSlices[${index}]`)
    exactKeys(slice, [
      "sliceId", "sliceName", "roomId", "environmentId", "runtimeGeneration",
      "workerKernelId", "workerMachineId", "ports",
    ], `headedSlices[${index}]`)
    const row = {
      sliceId: text(slice.sliceId, `headedSlices[${index}].sliceId`),
      sliceName: text(slice.sliceName, `headedSlices[${index}].sliceName`, new RegExp(`^drillh-${escapeRegExp(runId)}-[a-z0-9-]{1,32}$`)),
      roomId: text(slice.roomId, `headedSlices[${index}].roomId`),
      environmentId: text(slice.environmentId, `headedSlices[${index}].environmentId`),
      runtimeGeneration: integer(slice.runtimeGeneration, `headedSlices[${index}].runtimeGeneration`, 1, Number.MAX_SAFE_INTEGER),
      workerKernelId: text(slice.workerKernelId, `headedSlices[${index}].workerKernelId`),
      workerMachineId: text(slice.workerMachineId, `headedSlices[${index}].workerMachineId`),
      ports: validatePorts(slice.ports, `headedSlices[${index}].ports`),
    }
    row.publishedPorts = publishedPorts(row.ports)
    if (row.workerKernelId !== homeKernel.kernelId || row.workerMachineId !== homeKernel.machineId) {
      throw new Error(`headedSlices[${index}] must be a local slice on the configured home kernel and machine`)
    }
    return row
  })
  for (const field of ["sliceId", "sliceName", "roomId", "environmentId"]) {
    assertUnique(headedSlices.map((slice) => slice[field]), `headedSlices.${field}`)
  }
  assertUnique(headedSlices.flatMap((slice) => [
    ...Object.entries(slice.ports)
      .filter(([key]) => !key.endsWith("_range_start"))
      .map(([, port]) => port),
    ...rangePorts(slice.ports.codex_range_start),
    ...rangePorts(slice.ports.opencode_range_start),
  ]), "headedSlices.ports")

  object(config.workflow, "workflow")
  exactKeys(config.workflow, ["roomId", "workflowId", "endpointId", "prompt"], "workflow")
  const workflow = {
    roomId: text(config.workflow.roomId, "workflow.roomId"),
    workflowId: text(config.workflow.workflowId, "workflow.workflowId"),
    endpointId: text(config.workflow.endpointId, "workflow.endpointId"),
    prompt: text(config.workflow.prompt, "workflow.prompt", null, 1_000),
  }
  if (!headedSlices.some((slice) => slice.roomId === workflow.roomId)) {
    throw new Error("workflow.roomId must be one of the prepared Room identities")
  }

  object(config.workspace, "workspace")
  exactKeys(config.workspace, ["path", "worktree"], "workspace")
  const workspace = {
    path: externalAbsolutePath(config.workspace.path, repoRoot, "workspace.path"),
    worktree: externalAbsolutePath(config.workspace.worktree, repoRoot, "workspace.worktree"),
  }

  object(config.limits, "limits")
  exactKeys(config.limits, [
    "durationMs", "sampleIntervalMs", "maxKernelLatencyMs", "maxMemoryBytes", "maxCpuPercent",
  ], "limits")
  const limits = {
    durationMs: integer(config.limits.durationMs, "limits.durationMs", 5_000, COORDINATED_LOAD_MAX_DURATION_MS),
    sampleIntervalMs: integer(config.limits.sampleIntervalMs, "limits.sampleIntervalMs", 500, 30_000),
    maxKernelLatencyMs: integer(config.limits.maxKernelLatencyMs, "limits.maxKernelLatencyMs", 1, 60_000),
    maxMemoryBytes: integer(config.limits.maxMemoryBytes, "limits.maxMemoryBytes", 64 * 1024 * 1024, 1024 * 1024 * 1024 * 1024),
    maxCpuPercent: number(config.limits.maxCpuPercent, "limits.maxCpuPercent", 1, 100_000),
  }
  if (limits.sampleIntervalMs >= limits.durationMs) throw new Error("sampleIntervalMs must be shorter than durationMs")

  object(config.slowViewer, "slowViewer")
  exactKeys(config.slowViewer, ["viewerId", "afterSample", "delayMs"], "slowViewer")
  const slowViewer = {
    viewerId: text(config.slowViewer.viewerId, "slowViewer.viewerId", /^(web-local|web-relay)$/),
    afterSample: integer(config.slowViewer.afterSample, "slowViewer.afterSample", 0, Math.ceil(limits.durationMs / limits.sampleIntervalMs) - 1),
    delayMs: integer(config.slowViewer.delayMs, "slowViewer.delayMs", 100, 5_000),
  }
  const evidenceRoot = externalAbsolutePath(config.evidenceRoot, repoRoot, "evidenceRoot")

  return {
    schema: COORDINATED_LOAD_SCHEMA,
    runId,
    approval: { reference: config.approval.reference, approvedMaxHeadedSlices },
    homeKernel,
    relay,
    headedSlices,
    workflow,
    workspace,
    limits,
    slowViewer,
    evidenceRoot,
  }
}

function loopbackWebSocket(value, label) {
  text(value, label)
  let endpoint
  try { endpoint = new URL(value) } catch { throw new Error(`${label} must be a loopback WebSocket URL`) }
  if (!new Set(["ws:", "wss:"]).has(endpoint.protocol)
    || endpoint.username || endpoint.password || endpoint.search || endpoint.hash
    || !isLoopbackHost(endpoint.hostname)) {
    throw new Error(`${label} must be a credential-free loopback WebSocket URL`)
  }
  return endpoint.href
}

function isLoopbackHost(hostname) {
  const host = hostname.toLowerCase().replace(/^\[|\]$/g, "")
  if (["localhost", "::1"].includes(host)) return true
  const parts = host.split(".").map(Number)
  return parts.length === 4 && parts.every((part) => Number.isInteger(part) && part >= 0 && part <= 255)
    && parts[0] === 127
}

function externalAbsolutePath(value, repoRoot, label) {
  text(value, label)
  if (!path.isAbsolute(value)) throw new Error(`${label} must be absolute`)
  const resolved = path.resolve(value)
  const relative = path.relative(path.resolve(repoRoot), resolved)
  if (relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative))) {
    throw new Error(`${label} must be outside the repository`)
  }
  return resolved
}

function exactKeys(value, keys, label) {
  const actual = Object.keys(value).sort()
  const expected = [...keys].sort()
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    throw new Error(`${label} has missing or unsupported fields`)
  }
}

function object(value, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${label} must be an object`)
}

function text(value, label, pattern = null, maxLength = 256) {
  if (typeof value !== "string" || value.length === 0 || value.length > maxLength || value.trim() !== value
    || (pattern && !pattern.test(value))) {
    throw new Error(`${label} is invalid`)
  }
  return value
}

function integer(value, label, minimum, maximum) {
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) throw new Error(`${label} is out of bounds`)
  return value
}

function number(value, label, minimum, maximum) {
  if (!Number.isFinite(value) || value < minimum || value > maximum) throw new Error(`${label} is out of bounds`)
  return value
}

function assertUnique(values, label) {
  if (new Set(values).size !== values.length) throw new Error(`${label} must be unique`)
}

function validatePorts(value, label) {
  const keys = [
    "codex", "opencode", "kernel", "mcp", "relay", "novnc",
    "codex_range_start", "opencode_range_start",
  ]
  object(value, label)
  exactKeys(value, keys, label)
  const ports = Object.fromEntries(keys.map((key) => [key, integer(value[key], `${label}.${key}`, 1, 65_535)]))
  assertUnique(Object.values(ports), label)
  if (ports.codex_range_start > 65_516 || ports.opencode_range_start > 65_516) {
    throw new Error(`${label} published port range exceeds the TCP port limit`)
  }
  return ports
}

function rangePorts(start) {
  return Array.from({ length: 20 }, (_, index) => start + index)
}

function publishedPorts(ports) {
  return [ports.codex, ports.opencode, ports.kernel, ports.relay, ports.novnc,
    ...rangePorts(ports.codex_range_start), ...rangePorts(ports.opencode_range_start)]
    .sort((left, right) => left - right)
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
}
