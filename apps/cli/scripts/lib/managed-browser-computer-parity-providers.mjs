const PROVIDER_FAMILIES = Object.freeze(["codex", "opencode", "claude"])
const ACTIVE_PROVIDER_RUN_STATES = new Set(["Running", "Parked"])
const OFFICIAL_PROVIDER_STATUS = "official"

export const SELKIES_PROVIDER_FAMILIES = PROVIDER_FAMILIES

/**
 * Execute the canonical selkies.providers operation through released public
 * request constructors. This module deliberately does not launch a provider
 * or inspect credentials. The home client is the Room authority; the worker
 * client is the authenticated target for relay identity, provider catalog,
 * auth, runtime, and history observations. The caller must supply the
 * target's supported account/bootstrap preparation path. Acceptance follows
 * the end-to-end plan's managed-provider and credential-transfer contract: a
 * supported transfer is allowed, while worker-authenticated kernel-managed
 * runtime and provider-native execution history—not an inferred file-copy or
 * no-copy boolean—are the public proof.
 */
export async function runSelkiesProviders({
  homeClient,
  workerClient,
  requestApi,
  request,
  signal,
} = {}) {
  requireClient(homeClient, "home Room")
  requireClient(workerClient, "worker provider")
  requireRequestApi(requestApi)

  const binding = requireBinding(request?.binding)
  if (request?.displayBackend !== "selkies") {
    fail("selkies_providers_requires_selkies", "selkies.providers requires the selkies display backend")
  }
  const providerProfiles = requireProviderMap(request?.providerProfiles, "providerProfiles")
  const providerRunIds = requireProviderMap(request?.providerRunIds, "providerRunIds")

  const identity = await readTargetBinding({ homeClient, workerClient, requestApi, binding, signal })
  const commandCatalogs = responseVariant(
    await sendWithAbortSignal(
      workerClient,
      requestApi.getProviderCommandCatalogsRequest(),
      signal,
      "selkies.providers command catalogs",
    ),
    "ProviderCommandCatalogs",
    "selkies.providers command catalogs",
  ).catalogs
  validateShippedCommandCatalogs(commandCatalogs)

  const providers = {}
  const providerEvidence = {}
  for (const provider of PROVIDER_FAMILIES) {
    const accountProfile = providerProfiles[provider]
    const providerRunId = providerRunIds[provider]
    const catalog = responseVariant(
      await sendWithAbortSignal(
        workerClient,
        requestApi.getProviderCatalogRequest({
          provider,
          accountProfile,
          executionLocation: { kind: "worker", kernel_ref: binding.kernelId },
        }),
        signal,
        `${provider} provider catalog`,
      ),
      "ProviderCatalog",
      `${provider} provider catalog`,
    ).catalog
    const catalogEvidence = validateProviderCatalog(catalog, provider)

    const authStatus = responseVariant(
      await sendWithAbortSignal(
        workerClient,
        requestApi.getProviderAuthStatusRequest(provider, accountProfile),
        signal,
        `${provider} provider auth status`,
      ),
      "ProviderAuthStatus",
      `${provider} provider auth status`,
    ).status
    const authEvidence = validateProviderAuthStatus(authStatus, provider, accountProfile)

    const runtime = responseVariant(
      await sendWithAbortSignal(
        workerClient,
        requestApi.getProviderRunRequest(providerRunId),
        signal,
        `${provider} provider runtime`,
      ),
      "ProviderRun",
      `${provider} provider runtime`,
    ).provider_run
    const runtimeEvidence = validateProviderRuntime(runtime, provider, accountProfile, binding, providerRunId)

    // The shipped catalog is support evidence, never the acceptance decision
    // by itself. The status is emitted only after target auth/runtime checks;
    // completed worker history is required below before returning success.
    providers[provider] = OFFICIAL_PROVIDER_STATUS
    providerEvidence[provider] = {
      catalog: catalogEvidence,
      auth: authEvidence,
      runtime: runtimeEvidence,
    }
  }
  const executionEvidence = await readProviderExecutionEvidence({
    workerClient,
    requestApi,
    binding,
    providerEvidence,
    providerRunIds,
    signal,
  })

  return {
    ...identity,
    displayBackend: "selkies",
    providers,
    providerEvidence,
    providerExecutionEvidence: executionEvidence,
  }
}

function requireRequestApi(requestApi) {
  if (!requestApi || typeof requestApi !== "object") {
    fail("selkies_providers_request_api_required", "selkies.providers requires released request constructors")
  }
  for (const name of [
    "relayStatusRequest",
    "getRoomEnvironmentStateRequest",
    "getProviderCommandCatalogsRequest",
    "getProviderCatalogRequest",
    "getProviderAuthStatusRequest",
    "getProviderRunRequest",
    "getSessionHistoryOutlineRequest",
  ]) {
    if (typeof requestApi[name] !== "function") {
      fail("selkies_providers_request_api_required", `selkies.providers requires ${name}`)
    }
  }
}

function requireClient(client, label) {
  if (!client || typeof client.send !== "function") {
    fail("selkies_providers_public_client_required", `selkies.providers requires a public ${label} client`)
  }
}

function requireBinding(binding) {
  if (!binding || typeof binding !== "object") {
    fail("selkies_providers_binding_required", "selkies.providers requires a target binding")
  }
  return {
    kernelId: requireText(binding.kernelId, "binding.kernelId"),
    machineId: requireText(binding.machineId, "binding.machineId"),
    roomId: requireText(binding.roomId, "binding.roomId"),
    environmentId: requireText(binding.environmentId, "binding.environmentId"),
  }
}

function requireProviderMap(value, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    fail("selkies_providers_target_observation_required", `selkies.providers requires ${label}`)
  }
  const map = {}
  for (const provider of PROVIDER_FAMILIES) {
    map[provider] = requireText(value[provider], `${label}.${provider}`)
  }
  return map
}

async function readTargetBinding({ homeClient, workerClient, requestApi, binding, signal }) {
  const relayStatus = responseVariant(
    await sendWithAbortSignal(
      workerClient,
      requestApi.relayStatusRequest(),
      signal,
      "selkies.providers relay identity",
    ),
    "RelayStatus",
    "selkies.providers relay identity",
  ).status
  if (relayStatus?.configured !== true
    || relayStatus.connected !== true
    || relayStatus.relay_token_configured !== true) {
    fail("selkies_providers_target_not_connected", "selkies.providers requires a connected target relay")
  }
  const targetKernelId = requireText(relayStatus.daemon_id, "RelayStatus.status.daemon_id")
  const targetMachineId = requireText(relayStatus.machine_id, "RelayStatus.status.machine_id")
  if (targetKernelId !== binding.kernelId || targetMachineId !== binding.machineId) {
    fail("selkies_providers_target_identity_mismatch", "selkies.providers target identity did not match its binding")
  }

  const environment = responseVariant(
    await sendWithAbortSignal(
      homeClient,
      requestApi.getRoomEnvironmentStateRequest(binding.roomId),
      signal,
      "selkies.providers Room identity",
    ),
    "RoomEnvironmentState",
    "selkies.providers Room identity",
  ).environment
  const roomId = requireText(environment?.session_id, "RoomEnvironmentState.environment.session_id")
  const environmentId = requireText(environment?.environment_id, "RoomEnvironmentState.environment.environment_id")
  if (roomId !== binding.roomId || environmentId !== binding.environmentId) {
    fail("selkies_providers_target_identity_mismatch", "selkies.providers Room identity did not match its binding")
  }
  return {
    kernelId: targetKernelId,
    machineId: targetMachineId,
    roomId,
    environmentId,
  }
}

async function readProviderExecutionEvidence({
  workerClient,
  requestApi,
  binding,
  providerEvidence,
  providerRunIds,
  signal,
}) {
  const agentInstanceIds = new Set(PROVIDER_FAMILIES.map(
    (provider) => providerEvidence[provider].runtime.agentInstanceId,
  ))
  if (agentInstanceIds.size !== 1) {
    fail(
      "selkies_providers_target_identity_mismatch",
      "selkies.providers provider runs did not share one authenticated target agent",
    )
  }
  const agentInstanceId = [...agentInstanceIds][0]
  const outline = responseVariant(
    await sendWithAbortSignal(
      workerClient,
      requestApi.getSessionHistoryOutlineRequest(binding.roomId, [agentInstanceId], 20),
      signal,
      "selkies.providers provider execution history",
    ),
    "SessionHistoryOutline",
    "selkies.providers provider execution history",
  )
  const agent = Array.isArray(outline?.agents)
    ? outline.agents.find((candidate) => candidate?.agent_id === agentInstanceId)
    : null
  if (!agent || !Array.isArray(agent.turns)) {
    fail(
      "selkies_providers_execution_evidence_required",
      "selkies.providers requires a completed provider turn observed at the authenticated worker",
    )
  }
  const result = {}
  for (const provider of PROVIDER_FAMILIES) {
    const providerRunId = providerRunIds[provider]
    const turn = agent.turns.find((candidate) => candidate?.lifecycle === "completed"
      && Number.isSafeInteger(candidate.completed_at_ms)
      && candidate.completed_at_ms >= candidate.started_at_ms
      && [...(candidate.entries ?? []), candidate.summary]
        .filter(Boolean)
        .some((pageEntry) => pageEntry?.entry?.session_id === binding.roomId
          && pageEntry.entry.agent_id === agentInstanceId
          && pageEntry.entry.provider_run_id === providerRunId
          && pageEntry.entry.kind === "provider_output"
          && hasText(pageEntry.entry.text)))
    if (!turn) {
      fail(
        "selkies_providers_execution_evidence_required",
        `selkies.providers requires a completed worker provider turn for ${provider}`,
      )
    }
    const outputEntry = [...(turn.entries ?? []), turn.summary]
      .filter(Boolean)
      .map((pageEntry) => pageEntry?.entry)
      .find((entry) => entry?.session_id === binding.roomId
        && entry.agent_id === agentInstanceId
        && entry.provider_run_id === providerRunId
        && entry.kind === "provider_output"
        && hasText(entry.text))
    const providerSessionId = providerEvidence[provider].runtime.providerSessionId
    for (const observedSessionId of [turn.external_provider_session_id, outputEntry?.external_provider_session_id]) {
      if (observedSessionId != null && observedSessionId !== providerSessionId) {
        fail(
          "selkies_providers_target_identity_mismatch",
          `selkies.providers observed a provider thread mismatch for ${provider}`,
        )
      }
    }
    result[provider] = {
      providerRunId,
      agentInstanceId,
      providerSessionId,
      turnId: requireText(turn.turn_id, `SessionHistoryOutline.${provider}.turn_id`),
      lifecycle: turn.lifecycle,
      completedAtMs: turn.completed_at_ms,
      outputObserved: true,
    }
  }
  return result
}

function validateShippedCommandCatalogs(catalogs) {
  if (!catalogs || typeof catalogs !== "object" || Array.isArray(catalogs)) {
    fail("selkies_providers_command_catalog_required", "selkies.providers returned no provider command catalogs")
  }
  for (const provider of PROVIDER_FAMILIES) {
    const catalog = catalogs[provider]
    if (!catalog || typeof catalog !== "object"
      || catalog.provider !== provider
      || catalog.source !== "shipped"
      || !Array.isArray(catalog.commands)) {
      fail(
        "selkies_providers_command_catalog_required",
        `selkies.providers has no shipped command contract for ${provider}`,
      )
    }
  }
}

function validateProviderCatalog(catalog, provider) {
  if (!catalog || typeof catalog !== "object"
    || !Array.isArray(catalog.all)
    || catalog.all.length === 0
    || !catalog.default
    || typeof catalog.default !== "object"
    || Array.isArray(catalog.default)
    || !Array.isArray(catalog.connected)
    || catalog.connected.length === 0) {
    fail(
      "selkies_providers_catalog_required",
      `selkies.providers returned no usable public catalog for ${provider}`,
    )
  }
  if (!catalog.all.every((entry) => entry && typeof entry === "object" && hasText(entry.id))) {
    fail("selkies_providers_catalog_required", `selkies.providers returned a malformed public catalog for ${provider}`)
  }
  return {
    provider,
    providerCount: catalog.all.length,
    connectedCount: catalog.connected.length,
    hasDefault: Object.keys(catalog.default).length > 0,
  }
}

function validateProviderAuthStatus(status, provider, accountProfile) {
  if (!status || typeof status !== "object"
    || providerFamily(status.provider) !== provider
    || status.auth_state !== "authenticated"
    || status.account_profile !== accountProfile) {
    fail(
      "selkies_providers_authenticated_runtime_required",
      `selkies.providers requires authenticated provider runtime observation for ${provider}`,
    )
  }
  return {
    provider: status.provider,
    accountProfile: status.account_profile,
    authState: status.auth_state,
  }
}

function validateProviderRuntime(runtime, provider, accountProfile, binding, providerRunId) {
  const supportedClientInterfaces = new Set(["chariox", "native_tui"])
  if (!runtime || typeof runtime !== "object"
    || runtime.id !== providerRunId
    || runtime.session_id !== binding.roomId
    || providerFamily(runtime.provider) !== provider
    || runtime.account_profile !== accountProfile
    || !ACTIVE_PROVIDER_RUN_STATES.has(runtime.state)
    || runtime.endpoint_mode !== "Managed"
    || !hasText(runtime.agent_instance_id)
    || !hasText(runtime.provider_session_id)) {
    fail(
      "selkies_providers_authenticated_runtime_required",
      `selkies.providers requires an active target provider runtime observation for ${provider}`,
    )
  }
  if (runtime.client_interface != null && !supportedClientInterfaces.has(runtime.client_interface)) {
    fail(
      "selkies_providers_authenticated_runtime_required",
      `selkies.providers requires a supported kernel provider client interface for ${provider}`,
    )
  }
  return {
    providerRunId: runtime.id,
    agentInstanceId: runtime.agent_instance_id,
    providerSessionId: requireText(runtime.provider_session_id, `ProviderRun.${provider}.provider_session_id`),
    provider: runtime.provider,
    accountProfile: runtime.account_profile,
    state: runtime.state,
    endpointMode: runtime.endpoint_mode,
    clientInterface: runtime.client_interface ?? "chariox",
    // A process label identifies a capability/runtime slot only. It is not
    // execution evidence; the worker history observation below is the
    // acceptance signal for a completed provider turn.
    processObserved: hasText(runtime.process_label),
  }
}

function providerFamily(value) {
  if (!hasText(value)) return null
  const normalized = value.trim().toLowerCase()
  if (normalized === "codex" || normalized === "opencode") return normalized
  if (normalized === "claude" || normalized === "claude-headless" || normalized === "claude-p") return "claude"
  return null
}

async function sendWithAbortSignal(client, request, signal, step) {
  if (signal?.aborted) throw abortError(step)
  const operation = Promise.resolve().then(() => client.send(request))
  if (!signal) return operation

  let abort
  const aborted = new Promise((_, reject) => {
    abort = () => reject(abortError(step))
  })
  signal.addEventListener("abort", abort, { once: true })
  if (signal.aborted) abort()
  try {
    return await Promise.race([operation, aborted])
  } finally {
    signal.removeEventListener("abort", abort)
  }
}

function responseVariant(response, variant, step) {
  if (!response || typeof response !== "object" || !(variant in response)) {
    fail("selkies_providers_public_response_malformed", `managed parity ${step} expected ${variant}`)
  }
  return response[variant]
}

function abortError(step) {
  const error = new Error(`managed parity ${step} was aborted while the public request was in flight`)
  error.name = "AbortError"
  return error
}

function requireText(value, label) {
  if (!hasText(value)) fail("selkies_providers_public_response_malformed", `managed parity response requires ${label}`)
  return value.trim()
}

function hasText(value) {
  return typeof value === "string" && value.trim() !== ""
}

function fail(code, message) {
  const error = new Error(message)
  error.code = code
  throw error
}
