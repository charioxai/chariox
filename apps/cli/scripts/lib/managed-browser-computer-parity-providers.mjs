import { createHash } from "node:crypto"

const PROVIDER_FAMILIES = Object.freeze(["codex", "opencode", "claude"])
const ACTIVE_PROVIDER_RUN_STATES = new Set(["Running", "Parked"])
const KNOWN_PROVIDER_RUN_STATES = new Set(["Starting", "Running", "Parked", "Ended"])
const OFFICIAL_PROVIDER_STATUS = "official"
const PROVIDER_STATE_EVIDENCE_SCHEMA = "chariox.browser_computer.provider_state_evidence.v1"

export const SELKIES_PROVIDER_FAMILIES = PROVIDER_FAMILIES
export const SELKIES_PROVIDER_STATE_EVIDENCE_SCHEMA = PROVIDER_STATE_EVIDENCE_SCHEMA

/**
 * Join the existing official provider harness to the public observation
 * adapter. The harness owns provider launch, prompt/tool/final-turn execution,
 * and any supported credential or provider-state transfer. This function only
 * passes its public result to the observation-only verifier below; it never
 * manufactures a transfer receipt or starts a provider itself.
 */
export async function runSelkiesProviderAcceptance({
  officialProviderHarness,
  homeClient,
  workerClient,
  requestApi,
  request,
  signal,
} = {}) {
  if (typeof officialProviderHarness !== "function") {
    fail(
      "selkies_providers_official_harness_required",
      "selkies.providers requires the existing official provider harness",
    )
  }
  let harnessResult
  try {
    harnessResult = await officialProviderHarness({
      homeClient,
      workerClient,
      requestApi,
      request,
      providers: [...PROVIDER_FAMILIES],
      signal,
    })
  } catch {
    fail(
      "selkies_providers_official_harness_failed",
      "selkies.providers official provider harness failed before returning public execution evidence",
    )
  }
  if (!harnessResult || typeof harnessResult !== "object" || Array.isArray(harnessResult)) {
    fail(
      "selkies_providers_official_harness_failed",
      "selkies.providers official provider harness returned no execution evidence",
    )
  }
  return runSelkiesProviders({
    homeClient,
    workerClient,
    requestApi,
    request: {
      ...request,
      providerRunIds: harnessResult.providerRunIds,
      officialExecutionEvidence: harnessResult.executionEvidence,
    },
    signal,
  })
}

/**
 * Verify the canonical selkies.providers observation through released public
 * request constructors. Provider processes are started by the existing
 * official provider harness; this adapter verifies the resulting worker-side
 * catalog, auth, run, and durable history evidence. The home client is the
 * Room authority. The worker client is the authenticated target for relay
 * identity and all provider observations. Provider credentials or provider
 * thread state may be transferred when the harness supports it; this adapter
 * records only observed runtime metadata and does not turn transfer into a
 * no-copy assertion.
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
  const providerRunRefs = requireProviderRunRefs(request?.providerRunIds)
  const officialExecutionEvidence = requireOfficialExecutionEvidence(request?.officialExecutionEvidence)

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
    const providerRunId = providerRunRefs[provider].current
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

    let previousRuntimeEvidence = null
    const previousProviderRunId = providerRunRefs[provider].previous
    if (previousProviderRunId) {
      const previousRuntime = responseVariant(
        await sendWithAbortSignal(
          workerClient,
          requestApi.getProviderRunRequest(previousProviderRunId),
          signal,
          `${provider} previous provider runtime`,
        ),
        "ProviderRun",
        `${provider} previous provider runtime`,
      ).provider_run
      previousRuntimeEvidence = validateProviderRuntime(
        previousRuntime,
        provider,
        accountProfile,
        binding,
        previousProviderRunId,
        { active: false },
      )
      if (previousRuntimeEvidence.agentInstanceId !== runtimeEvidence.agentInstanceId
        || previousRuntimeEvidence.ownerUserId !== runtimeEvidence.ownerUserId) {
        fail(
          "selkies_providers_target_identity_mismatch",
          `selkies.providers provider runs did not retain the same worker agent identity for ${provider}`,
        )
      }
      if (previousRuntimeEvidence.providerThreadId !== runtimeEvidence.providerThreadId) {
        fail(
          "selkies_providers_thread_continuity_required",
          `selkies.providers provider thread changed across runs for ${provider}`,
        )
      }
    }

    // The existing parity harness uses this value as a gate. It is emitted
    // only after target runtime/auth/history evidence passes; the shipped
    // catalog is support evidence, never the acceptance decision by itself.
    providers[provider] = OFFICIAL_PROVIDER_STATUS
    providerEvidence[provider] = {
      catalog: catalogEvidence,
      auth: authEvidence,
      runtime: runtimeEvidence,
      ...(previousRuntimeEvidence ? { previousRuntime: previousRuntimeEvidence } : {}),
    }
  }
  const executionEvidence = await readProviderExecutionEvidence({
    workerClient,
    requestApi,
    binding,
    providerEvidence,
    providerRunRefs,
    officialExecutionEvidence,
    signal,
  })
  const providerState = createProviderStateEvidence({
    identity,
    providerProfiles,
    providerRunRefs,
    providerEvidence,
    executionEvidence,
  })

  return {
    ...identity,
    displayBackend: "selkies",
    providers,
    providerEvidence,
    providerExecutionEvidence: executionEvidence,
    providerStateEvidence: providerState,
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

function requireProviderRunRefs(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    fail("selkies_providers_target_observation_required", "selkies.providers requires providerRunIds")
  }
  const refs = {}
  for (const provider of PROVIDER_FAMILIES) {
    const item = value[provider]
    if (!item || typeof item !== "object" || Array.isArray(item)) {
      fail(
        "selkies_providers_thread_continuity_required",
        `selkies.providers requires current and previous provider runs for ${provider}`,
      )
    }
    refs[provider] = {
      current: requireText(item.current, `providerRunIds.${provider}.current`),
      previous: requireText(item.previous, `providerRunIds.${provider}.previous`),
    }
    if (refs[provider].previous === refs[provider].current) {
      fail(
        "selkies_providers_thread_continuity_required",
        `selkies.providers requires distinct current and previous provider runs for ${provider}`,
      )
    }
  }
  return refs
}

function requireOfficialExecutionEvidence(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    fail(
      "selkies_providers_official_harness_required",
      "selkies.providers requires execution evidence from the official provider harness",
    )
  }
  return value
}

function createProviderStateEvidence({
  identity,
  providerProfiles,
  providerRunRefs,
  providerEvidence,
  executionEvidence,
}) {
  const observation = {
    target: { ...identity },
    providerProfiles: { ...providerProfiles },
    providerRunIds: Object.fromEntries(
      PROVIDER_FAMILIES.map((provider) => [provider, {
        current: providerRunRefs[provider].current,
        previous: providerRunRefs[provider].previous,
      }]),
    ),
    providerEvidence: clonePublicEvidence(providerEvidence),
    executionEvidence: clonePublicEvidence(executionEvidence),
  }
  return {
    schema: PROVIDER_STATE_EVIDENCE_SCHEMA,
    authority: "worker-provider-runtime",
    scope: "public worker runtime and durable history only",
    target: { ...identity },
    observedProviderRunIds: Object.fromEntries(
      PROVIDER_FAMILIES.map((provider) => [provider, {
        current: providerEvidence[provider].runtime.providerRunId,
        previous: providerEvidence[provider].previousRuntime?.providerRunId ?? null,
      }]),
    ),
    providerThreads: Object.fromEntries(
      PROVIDER_FAMILIES.map((provider) => [provider, {
        current: providerEvidence[provider].runtime.providerThreadId,
        previous: providerEvidence[provider].previousRuntime?.providerThreadId ?? null,
        continuity: providerEvidence[provider].previousRuntime
          ? providerEvidence[provider].runtime.providerThreadId
            === providerEvidence[provider].previousRuntime.providerThreadId
          : false,
      }]),
    ),
    persistedHistory: Object.fromEntries(
      PROVIDER_FAMILIES.map((provider) => [provider, executionEvidence[provider].outputObserved === true]),
    ),
    externalProviderImportsObserved: Object.fromEntries(
      PROVIDER_FAMILIES.map((provider) => [provider, providerEvidence[provider].runtime.externalProviderImportObserved]),
    ),
    observationDigest: selkiesProviderObservationDigest(observation),
  }
}

export function selkiesProviderObservationDigest(observation) {
  const canonical = {
    target: normalizeTarget(observation?.target),
    providerProfiles: normalizeProviderMap(observation?.providerProfiles),
    providerRunIds: normalizeProviderRunRefs(observation?.providerRunIds),
    providerEvidence: normalizeProviderEvidence(observation?.providerEvidence),
    executionEvidence: normalizeExecutionEvidence(observation?.executionEvidence),
  }
  return createHash("sha256").update(JSON.stringify(canonical), "utf8").digest("hex")
}

function clonePublicEvidence(value) {
  return JSON.parse(JSON.stringify(value))
}

function normalizeTarget(target) {
  return {
    kernelId: requireDigestText(target?.kernelId),
    machineId: requireDigestText(target?.machineId),
    roomId: requireDigestText(target?.roomId),
    environmentId: requireDigestText(target?.environmentId),
  }
}

function normalizeProviderMap(value) {
  return Object.fromEntries(PROVIDER_FAMILIES.map((provider) => [provider, requireDigestText(value?.[provider])]))
}

function normalizeProviderRunRefs(value) {
  return Object.fromEntries(PROVIDER_FAMILIES.map((provider) => {
    const item = value?.[provider]
    if (hasText(item)) return [provider, { current: requireDigestText(item), previous: null }]
    return [provider, {
      current: requireDigestText(item?.current),
      previous: item?.previous == null ? null : requireDigestText(item.previous),
    }]
  }))
}

function normalizeProviderEvidence(value) {
  return Object.fromEntries(PROVIDER_FAMILIES.map((provider) => {
    const item = value?.[provider]
    return [provider, {
      catalog: {
        provider,
        providerCount: requireDigestNumber(item?.catalog?.providerCount),
        connectedCount: requireDigestNumber(item?.catalog?.connectedCount),
        hasDefault: item?.catalog?.hasDefault === true,
      },
      auth: {
        provider: requireDigestText(item?.auth?.provider),
        accountProfile: requireDigestText(item?.auth?.accountProfile),
        authState: requireDigestText(item?.auth?.authState),
      },
      runtime: normalizeRuntimeEvidence(item?.runtime),
      previousRuntime: normalizeRuntimeEvidence(item?.previousRuntime),
    }]
  }))
}

function normalizeRuntimeEvidence(value) {
  return {
    providerRunId: requireDigestText(value?.providerRunId),
    agentInstanceId: requireDigestText(value?.agentInstanceId),
    ownerUserId: requireDigestText(value?.ownerUserId),
    provider: requireDigestText(value?.provider),
    accountProfile: requireDigestText(value?.accountProfile),
    state: requireDigestText(value?.state),
    endpointMode: requireDigestText(value?.endpointMode),
    clientInterface: requireDigestText(value?.clientInterface),
    providerThreadId: requireDigestText(value?.providerThreadId),
    processObserved: value?.processObserved === true,
    externalProviderImportObserved: value?.externalProviderImportObserved === true,
  }
}

function normalizeExecutionEvidence(value) {
  return Object.fromEntries(PROVIDER_FAMILIES.map((provider) => {
    const item = value?.[provider]
    return [provider, {
      providerRunId: requireDigestText(item?.providerRunId),
      previousProviderRunId: requireDigestText(item?.previousProviderRunId),
      agentInstanceId: requireDigestText(item?.agentInstanceId),
      providerThreadId: requireDigestText(item?.providerThreadId),
      turnId: requireDigestText(item?.turnId),
      promptId: requireDigestText(item?.promptId),
      toolCallId: requireDigestText(item?.toolCallId),
      finalTurnId: requireDigestText(item?.finalTurnId),
      lifecycle: requireDigestText(item?.lifecycle),
      completedAtMs: requireDigestNumber(item?.completedAtMs),
      outputObserved: item?.outputObserved === true,
      roundTripVerified: item?.roundTripVerified === true,
    }]
  }))
}

function requireDigestText(value) {
  if (!hasText(value)) throw new TypeError("digest evidence text is required")
  return value.trim()
}

function requireDigestNumber(value) {
  if (!Number.isSafeInteger(value) || value < 0) throw new TypeError("digest evidence number is required")
  return value
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
  providerRunRefs,
  officialExecutionEvidence,
  signal,
}) {
  const agentInstanceIds = new Set(PROVIDER_FAMILIES.map(
    (provider) => providerEvidence[provider].runtime.agentInstanceId,
  ))
  const ownerUserIds = new Set(PROVIDER_FAMILIES.map(
    (provider) => providerEvidence[provider].runtime.ownerUserId,
  ))
  if (agentInstanceIds.size !== 1 || ownerUserIds.size !== 1) {
    fail(
      "selkies_providers_target_identity_mismatch",
      "selkies.providers provider runs did not share one authenticated worker agent identity",
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
    const providerRunId = providerRunRefs[provider].current
    const harnessEvidence = validateOfficialExecutionEvidence(
      officialExecutionEvidence[provider],
      provider,
      providerRunRefs[provider],
    )
    const turn = findCompletedProviderTurn(
      agent.turns,
      binding.roomId,
      agentInstanceId,
      provider,
      providerRunId,
    )
    if (!turn) {
      fail(
        "selkies_providers_execution_evidence_required",
        `selkies.providers requires a completed worker prompt/tool/final turn for ${provider}`,
      )
    }
    const providerThreadId = providerEvidence[provider].runtime.providerThreadId
    assertHistoryThreadBinding(turn, providerThreadId, provider)
    const previousProviderRunId = providerRunRefs[provider].previous
    const previousTurn = findCompletedProviderTurn(
      agent.turns,
      binding.roomId,
      agentInstanceId,
      provider,
      previousProviderRunId,
    )
    if (!previousTurn) {
      fail(
        "selkies_providers_thread_continuity_required",
        `selkies.providers requires durable history for the previous ${provider} provider run`,
      )
    }
    assertHistoryThreadBinding(previousTurn, providerEvidence[provider].previousRuntime.providerThreadId, provider)
    result[provider] = {
      providerRunId,
      agentInstanceId,
      providerThreadId,
      turnId: requireText(turn.turn_id, `SessionHistoryOutline.${provider}.turn_id`),
      lifecycle: turn.lifecycle,
      completedAtMs: turn.completed_at_ms,
      outputObserved: true,
      ...(previousProviderRunId ? { previousProviderRunId } : {}),
      promptId: harnessEvidence.promptId,
      toolCallId: harnessEvidence.toolCallId,
      finalTurnId: harnessEvidence.finalTurnId,
      roundTripVerified: true,
    }
    if (harnessEvidence.finalTurnId !== result[provider].turnId) {
      fail(
        "selkies_providers_execution_evidence_required",
        `selkies.providers official ${provider} final turn did not match durable worker history`,
      )
    }
  }
  return result
}

function validateOfficialExecutionEvidence(value, provider, providerRunRefs) {
  const prompt = value?.prompt
  const tool = value?.tool
  const final = value?.final
  if (!value || typeof value !== "object" || Array.isArray(value)
    || providerFamily(value.provider) !== provider
    || value.providerRunId !== providerRunRefs.current
    || value.previousProviderRunId !== providerRunRefs.previous
    || !prompt || prompt.submitted !== true || !hasText(prompt.id)
    || !tool || tool.observed !== true || !hasText(tool.id)
    || !final || final.observed !== true || !hasText(final.turnId)) {
    fail(
      "selkies_providers_execution_evidence_required",
      `selkies.providers requires an official ${provider} prompt/tool/final round trip`,
    )
  }
  return {
    promptId: prompt.id.trim(),
    toolCallId: tool.id.trim(),
    finalTurnId: final.turnId.trim(),
  }
}

function findCompletedProviderTurn(turns, sessionId, agentInstanceId, provider, providerRunId) {
  return turns.find((candidate) => candidate?.lifecycle === "completed"
    && candidate.external_provider === provider
    && Number.isSafeInteger(candidate.completed_at_ms)
    && candidate.completed_at_ms >= candidate.started_at_ms
    && candidate.user_prompt?.entry?.session_id === sessionId
    && candidate.user_prompt.entry.agent_id === agentInstanceId
    && candidate.user_prompt.entry.kind === "user_prompt"
    && hasText(candidate.user_prompt.entry.text)
    && historyPageEntries(candidate).some((pageEntry) => pageEntry?.entry?.session_id === sessionId
      && pageEntry.entry.agent_id === agentInstanceId
      && pageEntry.entry.provider_run_id === providerRunId
      && pageEntry.entry.kind === "provider_output"
      && hasText(pageEntry.entry.text))
    && (candidate.blobs ?? []).some((blob) => blob?.kind === "provider_tool"
      && Number.isSafeInteger(blob.entry_count)
      && blob.entry_count > 0))
}

function assertHistoryThreadBinding(turn, providerThreadId, provider) {
  const observedProviderSessionIds = [
    turn.external_provider_session_id,
    ...historyPageEntries(turn).map((pageEntry) => pageEntry?.entry?.external_provider_session_id),
  ]
    .filter(hasText)
  if (!observedProviderSessionIds.includes(providerThreadId)) {
    fail(
      "selkies_providers_thread_continuity_required",
      `selkies.providers durable history did not retain the provider thread for ${provider}`,
    )
  }
}

function historyPageEntries(turn) {
  return [turn?.user_prompt, ...(turn?.entries ?? []), turn?.summary]
    .filter(Boolean)
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

function validateProviderRuntime(runtime, provider, accountProfile, binding, providerRunId, { active = true } = {}) {
  if (!runtime || typeof runtime !== "object"
    || runtime.id !== providerRunId
    || runtime.session_id !== binding.roomId
    || providerFamily(runtime.adapter_key) !== provider
    || providerFamily(runtime.provider) !== provider
    || runtime.account_profile !== accountProfile
    || !hasText(runtime.owner_user_id)
    || (active && !ACTIVE_PROVIDER_RUN_STATES.has(runtime.state))
    || (!KNOWN_PROVIDER_RUN_STATES.has(runtime.state))
    || !hasText(runtime.agent_instance_id)
    || runtime.endpoint_mode !== "managed"
    || runtime.client_interface !== "chariox"
    || !hasText(runtime.process_label)) {
    fail(
      "selkies_providers_authenticated_runtime_required",
      `selkies.providers requires an official authenticated target provider runtime observation for ${provider}`,
    )
  }
  const providerThreadId = providerThreadIdFromRuntime(runtime, provider)
  validateExternalProviderImport(runtime.external_provider_import, provider, accountProfile)
  return {
    providerRunId: runtime.id,
    agentInstanceId: runtime.agent_instance_id,
    ownerUserId: runtime.owner_user_id,
    provider: runtime.provider,
    accountProfile: runtime.account_profile,
    state: runtime.state,
    endpointMode: runtime.endpoint_mode,
    clientInterface: runtime.client_interface,
    providerThreadId,
    processObserved: true,
    externalProviderImportObserved: runtime.external_provider_import != null,
  }
}

function providerThreadIdFromRuntime(runtime, provider) {
  const candidates = [
    runtime.provider_session_id,
    runtime.resume_state?.[`${provider}_session_id`],
    runtime.resume_state?.[`${provider}_thread_id`],
    runtime.resume_state?.opencode_session_id,
    runtime.resume_state?.codex_thread_id,
    runtime.resume_state?.claude_session_id,
  ].filter(hasText).map((value) => value.trim())
  const unique = [...new Set(candidates)]
  if (unique.length === 0) {
    fail(
      "selkies_providers_thread_continuity_required",
      `selkies.providers requires a persisted provider thread id for ${provider}`,
    )
  }
  if (unique.length > 1) {
    fail(
      "selkies_providers_thread_continuity_required",
      `selkies.providers observed conflicting provider thread ids for ${provider}`,
    )
  }
  return unique[0]
}

function validateExternalProviderImport(importMetadata, provider, accountProfile) {
  if (importMetadata == null) return
  if (!importMetadata || typeof importMetadata !== "object" || Array.isArray(importMetadata)
    || providerFamily(importMetadata.external_provider) !== provider
    || importMetadata.account_profile != null && importMetadata.account_profile !== accountProfile
    || !hasText(importMetadata.external_provider_session_id)
    || !hasText(importMetadata.external_provider_session_provider_id)) {
    fail(
      "selkies_providers_state_transfer_invalid",
      `selkies.providers received malformed supported provider transfer metadata for ${provider}`,
    )
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
