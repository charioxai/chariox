import path from "node:path"

const DEFAULT_TIMEOUT_MS = 60_000
const MAX_TIMEOUT_MS = 120_000

function requireValue(argv, index, option) {
  const value = argv[index + 1]
  if (!value || value.startsWith("--")) {
    throw new Error(`${option} requires a value`)
  }
  return value
}

function parsePositiveInteger(value, option) {
  if (!/^\d+$/.test(value)) {
    throw new Error(`${option} must be a positive integer`)
  }
  const parsed = Number(value)
  if (!Number.isSafeInteger(parsed) || parsed < 1) {
    throw new Error(`${option} must be a positive integer`)
  }
  return parsed
}

export function parseCandidateKernelSmokeArgs(argv) {
  let binary = null
  let expectedProtocol = null
  let timeoutMs = DEFAULT_TIMEOUT_MS
  let help = false

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--") continue
    if (arg === "--help" || arg === "-h") {
      help = true
      continue
    }
    if (arg === "--binary") {
      binary = path.resolve(requireValue(argv, index, "--binary"))
      index += 1
      continue
    }
    if (arg === "--expected-protocol") {
      expectedProtocol = parsePositiveInteger(requireValue(argv, index, "--expected-protocol"), "--expected-protocol")
      index += 1
      continue
    }
    if (arg === "--timeout-ms") {
      timeoutMs = parsePositiveInteger(requireValue(argv, index, "--timeout-ms"), "--timeout-ms")
      index += 1
      if (timeoutMs > MAX_TIMEOUT_MS) {
        throw new Error(`--timeout-ms must be at most ${MAX_TIMEOUT_MS}`)
      }
      continue
    }
    throw new Error(`unknown option: ${arg}`)
  }

  if (help) return { help: true, binary: null, expectedProtocol: null, timeoutMs }
  if (!binary) throw new Error("--binary is required")
  if (expectedProtocol == null) throw new Error("--expected-protocol is required")
  return { help: false, binary, expectedProtocol, timeoutMs }
}

export function candidateKernelSmokeUsage() {
  return "Usage: node scripts/candidate-kernel-startup-smoke-drill.mjs --binary PATH --expected-protocol N [--timeout-ms MS]"
}

export function assertCandidateProtocolOutput(output, expectedProtocol) {
  const actual = String(output ?? "").trim()
  const expected = String(expectedProtocol)
  if (actual !== expected) {
    throw new Error(`candidate local daemon protocol mismatch: expected ${expected}, got ${actual || "<empty>"}`)
  }
  return Number(expected)
}

export function candidateKernelEndpoint(kernelPort) {
  if (!Number.isSafeInteger(kernelPort) || kernelPort < 1 || kernelPort > 65_535) {
    throw new Error("candidate kernel port is invalid")
  }
  return `ws://127.0.0.1:${kernelPort}`
}

export function makeCandidateKernelEnvironment({ rootDir, ports, tokenFile, runId, baseEnv = {} }) {
  if (!path.isAbsolute(rootDir) || !path.isAbsolute(tokenFile)) {
    throw new Error("candidate kernel smoke paths must be absolute")
  }
  if (!runId || /[^A-Za-z0-9_.-]/.test(runId)) {
    throw new Error("candidate kernel smoke run id is invalid")
  }
  const fallbackPath = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
  return {
    PATH: baseEnv.PATH || fallbackPath,
    USER: baseEnv.USER || "chariox-drill",
    LANG: baseEnv.LANG || "C.UTF-8",
    LC_ALL: baseEnv.LC_ALL || "C.UTF-8",
    CHARIOX_HOME: path.join(rootDir, "chariox-home"),
    HOME: path.join(rootDir, "home"),
    XDG_CONFIG_HOME: path.join(rootDir, "xdg-config"),
    XDG_STATE_HOME: path.join(rootDir, "xdg-state"),
    XDG_RUNTIME_DIR: path.join(rootDir, "xdg-runtime"),
    XDG_DATA_HOME: path.join(rootDir, "xdg-data"),
    XDG_CACHE_HOME: path.join(rootDir, "xdg-cache"),
    CHARIOX_LOG_DIR: path.join(rootDir, "logs"),
    CHARIOX_SESSION_HISTORY_DIR: path.join(rootDir, "history"),
    CHARIOX_DAEMON_SOCKET: path.join(rootDir, "daemon.sock"),
    CHARIOX_DAEMON_ID: `candidate-kernel-smoke-${runId}`,
    CHARIOX_MACHINE_ID: `candidate-machine-${runId}`,
    CHARIOX_KERNEL_HOST: "127.0.0.1",
    CHARIOX_KERNEL_PORT: String(ports.kernelPort),
    CHARIOX_MCP_HOST: "127.0.0.1",
    CHARIOX_MCP_PORT: String(ports.mcpPort),
    CHARIOX_OPENCODE_PORT: String(ports.openCodePort),
    CHARIOX_CODEX_PORT: String(ports.codexPort),
    CHARIOX_ACCEPT_REMOTE_LEASES: "0",
    CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE: tokenFile,
  }
}

export function assertNoRelayOrProviderEnvironment(env) {
  const forbidden = [
    "CHARIOX_RELAY_URL",
    "CHARIOX_RELAY_TOKEN",
    "CHARIOX_CLOUD_RELAY_CONFIG_JSON",
    "CHARIOX_CLOUD_RELAY_CONFIG_PATH",
    "CHARIOX_AEDS_URL",
    "CHARIOX_AEDS_TOKEN",
    "CHARIOX_EVENT_REGISTRY_URL",
    "CHARIOX_PROVIDER_DEV_STUB",
    "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN",
  ]
  const present = forbidden.filter((name) => env[name] !== undefined)
  if (present.length > 0) {
    throw new Error(`candidate kernel smoke environment must not include ${present.join(", ")}`)
  }
  return true
}
