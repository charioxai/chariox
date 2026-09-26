import { isAbsolute, resolve } from "node:path"

export const SHUTDOWN_TRIGGER_SCHEMA = "chariox.managed-shutdown-trigger-observation/v1"

export const SHUTDOWN_SCENARIOS = Object.freeze({
  shutdown_agents_done: Object.freeze({ mode: "agents_done", policy: { minimumRuntimeSeconds: 0, idleDelaySeconds: 900 }, minimumSeconds: 600 }),
  shutdown_idle_15m: Object.freeze({ mode: "idle_stop", policy: { minimumRuntimeSeconds: 0, idleDelaySeconds: 900 }, minimumSeconds: 1_500 }),
  shutdown_idle_30m: Object.freeze({ mode: "idle_stop", policy: { minimumRuntimeSeconds: 0, idleDelaySeconds: 1_800 }, minimumSeconds: 2_400 }),
  shutdown_minimum_3h: Object.freeze({ mode: "minimum_runtime", policy: { minimumRuntimeSeconds: 10_800, idleDelaySeconds: 14_400 }, minimumSeconds: 12_600 }),
  shutdown_disabled: Object.freeze({ mode: "disabled", policy: { minimumRuntimeSeconds: 0, idleDelaySeconds: null }, minimumSeconds: 600 }),
  shutdown_keep_running: Object.freeze({ mode: "keep_running", policy: { minimumRuntimeSeconds: 0, idleDelaySeconds: 900 }, minimumSeconds: 1_500 }),
  shutdown_restart_reconciliation: Object.freeze({ mode: "restart_reconciliation", policy: { minimumRuntimeSeconds: 0, idleDelaySeconds: 900 }, minimumSeconds: 1_500 }),
  shutdown_manual: Object.freeze({ mode: "manual", policy: { minimumRuntimeSeconds: 0, idleDelaySeconds: null }, minimumSeconds: 600 }),
  shutdown_custom: Object.freeze({ mode: "idle_stop", policy: { minimumRuntimeSeconds: 0, idleDelaySeconds: 600 }, minimumSeconds: 1_200 }),
  shutdown_explicit_lifecycle_reconciliation: Object.freeze({ mode: "explicit_lifecycle", policy: { minimumRuntimeSeconds: 0, idleDelaySeconds: null }, minimumSeconds: 600 }),
  shutdown_deployment_reconciliation: Object.freeze({ mode: "deployment_reconciliation", policy: { minimumRuntimeSeconds: 0, idleDelaySeconds: 900 }, minimumSeconds: 1_500 }),
})

export const SHUTDOWN_TRIGGER_LIMITS = Object.freeze({
  maximumBillableSeconds: 14_400,
  cleanupReserveMs: 300_000,
  pollMs: 10_000,
  maximumObservations: 64,
  maximumOperations: 100,
})

const FIXED_COMPUTE_CLASS = "agent-small"
const CONFIRMATION = "CREATE-AND-DELETE-ONE-MANAGED-TARGET"

export const USER_ACTIONS = new Set([
  "start_agent_via_normal_path", "finish_agent_via_normal_provider_path", "manual_stop_via_cloud_ui",
  "keep_running_via_cloud_ui", "restart_cloud_auto_stop_reconciliation", "signed_kernel_deployment_reconciliation",
])

export function requireValue(condition, message) {
  if (!condition) throw new Error(message)
}

export function nonEmpty(value, label) {
  requireValue(typeof value === "string" && value.trim().length > 0, `${label} is unavailable`)
  return value
}

function requireLoopbackKernelUrl(value) {
  let url
  try { url = new URL(value) } catch { throw new Error("kernel URL is invalid") }
  requireValue(url.protocol === "ws:" && ["127.0.0.1", "localhost", "[::1]"].includes(url.hostname)
    && url.port && url.pathname === "/kernel" && !url.username && !url.password && !url.search && !url.hash,
  "kernel URL must be an explicit loopback IPC endpoint")
  return url.toString()
}

export function parseArguments(argv) {
  const allowed = new Set([
    "--scenario", "--kernel-url", "--region", "--compute-class", "--max-billable-seconds",
    "--confirm-one-target", "--output",
  ])
  requireValue(argv.length % 2 === 0, "invalid arguments")
  const values = new Map()
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index]
    requireValue(allowed.has(flag) && !values.has(flag) && typeof argv[index + 1] === "string"
      && argv[index + 1].length > 0, "invalid arguments")
    values.set(flag, argv[index + 1])
  }
  requireValue(values.size === allowed.size && [...allowed].every((flag) => values.has(flag)),
    "required arguments are missing")
  const scenario = values.get("--scenario")
  const descriptor = SHUTDOWN_SCENARIOS[scenario]
  requireValue(descriptor, "shutdown scenario is unsupported")
  const computeClass = values.get("--compute-class")
  requireValue(computeClass === FIXED_COMPUTE_CLASS, "only the bounded agent-small compute class is supported")
  const region = values.get("--region")
  requireValue(/^[a-z0-9][a-z0-9-]{1,31}$/.test(region), "region is invalid")
  const maxBillableSeconds = Number(values.get("--max-billable-seconds"))
  requireValue(Number.isSafeInteger(maxBillableSeconds) && maxBillableSeconds >= descriptor.minimumSeconds
    && maxBillableSeconds <= SHUTDOWN_TRIGGER_LIMITS.maximumBillableSeconds,
  "billable time bound is outside the supported limit")
  requireValue(values.get("--confirm-one-target") === CONFIRMATION,
    "explicit one-target create/delete confirmation is required")
  const output = values.get("--output")
  requireValue(isAbsolute(output), "output path must be absolute")
  return Object.freeze({
    scenario,
    descriptor,
    kernelUrl: requireLoopbackKernelUrl(values.get("--kernel-url")),
    region,
    computeClass,
    maxBillableSeconds,
    output: resolve(output),
  })
}
