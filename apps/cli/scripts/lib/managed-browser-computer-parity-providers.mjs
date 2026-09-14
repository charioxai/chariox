import { createHash } from "node:crypto"

import { runRoomRealProviderAction } from "./live-room-real-provider.mjs"

const PROVIDER_FAMILIES = Object.freeze(["codex", "opencode", "claude"])
// These are the only provider-native resume fields in the Rust
// ProviderResumeState contract. A runtime may retain state for other provider
// families, but those fields do not identify this provider's thread.
const PROVIDER_RESUME_STATE_FIELDS = Object.freeze({
  codex: "codex_thread_id",
  opencode: "opencode_session_id",
  claude: "claude_session_id",
})
const KNOWN_PROVIDER_RUN_STATES = new Set(["Starting", "Running", "Parked", "Ended"])
const EXECUTED_PROVIDER_RUN_STATES = new Set(["Running", "Parked", "Ended"])
const OFFICIAL_PROVIDER_STATUS = "official"
const PROVIDER_STATE_EVIDENCE_SCHEMA = "chariox.browser_computer.provider_state_evidence.v1"

export const SELKIES_PROVIDER_FAMILIES = PROVIDER_FAMILIES
export const SELKIES_PROVIDER_STATE_EVIDENCE_SCHEMA = PROVIDER_STATE_EVIDENCE_SCHEMA

/**
 * Run the existing official Room provider action harness through the worker's
 * public client, then verify the resulting public runtime and durable history.
 * Credential/bootstrap transfer remains owned by the provider/kernel path;
 * this adapter does not manufacture a transfer receipt or provider evidence.
 */
export async function runSelkiesProviderAcceptance({
  homeClient,
  workerClient,
  requestApi,
  request,
  signal,
} = {}) {
  let harnessResult
  try {
    harnessResult = await runOfficialSelkiesProviderHarness({
      homeClient,
      workerClient,
      requestApi,
      request,
      signal,
    })
  } catch (error) {
    // Preserve public validation codes and cancellation. Unknown failures keep
    // their original error as cause so the live drill can diagnose the failed
    // public operation without exposing provider credentials.
    if (error?.name === "AbortError" || hasText(error?.code)) throw error
    const wrapped = new Error(
      "selkies.providers official provider harness failed before returning public execution evidence",
      { cause: error },
    )
    wrapped.code = "selkies_providers_official_harness_failed"
    throw wrapped
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
      providerPromptIds: harnessResult.providerPromptIds,
      providerProfiles: harnessResult.providerProfiles,
    },
    signal,
  })
}

async function runOfficialSelkiesProviderHarness({ workerClient, requestApi, request, signal }) {
  requireClient(workerClient, "worker provider")
  requireOfficialHarnessRequestApi(requestApi)
  const binding = requireBinding(request?.binding)
  const providerProfiles = resolveProviderProfiles(request?.providerProfiles)
  const providerRunIds = {}
  const providerPromptIds = {}

  for (const provider of PROVIDER_FAMILIES) {
    const accountProfile = providerProfiles[provider]
    const catalog = responseVariant(
      await sendWithAbortSignal(
        workerClient,
        requestApi.getProviderCatalogRequest({
          provider,
          accountProfile,
          executionLocation: { kind: "worker", kernel_ref: binding.kernelId },
        }),
        signal,
        `${provider} official provider catalog`,
      ),
      "ProviderCatalog",
      `${provider} official provider catalog`,
    ).catalog
    validateProviderCatalog(catalog, provider)
    const authStatus = responseVariant(
      await sendWithAbortSignal(
        workerClient,
        requestApi.getProviderAuthStatusRequest(provider, accountProfile),
        signal,
        `${provider} official provider auth status`,
      ),
      "ProviderAuthStatus",
      `${provider} official provider auth status`,
    ).status
    validateProviderAuthStatus(authStatus, provider, accountProfile)
    const model = selectProviderModel(catalog, provider, request?.providerModels?.[provider])
    const publicClient = {
      send: (publicRequest) => sendWithAbortSignal(
        workerClient,
        publicRequest,
        signal,
        `${provider} official provider request`,
      ),
    }
    const action = await runRoomRealProviderAction({
      client: publicClient,
      requests: requestApi,
      sessionId: binding.roomId,
      sliceId: hasText(request?.sliceId) ? request.sliceId.trim() : undefined,
      workspace: hasText(request?.workspaceId) ? request.workspaceId.trim() : undefined,
      options: {
        provider,
        model,
        mode: "computer",
        accountProfile,
        importFirst: false,
      },
      waitFor: (probe, timeoutMs, description) => waitForPublicProbe(
        probe,
        timeoutMs,
        description,
        signal,
      ),
      withTimeout: (operation, timeoutMs, description) => withPublicTimeout(
        operation,
        timeoutMs,
        description,
        signal,
      ),
      checkpoint: async () => {},
    })
    const promptId = requireText(action.settlement?.promptId, `${provider} official prompt id`)
    const turn = await readPromptTurn({
      workerClient: publicClient,
      requestApi,
      sessionId: binding.roomId,
      agentId: requireText(action.agentId, `${provider} official agent id`),
      promptId,
      signal,
    })
    const providerRunId = requireText(
      turn.user_prompt?.entry?.provider_run_id,
      `${provider} official provider run id`,
    )
    providerRunIds[provider] = { current: providerRunId, previous: null }
    providerPromptIds[provider] = promptId
  }

  return { providerRunIds, providerPromptIds, providerProfiles }
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
  const providerPromptIds = requireProviderPromptIds(request?.providerPromptIds)

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
    providerPromptIds,
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
    "getSessionHistoryBlobContentRequest",
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

function resolveProviderProfiles(value) {
  return Object.fromEntries(PROVIDER_FAMILIES.map((provider) => [
    provider,
    hasText(value?.[provider]) ? value[provider].trim() : "default",
  ]))
}

function requireProviderPromptIds(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    fail(
      "selkies_providers_execution_evidence_required",
      "selkies.providers requires prompt identities returned by public SubmitPrompt",
    )
  }
  return Object.fromEntries(PROVIDER_FAMILIES.map((provider) => [
    provider,
    requireText(value[provider], `providerPromptIds.${provider}`),
  ]))
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
      previous: item.previous == null
        ? null
        : requireText(item.previous, `providerRunIds.${provider}.previous`),
    }
    if (refs[provider].previous != null && refs[provider].previous === refs[provider].current) {
      fail(
        "selkies_providers_thread_continuity_required",
        `selkies.providers requires distinct current and previous provider runs for ${provider}`,
      )
    }
  }
  return refs
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
        continuity: executionEvidence[provider].threadContinuity === true,
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
      previousRuntime: item?.previousRuntime == null
        ? null
        : normalizeRuntimeEvidence(item.previousRuntime),
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
      previousProviderRunId: item?.previousProviderRunId == null
        ? null
        : requireDigestText(item.previousProviderRunId),
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
  providerPromptIds,
  signal,
}) {
  const outlines = new Map()
  const readAgentOutline = async (agentInstanceId, provider) => {
    if (outlines.has(agentInstanceId)) return outlines.get(agentInstanceId)
    const outline = responseVariant(
      await sendWithAbortSignal(
        workerClient,
        requestApi.getSessionHistoryOutlineRequest(binding.roomId, [agentInstanceId], 20),
        signal,
        `${provider} provider execution history`,
      ),
      "SessionHistoryOutline",
      `${provider} provider execution history`,
    )
    const agent = Array.isArray(outline?.agents)
      ? outline.agents.find((candidate) => candidate?.agent_id === agentInstanceId)
      : null
    if (!agent || !Array.isArray(agent.turns)) {
      fail(
        "selkies_providers_execution_evidence_required",
        `selkies.providers requires a completed provider turn at the authenticated worker for ${provider}`,
      )
    }
    outlines.set(agentInstanceId, agent)
    return agent
  }

  const result = {}
  for (const provider of PROVIDER_FAMILIES) {
    const providerRunId = providerRunRefs[provider].current
    const agentInstanceId = providerEvidence[provider].runtime.agentInstanceId
    const agent = await readAgentOutline(agentInstanceId, provider)
    const turn = findCompletedProviderTurn(
      agent.turns,
      binding.roomId,
      agentInstanceId,
      provider,
      providerRunId,
      providerPromptIds[provider],
    )
    if (!turn) {
      fail(
        "selkies_providers_execution_evidence_required",
        `selkies.providers requires a completed worker prompt/tool/final turn for ${provider}`,
      )
    }
    const providerThreadId = providerEvidence[provider].runtime.providerThreadId
    assertHistoryThreadBinding(turn, providerThreadId, provider)
    const toolCallId = await readCompletedProviderToolCallId({
      workerClient,
      requestApi,
      binding,
      agentInstanceId,
      provider,
      providerRunId,
      turn,
      signal,
    })
    const previousProviderRunId = providerRunRefs[provider].previous
    const previousTurn = findCompletedProviderTurn(
      previousProviderRunId ? await readAgentOutline(
        providerEvidence[provider].previousRuntime.agentInstanceId,
        provider,
      ).then((previousAgent) => previousAgent.turns) : [],
      binding.roomId,
      providerEvidence[provider].previousRuntime?.agentInstanceId ?? agentInstanceId,
      provider,
      previousProviderRunId,
    )
    if (previousProviderRunId && !previousTurn) {
      fail(
        "selkies_providers_thread_continuity_required",
        `selkies.providers requires durable history for the previous ${provider} provider run`,
      )
    }
    if (previousTurn) {
      assertHistoryThreadBinding(previousTurn, providerEvidence[provider].previousRuntime.providerThreadId, provider)
      await readCompletedProviderToolCallId({
        workerClient,
        requestApi,
        binding,
        agentInstanceId: providerEvidence[provider].previousRuntime.agentInstanceId,
        provider,
        providerRunId: previousProviderRunId,
        turn: previousTurn,
        signal,
      })
    }
    result[provider] = {
      providerRunId,
      agentInstanceId,
      providerThreadId,
      turnId: requireText(turn.turn_id, `SessionHistoryOutline.${provider}.turn_id`),
      lifecycle: turn.lifecycle,
      completedAtMs: turn.completed_at_ms,
      outputObserved: true,
      previousProviderRunId,
      promptId: providerPromptIds[provider],
      toolCallId,
      finalTurnId: turn.turn_id,
      roundTripVerified: true,
      threadContinuity: true,
    }
  }
  return result
}

function findCompletedProviderTurn(
  turns,
  sessionId,
  agentInstanceId,
  provider,
  providerRunId,
  promptId = null,
) {
  return turns.find((candidate) => candidate?.lifecycle === "completed"
    && candidate.external_provider === provider
    && (promptId == null || candidate.prompt_id === promptId)
    && Number.isSafeInteger(candidate.completed_at_ms)
    && candidate.completed_at_ms >= candidate.started_at_ms
    && candidate.user_prompt?.entry?.session_id === sessionId
    && candidate.user_prompt.entry.agent_id === agentInstanceId
    && candidate.user_prompt.entry.kind === "user_prompt"
    && candidate.user_prompt.entry.provider_run_id === providerRunId
    && hasText(candidate.user_prompt.entry.text)
    && historyPageEntries(candidate).some((pageEntry) => historyEntryBelongsToTurn(
      pageEntry,
      candidate,
      sessionId,
      agentInstanceId,
      providerRunId,
      "provider_output",
    ) && hasText(pageEntry.entry.text))
  )
}

async function readCompletedProviderToolCallId({
  workerClient,
  requestApi,
  binding,
  agentInstanceId,
  provider,
  providerRunId,
  turn,
  signal,
}) {
  for (const blob of turn.blobs ?? []) {
    if (blob?.kind !== "provider_tool" || !Number.isSafeInteger(blob.entry_count) || blob.entry_count <= 0) continue
    const content = responseVariant(
      await sendWithAbortSignal(
        workerClient,
        requestApi.getSessionHistoryBlobContentRequest(binding.roomId, agentInstanceId, blob.blob_id),
        signal,
        `${provider} provider tool history`,
      ),
      "SessionHistoryBlobContent",
      `${provider} provider tool history`,
    )
    for (const pageEntry of content?.entries ?? []) {
      const entry = pageEntry?.entry
      if (!historyEntryBelongsToTurn(
        pageEntry,
        turn,
        binding.roomId,
        agentInstanceId,
        providerRunId,
        "provider_tool",
      )) continue
      const payload = parseProviderToolPayload(entry.text)
      const status = providerToolStatus(payload)
      const toolCallId = providerToolCallId(payload) ?? (hasText(entry.merge_key) ? entry.merge_key.trim() : null)
      if (toolCallId && ["completed", "complete", "succeeded", "success"].includes(status)) {
        return toolCallId
      }
    }
  }
  fail(
    "selkies_providers_execution_evidence_required",
    `selkies.providers requires a completed durable provider tool call for ${provider}`,
  )
}

function historyEntryBelongsToTurn(pageEntry, turn, sessionId, agentInstanceId, providerRunId, kind) {
  const entry = pageEntry?.entry
  return entry?.session_id === sessionId
    && entry.agent_id === agentInstanceId
    && entry.provider_run_id === providerRunId
    && entry.kind === kind
    && hasText(entry.external_provider_turn_id)
    && hasText(turn.external_provider_turn_id)
    && entry.external_provider_turn_id === turn.external_provider_turn_id
}

function parseProviderToolPayload(text) {
  if (typeof text !== "string") return null
  try {
    const value = JSON.parse(text)
    return value && typeof value === "object" && !Array.isArray(value) ? value : null
  } catch {
    return null
  }
}

function providerToolStatus(value) {
  if (!value || typeof value !== "object") return ""
  const status = value.status ?? value.state?.status
  return typeof status === "string" ? status.trim().toLowerCase() : ""
}

function providerToolCallId(value) {
  if (!value || typeof value !== "object") return null
  for (const field of ["call_id", "id", "tool_call_id"]) {
    if (hasText(value[field])) return value[field].trim()
  }
  return null
}

function requireOfficialHarnessRequestApi(requestApi) {
  for (const name of [
    "getProviderCatalogRequest",
    "getProviderAuthStatusRequest",
    "spawnAgentRequest",
    "attachToSessionRequest",
    "submitPromptRequest",
    "getSessionStateRequest",
    "listRoomEnvironmentActionHistoryRequest",
    "getSessionHistoryOutlineRequest",
  ]) {
    if (typeof requestApi?.[name] !== "function") {
      fail("selkies_providers_request_api_required", `selkies.providers official harness requires ${name}`)
    }
  }
}

function selectProviderModel(catalog, provider, requestedModel) {
  if (hasText(requestedModel)) return requestedModel.trim()
  if (hasText(catalog?.default?.[provider])) return catalog.default[provider].trim()
  const providerEntry = catalog?.all?.find((entry) => providerFamily(entry?.id) === provider)
  const modelId = providerEntry && typeof providerEntry.models === "object"
    ? Object.keys(providerEntry.models).find((id) => hasText(id))
    : null
  if (modelId) return modelId
  for (const entry of catalog?.all ?? []) {
    if (entry?.models && typeof entry.models === "object") {
      const firstModel = Object.keys(entry.models).find((id) => hasText(id))
      if (firstModel) return firstModel
    }
  }
  fail(
    "selkies_providers_catalog_required",
    `selkies.providers could not select a public model for ${provider}`,
  )
}

async function readPromptTurn({ workerClient, requestApi, sessionId, agentId, promptId, signal }) {
  const outline = responseVariant(
    await sendWithAbortSignal(
      workerClient,
      requestApi.getSessionHistoryOutlineRequest(sessionId, [agentId], 20),
      signal,
      "official provider prompt history",
    ),
    "SessionHistoryOutline",
    "official provider prompt history",
  )
  const agent = Array.isArray(outline?.agents)
    ? outline.agents.find((candidate) => candidate?.agent_id === agentId)
    : null
  const turn = agent?.turns?.find((candidate) => candidate?.prompt_id === promptId)
  if (!turn) {
    fail(
      "selkies_providers_execution_evidence_required",
      "selkies.providers official harness returned a prompt without a durable turn",
    )
  }
  return turn
}

async function waitForPublicProbe(probe, timeoutMs, description, signal) {
  const deadline = Date.now() + timeoutMs
  while (true) {
    if (signal?.aborted) throw abortError(description)
    const value = await probe()
    if (value) return value
    const remainingMs = deadline - Date.now()
    if (remainingMs <= 0) throw new Error(`${description} before the public deadline`)
    await new Promise((resolve) => setTimeout(resolve, Math.min(100, remainingMs)))
  }
}

async function withPublicTimeout(operation, timeoutMs, description, signal) {
  if (signal?.aborted) throw abortError(description)
  let timer
  let abort
  const aborted = new Promise((_, reject) => {
    abort = () => reject(abortError(description))
    signal?.addEventListener("abort", abort, { once: true })
  })
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${description} exceeded ${timeoutMs}ms`)), timeoutMs)
  })
  try {
    return await Promise.race([Promise.resolve(operation), aborted, timeout])
  } finally {
    clearTimeout(timer)
    signal?.removeEventListener("abort", abort)
  }
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
    || (active && !EXECUTED_PROVIDER_RUN_STATES.has(runtime.state))
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
  const resumeStateField = PROVIDER_RESUME_STATE_FIELDS[provider]
  // RuntimeProviderRun derives provider_session_id from this provider-native
  // field; prefer the native value when it is present and use the projected
  // field only for runtimes that omit resume_state in their public payload.
  const providerNativeThreadId = resumeStateField == null
    ? null
    : runtime.resume_state?.[resumeStateField]
  const providerThreadId = hasText(providerNativeThreadId)
    ? providerNativeThreadId.trim()
    : hasText(runtime.provider_session_id)
      ? runtime.provider_session_id.trim()
      : null
  if (providerThreadId == null) {
    fail(
      "selkies_providers_thread_continuity_required",
      `selkies.providers requires a persisted provider thread id for ${provider}`,
    )
  }
  return providerThreadId
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
