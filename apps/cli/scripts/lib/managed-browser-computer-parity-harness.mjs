import {
  isSensitiveDrillKey,
  looksLikeDrillSecretValue,
} from "./drill-secrets.mjs"

export const MANAGED_BROWSER_COMPUTER_PARITY_SCHEMA = "chariox.browser_computer.managed_parity.v2"
export const MANAGED_BROWSER_COMPUTER_PARITY_MINIMUM_PROTOCOL = 322

const CLIENTS = Object.freeze(["web", "local_tui", "remote_tui"])
const PROVIDERS = Object.freeze(["codex", "opencode", "claude"])
const PRODUCT_SOURCES = Object.freeze(["kernel", "relay", "machine", "provider", "git", "vault", "browser", "computer"])
const VERSION_FIELDS = Object.freeze([...PRODUCT_SOURCES])
const CLEANUP_FIELDS = Object.freeze([
  "managedMachines", "rooms", "environments", "processes", "listeners", "containers", "images", "volumes",
  "networks", "profiles", "sessions", "agents", "workflows", "grants", "tunnels", "deployments", "dnsRecords",
  "cloudRecords", "activeTargets", "temporaryFiles", "retainedEvidenceLeakCount",
])
const DEFAULT_STEP_TIMEOUT_MS = 10 * 60_000
const DEFAULT_QUIESCE_TIMEOUT_MS = 30_000

export async function runManagedBrowserComputerParityHarness({
  config,
  transport,
  inspector,
  adapterVerification = null,
  now = () => new Date(),
  signal = null,
}) {
  validateConfig(config)
  if (!transport || typeof transport.run !== "function" || typeof transport.settle !== "function") {
    throw new Error("managed parity product adapter run and settle are required")
  }
  if (!inspector || inspector === transport || typeof inspector.run !== "function") {
    throw new Error("managed parity independent inspector is required")
  }

  const startedAt = now().toISOString()
  const steps = []
  const operationIds = new Set()
  const productSources = new Set()
  const resources = {}
  let failure = validateAdapterAuthority(config, transport, inspector, adapterVerification)
  let adapterAuthorized = failure === null
  let preflight = null
  let evidenceManifest = null
  let rollback = { status: "not_run" }
  let cleanup = { clean: false, inventory: null, adapterSettled: false }

  const run = async (name, input = {}, { bound = true } = {}) => {
    const request = { runId: config.runId, ...(bound ? { binding: { ...config.expected } } : {}), ...input }
    assertSecretFree(request, `managed parity ${name} request`)
    try {
      const result = await runBoundedTransportStep(
        transport,
        name,
        request,
        config.stepTimeoutMs ?? DEFAULT_STEP_TIMEOUT_MS,
        config.quiesceTimeoutMs ?? DEFAULT_QUIESCE_TIMEOUT_MS,
        name.startsWith("cleanup.") ? null : signal,
      )
      assertSecretFree(result, `managed parity ${name} result`)
      validateProductEvidence(result, name, operationIds, productSources)
      steps.push({ name, status: "passed", result })
      return result
    } catch (error) {
      const normalized = error instanceof HarnessFailure
        ? error
        : new HarnessFailure(!name.startsWith("cleanup.") && signal?.aborted ? "managed_parity_interrupted" : "managed_parity_step_failed", name)
      steps.push({ name, status: "failed", code: normalized.code })
      throw normalized
    }
  }

  try {
    if (failure) throw failure
    preflight = await run("preflight", { resourceCeilings: { ...config.resourceCeilings } }, { bound: false })
    validatePreflight(preflight, config)
    resources.before = preflight.resources

    await exerciseBackend({ backend: "selkies", run, expected: config.expected, fullAcceptance: true })
    resources.during = await run("resources.during", { phase: "during" })
    validateResourceSample(resources.during, "during", config.resourceCeilings)
    await exerciseBackend({ backend: "novnc", run, expected: config.expected, fullAcceptance: false })
    rollback = { status: "rollback_only", backend: "novnc" }

    evidenceManifest = await run("evidence.inspect", { scope: "all_run_evidence" })
    validateEvidenceManifest(evidenceManifest)
    for (const source of PRODUCT_SOURCES) {
      if (!productSources.has(source)) fail("product_origin_evidence_required", "evidence.inspect")
    }
  } catch (error) {
    failure = error instanceof HarnessFailure ? error : new HarnessFailure("managed_parity_step_failed", "unknown")
  } finally {
    let adapterSettled = !adapterAuthorized
    if (adapterAuthorized) {
      try {
        const settled = await settleAdapter(transport, config.runId, config.quiesceTimeoutMs ?? DEFAULT_QUIESCE_TIMEOUT_MS)
        assertSecretFree(settled, "managed parity adapter settlement")
        if (settled?.settled !== true || !operationId(settled.operationId)) fail("adapter_not_quiescent", "adapter.settle")
        adapterSettled = true
        steps.push({ name: "adapter.settle", status: "passed", result: settled })
      } catch (error) {
        const normalized = error instanceof HarnessFailure ? error : new HarnessFailure("adapter_not_quiescent", "adapter.settle")
        steps.push({ name: "adapter.settle", status: "failed", code: normalized.code })
        failure = normalized
      }
    }

    if (adapterSettled) {
      try {
        if (adapterAuthorized) await run("cleanup.perform", { scope: "run_owned_resources" })
        const inventory = await runIndependentInspection(inspector, config)
        validateCleanupInventory(inventory, config.resourceCeilings)
        resources.after = inventory.resources
        cleanup = { clean: true, inventory, adapterSettled: true }
      } catch {
        cleanup = { clean: false, inventory: null, adapterSettled: true }
        if (!failure) failure = new HarnessFailure("cleanup_incomplete", "cleanup.inspect")
      }
    }
  }

  const completedAt = now().toISOString()
  const report = {
    schema: MANAGED_BROWSER_COMPUTER_PARITY_SCHEMA,
    runId: config.runId,
    status: failure ? "failed" : "passed",
    startedAt,
    completedAt,
    adapter: adapterVerification ? { identity: adapterVerification.identity, sha256: adapterVerification.sha256 } : null,
    source: preflight?.source ?? { ossSha: config.ossSha, cloudSha: config.cloudSha },
    signedImage: preflight?.image ?? { ...config.image, verified: false },
    versions: preflight?.versions ?? null,
    protocol: preflight?.protocol ?? null,
    target: preflight?.target ?? { ...config.expected },
    identity: { ...config.expected },
    resourceCeilings: { ...config.resourceCeilings },
    resources,
    clients: [...CLIENTS],
    providers: [...PROVIDERS],
    acceptanceBackend: failure ? null : "selkies",
    rollback,
    evidenceManifest,
    steps,
    cleanup,
    ...(failure ? { failure: { code: failure.code, step: failure.step } } : {}),
  }
  assertSecretFree(report, "managed parity report")
  return report
}

function validateAdapterAuthority(config, transport, inspector, verification) {
  if (!verification || verification.verifiedBy !== "chariox-harness-loader") {
    return new HarnessFailure("reviewed_adapter_verification_required", "adapter")
  }
  if (verification.identity !== config.adapter.identity || verification.sha256 !== config.adapter.sha256) {
    return new HarnessFailure("reviewed_adapter_mismatch", "adapter")
  }
  if (transport.authority?.kind !== "product") {
    return new HarnessFailure("authoritative_product_adapter_required", "adapter")
  }
  if (transport.authority.identity !== verification.identity || transport.authority.sha256 !== verification.sha256) {
    return new HarnessFailure("reviewed_adapter_mismatch", "adapter")
  }
  if (inspector.authority?.kind !== "independent-product-inspector") {
    return new HarnessFailure("independent_cleanup_inspector_required", "adapter")
  }
  return null
}

async function exerciseBackend({ backend, run, expected, fullAcceptance }) {
  const created = await run(`${backend}.create`, { displayBackend: backend === "selkies" ? null : "novnc", kernelOwnedDefault: backend === "selkies" })
  validateBoundTarget(created, expected, backend)
  for (const field of ["roomCount", "browserCount", "profileCount"]) requireExactCount(created[field], field)

  for (const client of CLIENTS) {
    const attached = await run(`${backend}.attach`, { client, displayBackend: backend })
    validateBoundTarget(attached, expected, backend)
    if (attached.client !== client) fail("product_client_path_mismatch", `${backend}.attach`)
  }

  if (fullAcceptance) {
    const providers = await run(`${backend}.providers`, { displayBackend: backend })
    validateBoundTarget(providers, expected, backend)
    validateOfficialProviders(providers.providers, `${backend}.providers`)
    if (providers.providerStateCopied !== false) fail("provider_state_copy_forbidden", `${backend}.providers`)

    const browser = await run(`${backend}.browser`, { displayBackend: backend })
    validateBoundTarget(browser, expected, backend)
    if (browser.structuredActions !== true || !positiveInteger(browser.mutationCount) || browser.browserCount !== 1) fail("browser_structured_actions_required", `${backend}.browser`)
    requireCausal(browser.causal, ["actionOperationId", "observedMutationOperationId"], `${backend}.browser`)

    const computer = await run(`${backend}.computer`, { displayBackend: backend })
    validateBoundTarget(computer, expected, backend)
    requireTrueFields(computer, ["screenshot", "pointer", "keyboard"], `${backend}.computer`, "computer_screenshot_input_required")
    requireCausal(computer.causal, ["pointerOperationId", "keyboardOperationId", "beforeFrameDigest", "afterFrameDigest"], `${backend}.computer`, true)

    const takeover = await run(`${backend}.takeover`, { displayBackend: backend })
    validateBoundTarget(takeover, expected, backend)
    requireTrueFields(takeover, ["overlayVisible", "takeoverCompleted", "actorAttributed"], `${backend}.takeover`, "actor_takeover_required")
    requireCausal(takeover.causal, ["agentInputOperationId", "humanInputOperationId", "cancellationOperationId"], `${backend}.takeover`)
    if (takeover.causal.attributedActor !== "human") fail("causal_physical_effect_required", `${backend}.takeover`)

    const persistence = await run(`${backend}.persistence`, { displayBackend: backend })
    validateBoundTarget(persistence, expected, backend)
    requireTrueFields(persistence, ["saved", "restarted", "sameRoom", "sameEnvironment", "sameProfile"], `${backend}.persistence`, "persistence_restart_required")

    const vault = await run(`${backend}.vault`, { displayBackend: backend, fixture: "synthetic-vault-marker-v1" })
    validateBoundTarget(vault, expected, backend)
    requireTrueFields(vault, ["syntheticValueInserted", "valueObservedOnlyAtTarget"], `${backend}.vault`, "synthetic_vault_check_failed")
    if (!vault.leakScan || Object.values(vault.leakScan).some((count) => count !== 0)) fail("synthetic_vault_leak_detected", `${backend}.vault`)

    const git = await run(`${backend}.git`, { displayBackend: backend })
    validateBoundTarget(git, expected, backend)
    if (git.available !== true || git.source !== "product-managed") fail("git_auth_check_failed", `${backend}.git`)

    const reconnect = await run(`${backend}.reconnect`, { displayBackend: backend, fault: "relay_disconnect" })
    validateBoundTarget(reconnect, expected, backend)
    if (reconnect.faultInjected !== true || reconnect.reconnected !== true || reconnect.duplicateActions !== 0 || reconnect.duplicateBrowsers !== 0) fail("bounded_reconnect_required", `${backend}.reconnect`)
    requireCausal(reconnect.causal, ["disconnectOperationId", "reconnectOperationId", "postReconnectOperationId"], `${backend}.reconnect`)
  } else {
    const rollback = await run(`${backend}.rollback`, { displayBackend: backend })
    validateBoundTarget(rollback, expected, backend)
    if (rollback.rollbackReachable !== true || rollback.finalAcceptance !== false) fail("novnc_cannot_claim_acceptance", `${backend}.rollback`)
  }

  const destroyed = await run(`${backend}.destroy`, { displayBackend: backend })
  validateBoundTarget(destroyed, expected, backend)
  if (destroyed.destroyed !== true) fail("environment_destroy_required", `${backend}.destroy`)
}

function validatePreflight(value, config) {
  requireRecord(value, "managed parity preflight")
  const verification = value.image?.releaseVerification
  if (value.image?.digest !== config.image.digest || value.image?.signature !== config.image.signature
    || value.image?.signerFingerprint !== config.image.signerFingerprint || value.image?.verified !== true
    || verification?.verifier !== "chariox-managed-image-release-verifier"
    || verification?.verifierSha256 !== config.image.releaseVerifierSha256
    || verification?.exitCode !== 0
    || verification?.verifiedDigest !== config.image.digest || verification?.runningDigest !== config.image.digest
    || !operationId(verification?.operationId)) fail("signed_image_required", "preflight")
  if (value.source?.ossSha !== config.ossSha || value.source?.cloudSha !== config.cloudSha) fail("source_identity_mismatch", "preflight")
  if (!Number.isSafeInteger(value.protocol?.kernel) || value.protocol.kernel < MANAGED_BROWSER_COMPUTER_PARITY_MINIMUM_PROTOCOL) fail("kernel_protocol_322_required", "preflight")
  if (!positiveInteger(value.protocol?.relay) || !text(value.protocol?.relayVersion)) fail("relay_version_required", "preflight")
  for (const field of VERSION_FIELDS) if (value.versions?.[field] !== config.versions[field]) fail("exact_product_versions_required", "preflight")
  if (value.target?.kernelId !== config.expected.kernelId || value.target?.machineId !== config.expected.machineId) fail("target_identity_mismatch", "preflight")
  if (!Number.isFinite(value.target?.heartbeatAgeMs) || value.target.heartbeatAgeMs < 0 || value.target.heartbeatAgeMs > config.resourceCeilings.maximumHeartbeatAgeMs) fail("fresh_target_required", "preflight")
  validateOfficialProviders(value.capabilities?.providers, "preflight")
  if (value.capabilities?.gitAuth !== true) fail("git_auth_capability_required", "preflight")
  if (value.capabilities?.syntheticVault !== true) fail("synthetic_vault_capability_required", "preflight")
  for (const capability of ["browserStructuredActions", "computerScreenshotInput", "actorTakeover", "persistence", "selkies", "novncRollback"]) {
    if (value.capabilities?.[capability] !== true) fail("browser_computer_capability_required", "preflight")
  }
  validateResourceSample(value.resources, "before", config.resourceCeilings)
}

function validateOfficialProviders(providers, step) {
  for (const provider of PROVIDERS) if (providers?.[provider] !== "official") fail("official_provider_capability_required", step)
}

function validateResourceSample(resources, phase, ceilings) {
  if (resources?.phase !== phase || !operationId(resources?.operationId)
    || !nonNegativeFinite(resources.rssBytes) || resources.rssBytes > ceilings.maximumRssBytes
    || !nonNegativeFinite(resources.cpuPercent) || resources.cpuPercent > ceilings.maximumCpuPercent
    || !nonNegativeFinite(resources.freeMemoryBytes) || resources.freeMemoryBytes < ceilings.minimumFreeMemoryBytes
    || !nonNegativeFinite(resources.freeDiskBytes) || resources.freeDiskBytes < ceilings.minimumFreeDiskBytes) fail("resource_headroom_required", phase)
}

function validateEvidenceManifest(manifest) {
  if (manifest?.enumerationComplete !== true || !Array.isArray(manifest.files)
    || manifest.enumeratedFileCount !== manifest.files.length || manifest.files.length === 0) fail("evidence_inventory_incomplete", "evidence.inspect")
  const paths = new Set()
  for (const file of manifest.files) {
    if (!safeRelativePath(file?.relativePath) || paths.has(file.relativePath) || !digest(file.sha256)
      || !nonNegativeFinite(file.sizeBytes) || file.scan?.completed !== true || file.scan?.forbiddenMatches !== 0) fail("evidence_inventory_incomplete", "evidence.inspect")
    paths.add(file.relativePath)
  }
}

function validateCleanupInventory(inventory, ceilings) {
  requireRecord(inventory, "managed parity cleanup inventory")
  if (inventory.enumerated !== true || !operationId(inventory.operationId)) fail("cleanup_incomplete", "cleanup.inspect")
  for (const field of CLEANUP_FIELDS) if (!Object.hasOwn(inventory, field) || inventory[field] !== 0) fail("cleanup_incomplete", "cleanup.inspect")
  if (inventory.resources?.phase !== "after" || !operationId(inventory.resources?.operationId)
    || !Number.isFinite(inventory.resources?.rssDeltaBytes) || inventory.resources.rssDeltaBytes > ceilings.maximumPostRunRssDeltaBytes
    || !Number.isFinite(inventory.resources?.diskDeltaBytes) || inventory.resources.diskDeltaBytes > ceilings.maximumPostRunDiskDeltaBytes) fail("cleanup_incomplete", "cleanup.inspect")
}

function validateProductEvidence(value, step, operationIds, sources) {
  requireRecord(value, "managed parity product result")
  if (!Array.isArray(value.evidence) || value.evidence.length === 0) fail("product_origin_evidence_required", step)
  for (const item of value.evidence) {
    if (item?.origin !== "chariox-product" || !PRODUCT_SOURCES.includes(item.source) || !operationId(item.operationId) || operationIds.has(item.operationId)) fail("product_origin_evidence_required", step)
    if (requiresPhysicalEffect(step) && item.physicalEffect !== true) fail("physical_effect_evidence_required", step)
    operationIds.add(item.operationId)
    sources.add(item.source)
  }
}

function requiresPhysicalEffect(step) {
  return /\.(create|browser|computer|takeover|persistence|vault|git|reconnect|destroy)$/.test(step) || step === "cleanup.perform"
}

function requireCausal(causal, fields, step, frameDigests = false) {
  if (causal?.physicalEffect !== true || fields.some((field) => !operationId(causal?.[field]) && !(frameDigests && digest(causal?.[field])))) fail("causal_physical_effect_required", step)
  const ids = fields.filter((field) => field.endsWith("OperationId")).map((field) => causal[field])
  if (new Set(ids).size !== ids.length) fail("causal_physical_effect_required", step)
  if (frameDigests && causal.beforeFrameDigest === causal.afterFrameDigest) fail("causal_physical_effect_required", step)
}

function validateBoundTarget(value, expected, backend) {
  requireRecord(value, "managed parity product result")
  for (const field of ["kernelId", "machineId", "roomId", "environmentId"]) if (value[field] !== expected[field]) fail("target_identity_mismatch", "product")
  if (value.displayBackend !== backend) fail("display_backend_mismatch", "product")
  for (const field of ["roomCount", "browserCount", "profileCount"]) if (Object.hasOwn(value, field) && value[field] !== 1) fail("single_room_environment_required", "product")
}

function validateConfig(config) {
  requireRecord(config, "managed parity config")
  if (!safeRunId(config.runId) || !sha(config.ossSha) || !sha(config.cloudSha)) throw new Error("managed parity config requires a run id and exact OSS and Cloud SHAs")
  if (!digest(config.image?.digest) || !base64Ed25519Signature(config.image?.signature) || !digest(config.image?.signerFingerprint) || !digest(config.image?.releaseVerifierSha256)) throw new Error("managed parity config requires an exact signed image and release verifier identity")
  if (!text(config.adapter?.identity) || !digest(config.adapter?.sha256)) throw new Error("managed parity config requires a pinned reviewed adapter identity and hash")
  for (const field of VERSION_FIELDS) if (!text(config.versions?.[field])) throw new Error(`managed parity config versions.${field} is required`)
  for (const field of ["kernelId", "machineId", "roomId", "environmentId"]) if (!text(config.expected?.[field])) throw new Error(`managed parity config expected.${field} is required`)
  for (const field of ["maximumRssBytes", "maximumCpuPercent", "minimumFreeMemoryBytes", "minimumFreeDiskBytes", "maximumHeartbeatAgeMs", "maximumPostRunRssDeltaBytes", "maximumPostRunDiskDeltaBytes"]) {
    if (!Number.isFinite(config.resourceCeilings?.[field]) || config.resourceCeilings[field] < 0) throw new Error(`managed parity config resourceCeilings.${field} is invalid`)
  }
  for (const field of ["stepTimeoutMs", "quiesceTimeoutMs"]) if (config[field] !== undefined && (!positiveInteger(config[field]) || config[field] > 3_600_000)) throw new Error(`managed parity config ${field} must be 1..3600000`)
  assertSecretFree(config, "managed parity config")
}

async function runBoundedTransportStep(transport, step, request, timeoutMs, quiesceTimeoutMs, externalSignal) {
  const controller = new AbortController()
  let timer
  let removeExternalAbort = () => {}
  const task = Promise.resolve().then(() => transport.run(step, request, { signal: controller.signal }))
  try {
    if (externalSignal?.aborted) throw new HarnessFailure("managed_parity_interrupted", step)
    const interrupted = new Promise((_, reject) => {
      if (!externalSignal) return
      const interrupt = () => { controller.abort(); reject(new HarnessFailure("managed_parity_interrupted", step)) }
      externalSignal.addEventListener("abort", interrupt, { once: true })
      removeExternalAbort = () => externalSignal.removeEventListener("abort", interrupt)
    })
    return await Promise.race([task, interrupted, new Promise((_, reject) => {
      timer = setTimeout(() => { controller.abort(); reject(new HarnessFailure("managed_parity_step_timeout", step)) }, timeoutMs)
    })])
  } catch (error) {
    if (error instanceof HarnessFailure && ["managed_parity_step_timeout", "managed_parity_interrupted"].includes(error.code)) {
      await awaitSettlement(task, quiesceTimeoutMs, step)
    }
    throw error
  } finally {
    clearTimeout(timer)
    removeExternalAbort()
  }
}

async function awaitSettlement(task, timeoutMs, step) {
  let timer
  try {
    await Promise.race([
      task.then(() => undefined, () => undefined),
      new Promise((_, reject) => { timer = setTimeout(() => reject(new HarnessFailure("adapter_not_quiescent", step)), timeoutMs) }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

async function settleAdapter(transport, runId, timeoutMs) {
  const controller = new AbortController()
  let timer
  try {
    return await Promise.race([
      Promise.resolve().then(() => transport.settle({ runId }, { signal: controller.signal })),
      new Promise((_, reject) => { timer = setTimeout(() => { controller.abort(); reject(new HarnessFailure("adapter_not_quiescent", "adapter.settle")) }, timeoutMs) }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

async function runIndependentInspection(inspector, config) {
  const request = { runId: config.runId, binding: { ...config.expected }, resourceCeilings: { ...config.resourceCeilings }, scope: "all_task_owned_resources" }
  assertSecretFree(request, "managed parity cleanup inspection request")
  const result = await inspector.run("cleanup.inspect", request)
  assertSecretFree(result, "managed parity cleanup inspection result")
  return result
}

function requireTrueFields(value, fields, step, code) { if (fields.some((field) => value?.[field] !== true)) fail(code, step) }
function requireExactCount(value, field) { if (value !== 1) fail("single_room_environment_required", field) }
function requireRecord(value, label) { if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${label} must be an object`) }

function assertSecretFree(value, source, key = "") {
  if (isSensitiveDrillKey(key)) throw new Error(`${source} contains forbidden sensitive field ${key}`)
  if (typeof value === "string") {
    if (looksLikeDrillSecretValue(value)) throw new Error(`${source} contains a secret-looking value`)
    return
  }
  if (!value || typeof value !== "object") return
  if (Array.isArray(value)) return value.forEach((item, index) => assertSecretFree(item, `${source}[${index}]`, key))
  for (const [childKey, childValue] of Object.entries(value)) assertSecretFree(childValue, `${source}.${childKey}`, childKey)
}

function positiveInteger(value) { return Number.isSafeInteger(value) && value > 0 }
function nonNegativeFinite(value) { return Number.isFinite(value) && value >= 0 }
function operationId(value) { return typeof value === "string" && /^[A-Za-z0-9][A-Za-z0-9._:-]{2,127}$/.test(value) }
function digest(value) { return /^sha256:[0-9a-f]{64}$/.test(value ?? "") }
function safeRelativePath(value) { return text(value) && !value.startsWith("/") && !value.split("/").includes("..") }
function base64Ed25519Signature(value) {
  if (typeof value !== "string" || !/^[A-Za-z0-9+/]+={0,2}$/.test(value)) return false
  try { return Buffer.from(value, "base64").byteLength === 64 } catch { return false }
}
function text(value) { return typeof value === "string" && value.trim().length > 0 }
function sha(value) { return /^[0-9a-f]{40}$/.test(value ?? "") }
function safeRunId(value) { return typeof value === "string" && /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value) }
function fail(code, step) { throw new HarnessFailure(code, step) }

class HarnessFailure extends Error {
  constructor(code, step) { super(code); this.name = "HarnessFailure"; this.code = code; this.step = step }
}
