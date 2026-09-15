import {
  isSensitiveDrillKey,
  looksLikeDrillSecretValue,
} from "./drill-secrets.mjs"

export const MANAGED_BROWSER_COMPUTER_PARITY_SCHEMA = "chariox.browser_computer.managed_parity.v1"
export const MANAGED_BROWSER_COMPUTER_PARITY_MINIMUM_PROTOCOL = 322

const CLIENTS = Object.freeze(["web", "local_tui", "remote_tui"])
const PROVIDERS = Object.freeze(["codex", "opencode", "claude"])
const DEFAULT_STEP_TIMEOUT_MS = 10 * 60_000

export async function runManagedBrowserComputerParityHarness({
  config,
  transport,
  now = () => new Date(),
  signal = null,
}) {
  validateConfig(config)
  if (!transport || typeof transport.run !== "function") {
    throw new Error("managed parity transport.run is required")
  }

  const startedAt = now().toISOString()
  const steps = []
  let failure = null
  let preflight = null
  let rollback = { status: "not_run" }
  let cleanup = { clean: false, inventory: null }

  const run = async (name, input = {}, { bound = true } = {}) => {
    const request = {
      runId: config.runId,
      ...(bound ? { binding: { ...config.expected } } : {}),
      ...input,
    }
    assertSecretFree(request, `managed parity ${name} request`)
    try {
      const result = await runBoundedTransportStep(
        transport,
        name,
        request,
        config.stepTimeoutMs ?? DEFAULT_STEP_TIMEOUT_MS,
        name.startsWith("cleanup.") ? null : signal,
      )
      assertSecretFree(result, `managed parity ${name} result`)
      steps.push({ name, status: "passed", result })
      return result
    } catch (error) {
      const normalized = !name.startsWith("cleanup.") && signal?.aborted
        ? new HarnessFailure("managed_parity_interrupted", name)
        : error instanceof HarnessFailure
        ? error
        : new HarnessFailure("managed_parity_step_failed", name)
      steps.push({ name, status: "failed", code: normalized.code })
      throw normalized
    }
  }

  try {
    preflight = await run("preflight", {
      resourceCeilings: { ...config.resourceCeilings },
    }, { bound: false })
    validatePreflight(preflight, config)

    await exerciseBackend({ backend: "selkies", run, expected: config.expected, fullAcceptance: true })
    await exerciseBackend({ backend: "novnc", run, expected: config.expected, fullAcceptance: false })
    rollback = { status: "rollback_only", backend: "novnc" }
  } catch (error) {
    failure = error instanceof HarnessFailure
      ? error
      : new HarnessFailure("managed_parity_step_failed", "unknown")
  } finally {
    try {
      await run("cleanup.perform", { scope: "run_owned_resources" })
      const inventory = await run("cleanup.inspect", {
        resourceCeilings: { ...config.resourceCeilings },
      })
      validateCleanupInventory(inventory, config.resourceCeilings)
      cleanup = { clean: true, inventory }
    } catch {
      cleanup = {
        clean: false,
        inventory: steps.findLast(({ name }) => name === "cleanup.inspect")?.result ?? null,
      }
      failure = new HarnessFailure("cleanup_incomplete", "cleanup.inspect")
    }
  }

  const completedAt = now().toISOString()
  const report = {
    schema: MANAGED_BROWSER_COMPUTER_PARITY_SCHEMA,
    runId: config.runId,
    status: failure ? "failed" : "passed",
    startedAt,
    completedAt,
    source: preflight?.source ?? { ossSha: config.ossSha, cloudSha: config.cloudSha },
    signedImage: preflight?.image ?? { ...config.image, verified: false },
    protocol: preflight?.protocol ?? null,
    target: preflight?.target ?? { ...config.expected },
    identity: { ...config.expected },
    resourceCeilings: { ...config.resourceCeilings },
    clients: [...CLIENTS],
    providers: [...PROVIDERS],
    acceptanceBackend: failure ? null : "selkies",
    rollback,
    steps,
    cleanup,
    ...(failure ? { failure: { code: failure.code, step: failure.step } } : {}),
  }
  assertSecretFree(report, "managed parity report")
  return report
}

async function exerciseBackend({ backend, run, expected, fullAcceptance }) {
  const prefix = backend
  const created = await run(`${prefix}.create`, {
    displayBackend: backend === "selkies" ? null : "novnc",
    kernelOwnedDefault: backend === "selkies",
  })
  validateBoundTarget(created, expected, backend)
  requireExactCount(created.roomCount, "roomCount")
  requireExactCount(created.browserCount, "browserCount")
  requireExactCount(created.profileCount, "profileCount")

  for (const client of CLIENTS) {
    const attached = await run(`${prefix}.attach`, { client, displayBackend: backend })
    validateBoundTarget(attached, expected, backend)
    if (attached.client !== client) fail("product_client_path_mismatch", `${prefix}.attach`)
  }

  if (fullAcceptance) {
    const providers = await run(`${prefix}.providers`, { displayBackend: backend })
    validateBoundTarget(providers, expected, backend)
    validateOfficialProviders(providers.providers, `${prefix}.providers`)
    if (providers.providerStateCopied !== false) {
      fail("provider_state_copy_forbidden", `${prefix}.providers`)
    }

    const browser = await run(`${prefix}.browser`, { displayBackend: backend })
    validateBoundTarget(browser, expected, backend)
    if (browser.structuredActions !== true || !positiveInteger(browser.mutationCount) || browser.browserCount !== 1) {
      fail("browser_structured_actions_required", `${prefix}.browser`)
    }

    const computer = await run(`${prefix}.computer`, { displayBackend: backend })
    validateBoundTarget(computer, expected, backend)
    requireTrueFields(computer, ["screenshot", "pointer", "keyboard"], `${prefix}.computer`, "computer_screenshot_input_required")

    const takeover = await run(`${prefix}.takeover`, { displayBackend: backend })
    validateBoundTarget(takeover, expected, backend)
    requireTrueFields(takeover, ["overlayVisible", "takeoverCompleted", "actorAttributed"], `${prefix}.takeover`, "actor_takeover_required")

    const persistence = await run(`${prefix}.persistence`, { displayBackend: backend })
    validateBoundTarget(persistence, expected, backend)
    requireTrueFields(persistence, ["saved", "restarted", "sameRoom", "sameEnvironment", "sameProfile"], `${prefix}.persistence`, "persistence_restart_required")

    const vault = await run(`${prefix}.vault`, { displayBackend: backend, fixture: "synthetic-vault-marker-v1" })
    validateBoundTarget(vault, expected, backend)
    requireTrueFields(vault, ["syntheticValueInserted", "valueObservedOnlyAtTarget"], `${prefix}.vault`, "synthetic_vault_check_failed")
    if (!vault.leakScan || Object.values(vault.leakScan).some((count) => count !== 0)) {
      fail("synthetic_vault_leak_detected", `${prefix}.vault`)
    }

    const git = await run(`${prefix}.git`, { displayBackend: backend })
    validateBoundTarget(git, expected, backend)
    if (git.available !== true || git.source !== "product-managed") {
      fail("git_auth_check_failed", `${prefix}.git`)
    }

    const reconnect = await run(`${prefix}.reconnect`, { displayBackend: backend, fault: "relay_disconnect" })
    validateBoundTarget(reconnect, expected, backend)
    if (reconnect.faultInjected !== true || reconnect.reconnected !== true
      || reconnect.duplicateActions !== 0 || reconnect.duplicateBrowsers !== 0) {
      fail("bounded_reconnect_required", `${prefix}.reconnect`)
    }
  } else {
    const rollback = await run(`${prefix}.rollback`, { displayBackend: backend })
    validateBoundTarget(rollback, expected, backend)
    if (rollback.rollbackReachable !== true || rollback.finalAcceptance !== false) {
      fail("novnc_cannot_claim_acceptance", `${prefix}.rollback`)
    }
  }

  const destroyed = await run(`${prefix}.destroy`, { displayBackend: backend })
  validateBoundTarget(destroyed, expected, backend)
  if (destroyed.destroyed !== true) fail("environment_destroy_required", `${prefix}.destroy`)
}

function validatePreflight(value, config) {
  requireRecord(value, "managed parity preflight")
  if (value.image?.digest !== config.image.digest
    || value.image?.signature !== config.image.signature
    || value.image?.signerFingerprint !== config.image.signerFingerprint
    || value.image?.verified !== true) {
    fail("signed_image_required", "preflight")
  }
  if (value.source?.ossSha !== config.ossSha || value.source?.cloudSha !== config.cloudSha) {
    fail("source_identity_mismatch", "preflight")
  }
  if (!Number.isSafeInteger(value.protocol?.kernel)
    || value.protocol.kernel < MANAGED_BROWSER_COMPUTER_PARITY_MINIMUM_PROTOCOL) {
    fail("kernel_protocol_322_required", "preflight")
  }
  if (!positiveInteger(value.protocol?.relay) || !text(value.protocol?.relayVersion)) {
    fail("relay_version_required", "preflight")
  }
  if (value.target?.kernelId !== config.expected.kernelId || value.target?.machineId !== config.expected.machineId) {
    fail("target_identity_mismatch", "preflight")
  }
  if (!Number.isFinite(value.target?.heartbeatAgeMs)
    || value.target.heartbeatAgeMs < 0
    || value.target.heartbeatAgeMs > config.resourceCeilings.maximumHeartbeatAgeMs) {
    fail("fresh_target_required", "preflight")
  }
  validateOfficialProviders(value.capabilities?.providers, "preflight")
  if (value.capabilities?.gitAuth !== true) fail("git_auth_capability_required", "preflight")
  if (value.capabilities?.syntheticVault !== true) fail("synthetic_vault_capability_required", "preflight")
  for (const capability of [
    "browserStructuredActions", "computerScreenshotInput", "actorTakeover", "persistence", "selkies", "novncRollback",
  ]) {
    if (value.capabilities?.[capability] !== true) fail("browser_computer_capability_required", "preflight")
  }
  validateResourcePreflight(value.resources, config.resourceCeilings)
}

function validateOfficialProviders(providers, step) {
  for (const provider of PROVIDERS) {
    if (providers?.[provider] !== "official") {
      fail("official_provider_capability_required", step)
    }
  }
}

function validateResourcePreflight(resources, ceilings) {
  if (!resources
    || !nonNegativeFinite(resources.rssBytes) || resources.rssBytes > ceilings.maximumRssBytes
    || !nonNegativeFinite(resources.cpuPercent) || resources.cpuPercent > ceilings.maximumCpuPercent
    || !nonNegativeFinite(resources.freeMemoryBytes) || resources.freeMemoryBytes < ceilings.minimumFreeMemoryBytes
    || !nonNegativeFinite(resources.freeDiskBytes) || resources.freeDiskBytes < ceilings.minimumFreeDiskBytes) {
    fail("resource_headroom_required", "preflight")
  }
}

function validateCleanupInventory(inventory, ceilings) {
  requireRecord(inventory, "managed parity cleanup inventory")
  for (const field of [
    "managedMachines", "rooms", "environments", "processes", "listeners", "containers", "profiles",
    "activeTargets", "temporaryFiles", "retainedEvidenceLeakCount",
  ]) {
    if (inventory[field] !== 0) fail("cleanup_incomplete", "cleanup.inspect")
  }
  if (!Number.isFinite(inventory.resources?.rssDeltaBytes)
    || inventory.resources.rssDeltaBytes > ceilings.maximumPostRunRssDeltaBytes
    || !Number.isFinite(inventory.resources?.diskDeltaBytes)
    || inventory.resources.diskDeltaBytes > ceilings.maximumPostRunDiskDeltaBytes) {
    fail("cleanup_incomplete", "cleanup.inspect")
  }
}

function validateBoundTarget(value, expected, backend) {
  requireRecord(value, "managed parity product result")
  for (const field of ["kernelId", "machineId", "roomId", "environmentId"]) {
    if (value[field] !== expected[field]) fail("target_identity_mismatch", "product")
  }
  if (value.displayBackend !== backend) fail("display_backend_mismatch", "product")
  for (const field of ["roomCount", "browserCount", "profileCount"]) {
    if (Object.hasOwn(value, field) && value[field] !== 1) {
      fail("single_room_environment_required", "product")
    }
  }
}

function validateConfig(config) {
  requireRecord(config, "managed parity config")
  if (!safeRunId(config.runId) || !sha(config.ossSha) || !sha(config.cloudSha)) {
    throw new Error("managed parity config requires a run id and exact OSS and Cloud SHAs")
  }
  if (!/^sha256:[0-9a-f]{64}$/.test(config.image?.digest ?? "")
    || !base64Ed25519Signature(config.image?.signature)
    || !/^sha256:[0-9a-f]{64}$/.test(config.image?.signerFingerprint ?? "")) {
    throw new Error("managed parity config requires an exact signed image identity")
  }
  for (const field of ["kernelId", "machineId", "roomId", "environmentId"]) {
    if (!text(config.expected?.[field])) throw new Error(`managed parity config expected.${field} is required`)
  }
  for (const field of [
    "maximumRssBytes", "maximumCpuPercent", "minimumFreeMemoryBytes", "minimumFreeDiskBytes",
    "maximumHeartbeatAgeMs", "maximumPostRunRssDeltaBytes", "maximumPostRunDiskDeltaBytes",
  ]) {
    if (!Number.isFinite(config.resourceCeilings?.[field]) || config.resourceCeilings[field] < 0) {
      throw new Error(`managed parity config resourceCeilings.${field} is invalid`)
    }
  }
  if (config.stepTimeoutMs !== undefined && (!positiveInteger(config.stepTimeoutMs) || config.stepTimeoutMs > 3_600_000)) {
    throw new Error("managed parity config stepTimeoutMs must be 1..3600000")
  }
  assertSecretFree(config, "managed parity config")
}

async function runBoundedTransportStep(transport, step, request, timeoutMs, externalSignal) {
  const controller = new AbortController()
  let timer
  let removeExternalAbort = () => {}
  try {
    if (externalSignal?.aborted) throw new HarnessFailure("managed_parity_interrupted", step)
    const interrupted = new Promise((_, reject) => {
      if (!externalSignal) return
      const interrupt = () => {
        reject(new HarnessFailure("managed_parity_interrupted", step))
        controller.abort()
      }
      externalSignal.addEventListener("abort", interrupt, { once: true })
      removeExternalAbort = () => externalSignal.removeEventListener("abort", interrupt)
    })
    return await Promise.race([
      Promise.resolve().then(() => transport.run(step, request, { signal: controller.signal })),
      interrupted,
      new Promise((_, reject) => {
        timer = setTimeout(() => {
          controller.abort()
          reject(new HarnessFailure("managed_parity_step_timeout", step))
        }, timeoutMs)
      }),
    ])
  } finally {
    clearTimeout(timer)
    removeExternalAbort()
  }
}

function requireTrueFields(value, fields, step, code) {
  if (fields.some((field) => value?.[field] !== true)) fail(code, step)
}

function requireExactCount(value, field) {
  if (value !== 1) fail("single_room_environment_required", field)
}

function requireRecord(value, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${label} must be an object`)
}

function assertSecretFree(value, source, key = "") {
  if (isSensitiveDrillKey(key)) throw new Error(`${source} contains forbidden sensitive field ${key}`)
  if (typeof value === "string") {
    if (looksLikeDrillSecretValue(value)) throw new Error(`${source} contains a secret-looking value`)
    return
  }
  if (!value || typeof value !== "object") return
  if (Array.isArray(value)) {
    value.forEach((item, index) => assertSecretFree(item, `${source}[${index}]`, key))
    return
  }
  for (const [childKey, childValue] of Object.entries(value)) {
    assertSecretFree(childValue, `${source}.${childKey}`, childKey)
  }
}

function positiveInteger(value) {
  return Number.isSafeInteger(value) && value > 0
}

function nonNegativeFinite(value) {
  return Number.isFinite(value) && value >= 0
}

function base64Ed25519Signature(value) {
  if (typeof value !== "string" || !/^[A-Za-z0-9+/]+={0,2}$/.test(value)) return false
  try {
    return Buffer.from(value, "base64").byteLength === 64
  } catch {
    return false
  }
}

function text(value) {
  return typeof value === "string" && value.trim().length > 0
}

function sha(value) {
  return /^[0-9a-f]{40}$/.test(value ?? "")
}

function safeRunId(value) {
  return typeof value === "string" && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value)
}

function fail(code, step) {
  throw new HarnessFailure(code, step)
}

class HarnessFailure extends Error {
  constructor(code, step) {
    super(code)
    this.name = "HarnessFailure"
    this.code = code
    this.step = step
  }
}
