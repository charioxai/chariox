import assert from "node:assert/strict"
import { createECDH } from "node:crypto"
import { once } from "node:events"
import { createRequire } from "node:module"
import test from "node:test"
import { fileURLToPath } from "node:url"

import {
  runSelkiesProviderAcceptance,
  runSelkiesProviders,
  SELKIES_PROVIDER_STATE_EVIDENCE_SCHEMA,
} from "./managed-browser-computer-parity-providers.mjs"
import { createManagedBrowserComputerParityTransportFromPublicClient } from "./managed-browser-computer-parity-product-transport.mjs"

const kernelClientDistUrl = new URL("../../../../packages/kernel-client/dist/ipc.js", import.meta.url)
const kernelRequestsDistUrl = new URL("../../../../packages/kernel-client/dist/ipc-requests.js", import.meta.url)
const relayCryptoDistUrl = new URL("../../../../packages/kernel-client/dist/relay-crypto.js", import.meta.url)

const binding = Object.freeze({
  kernelId: "worker-kernel-1",
  machineId: "worker-machine-1",
  roomId: "room-1",
  environmentId: "environment-1",
})

const homeBinding = Object.freeze({
  kernelId: "home-kernel-1",
  machineId: "home-machine-1",
})

const providerProfiles = Object.freeze({
  codex: "codex-default",
  opencode: "opencode-default",
  claude: "claude-default",
})

const providerRunIds = Object.freeze({
  codex: Object.freeze({ previous: "provider-run-codex-before", current: "provider-run-codex" }),
  opencode: Object.freeze({ previous: "provider-run-opencode-before", current: "provider-run-opencode" }),
  claude: Object.freeze({ previous: "provider-run-claude-before", current: "provider-run-claude" }),
})

const providerThreadIds = Object.freeze({
  codex: "codex-thread-fixture",
  opencode: "opencode-thread-fixture",
  claude: "claude-thread-fixture",
})

test("selkies.providers keeps Room authority separate from worker provider observations", async () => {
  const modules = await loadPublicClientModules()
  const { LocalIpcClient } = modules.kernelClient
  const requestApi = modules.requestApi
  const homeRelay = await createControlledRelay({ requestApi, role: "home" })
  const workerRelay = await createControlledRelay({ requestApi, role: "worker" })
  const homeClient = new LocalIpcClient(homeRelay.endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: homeBinding.kernelId,
  })
  const workerClient = new LocalIpcClient(workerRelay.endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: binding.kernelId,
  })
  try {
    const result = await runSelkiesProviders({
      homeClient,
      workerClient,
      requestApi,
      request: {
        displayBackend: "selkies",
        binding,
        providerProfiles,
        providerRunIds,
        providerPromptIds: fixtureProviderPromptIds(),
      },
    })

    assert.deepEqual(result.providers, {
      codex: "official",
      opencode: "official",
      claude: "official",
    })
    assert.equal(Object.hasOwn(result, "providerStateCopied"), false)
    assert.equal(result.providerStateEvidence.schema, SELKIES_PROVIDER_STATE_EVIDENCE_SCHEMA)
    assert.equal(result.providerStateEvidence.authority, "worker-provider-runtime")
    assert.equal(result.providerStateEvidence.scope, "public worker runtime and durable history only")
    assert.deepEqual(result.providerStateEvidence.observedProviderRunIds, providerRunIds)
    assert.deepEqual(result.providerStateEvidence.providerThreads, {
      codex: { current: providerThreadIds.codex, previous: providerThreadIds.codex, continuity: true },
      opencode: { current: providerThreadIds.opencode, previous: providerThreadIds.opencode, continuity: true },
      claude: { current: providerThreadIds.claude, previous: providerThreadIds.claude, continuity: true },
    })
    assert.deepEqual(result.providerStateEvidence.persistedHistory, {
      codex: true,
      opencode: true,
      claude: true,
    })
    assert.deepEqual(result.providerStateEvidence.externalProviderImportsObserved, {
      codex: false,
      opencode: false,
      claude: false,
    })
    assert.deepEqual(result.kernelId, binding.kernelId)
    assert.deepEqual(result.machineId, binding.machineId)
    assert.deepEqual(result.roomId, binding.roomId)
    assert.deepEqual(result.environmentId, binding.environmentId)
    assert.deepEqual(homeRelay.requests, [requestApi.getRoomEnvironmentStateRequest(binding.roomId)])
    assert.deepEqual(workerRelay.requests, expectedWorkerRequests(requestApi))
    assert.equal(workerRelay.launchRequests, 0, "observation adapter must not launch a provider")
    assert.equal(workerRelay.credentialRequests, 0, "observation adapter must not read or mutate credentials")
    assert.deepEqual(Object.keys(result.providerExecutionEvidence), ["codex", "opencode", "claude"])
    assert.ok(result.providerExecutionEvidence.codex.outputObserved)
    assert.ok(result.providerExecutionEvidence.codex.roundTripVerified)
  } finally {
    await closeClientsAndRelays(homeClient, workerClient, homeRelay, workerRelay)
  }
})

test("production selkies.providers dispatch executes public prompt/tool/final rounds", async () => {
  const modules = await loadPublicClientModules()
  const { LocalIpcClient } = modules.kernelClient
  const requestApi = modules.requestApi
  const homeRelay = await createControlledRelay({ requestApi, role: "home" })
  const workerRelay = await createControlledRelay({ requestApi, role: "worker", execution: true })
  const homeClient = new LocalIpcClient(homeRelay.endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: homeBinding.kernelId,
  })
  const workerClient = new LocalIpcClient(workerRelay.endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: binding.kernelId,
  })
  try {
    const transport = createManagedBrowserComputerParityTransportFromPublicClient({
      client: workerClient,
      displayClient: homeClient,
      identityClient: workerClient,
      requestApi,
    })
    const result = await transport.run("selkies.providers", { displayBackend: "selkies", binding })
    assert.deepEqual(result.providers, { codex: "official", opencode: "official", claude: "official" })
    assert.equal(new Set(workerRelay.spawnedAgentIds).size, 3)
    assert.equal(workerRelay.submitRequests, 3)
    assert.equal(workerRelay.launchRequests, 0, "official harness uses the released SpawnAgent public path")
    assert.equal(workerRelay.requests.some((request) => "SubmitPrompt" in request), true)
    assert.equal(workerRelay.requests.some((request) => "GetSessionHistoryBlobContent" in request), true)
    assert.deepEqual(result.providerStateEvidence.persistedHistory, { codex: true, opencode: true, claude: true })
  } finally {
    await closeClientsAndRelays(homeClient, workerClient, homeRelay, workerRelay)
  }
})

test("production selkies.providers fails when public execution omits a durable tool round trip", async () => {
  const modules = await loadPublicClientModules()
  const { LocalIpcClient } = modules.kernelClient
  const requestApi = modules.requestApi
  const homeRelay = await createControlledRelay({ requestApi, role: "home" })
  const workerRelay = await createControlledRelay({ requestApi, role: "worker", execution: true, omitToolProvider: "codex" })
  const homeClient = new LocalIpcClient(homeRelay.endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: homeBinding.kernelId,
  })
  const workerClient = new LocalIpcClient(workerRelay.endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: binding.kernelId,
  })
  try {
    const transport = createManagedBrowserComputerParityTransportFromPublicClient({
      client: workerClient,
      displayClient: homeClient,
      identityClient: workerClient,
      requestApi,
    })
    await assert.rejects(
      () => transport.run("selkies.providers", { displayBackend: "selkies", binding }),
      /requires a completed durable provider tool call for codex/,
    )
    assert.equal(workerRelay.submitRequests, 3)
    assert.equal(
      workerRelay.requests.some((request) => "GetSessionHistoryBlobContent" in request),
      false,
      "missing durable tool evidence must fail closed before fabricating a blob round trip",
    )
  } finally {
    await closeClientsAndRelays(homeClient, workerClient, homeRelay, workerRelay)
  }
})

test("selkies.providers does not accept an advertised catalog without authenticated runtime proof", async () => {
  const requests = []
  const requestApi = minimalRequestApi()
  const homeClient = {
    async send(request) {
      requests.push(request)
      if ("GetRoomEnvironmentState" in request) {
        return { RoomEnvironmentState: { environment: { session_id: binding.roomId, environment_id: binding.environmentId } } }
      }
      throw new Error("home received worker-only request: " + JSON.stringify(request))
    },
  }
  const workerClient = {
    async send(request) {
      requests.push(request)
      if ("RelayStatus" in request) {
        return { RelayStatus: { status: { configured: true, connected: true, relay_token_configured: true, daemon_id: binding.kernelId, machine_id: binding.machineId } } }
      }
      if ("GetProviderCommandCatalogs" in request) {
        return { ProviderCommandCatalogs: { catalogs: shippedCatalogs() } }
      }
      if ("GetProviderCatalog" in request) {
        return { ProviderCatalog: { catalog: advertisedCatalog() } }
      }
      if ("GetProviderAuthStatus" in request) {
        return { ProviderAuthStatus: { status: { provider: request.GetProviderAuthStatus.provider, auth_state: "not_logged_in", account_profile: request.GetProviderAuthStatus.account_profile } } }
      }
      throw new Error("unexpected request: " + JSON.stringify(request))
    },
  }

  await assert.rejects(
    () => runSelkiesProviders({
      homeClient,
      workerClient,
      requestApi,
      request: {
        displayBackend: "selkies",
        binding,
        providerProfiles,
        providerRunIds,
        providerPromptIds: fixtureProviderPromptIds(),
      },
    }),
    /requires authenticated provider runtime observation/,
  )
  assert.equal(requests.filter((request) => "GetProviderRun" in request).length, 0)
})

test("selkies.providers does not treat a Starting process as official provider execution", async () => {
  const modules = await loadPublicClientModules()
  const { LocalIpcClient } = modules.kernelClient
  const requestApi = modules.requestApi
  const homeRelay = await createControlledRelay({ requestApi, role: "home" })
  const workerRelay = await createControlledRelay({ requestApi, role: "worker", providerRunState: "Starting" })
  const homeClient = new LocalIpcClient(homeRelay.endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: homeBinding.kernelId,
  })
  const workerClient = new LocalIpcClient(workerRelay.endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: binding.kernelId,
  })
  try {
    await assert.rejects(
      () => runSelkiesProviders({
        homeClient,
        workerClient,
        requestApi,
        request: {
          displayBackend: "selkies",
          binding,
          providerProfiles,
          providerRunIds,
          providerPromptIds: fixtureProviderPromptIds(),
        },
      }),
      /requires an official authenticated target provider runtime observation/,
    )
    assert.equal(workerRelay.requests.filter((request) => "GetSessionHistoryOutline" in request).length, 0)
  } finally {
    await closeClientsAndRelays(homeClient, workerClient, homeRelay, workerRelay)
  }
})

test("selkies.providers permits supported provider import metadata without claiming a no-copy proof", async () => {
  const modules = await loadPublicClientModules()
  const { LocalIpcClient } = modules.kernelClient
  const requestApi = modules.requestApi
  const homeRelay = await createControlledRelay({ requestApi, role: "home" })
  const workerRelay = await createControlledRelay({
    requestApi,
    role: "worker",
    importedProviderRunId: providerRunIds.codex.current,
  })
  const homeClient = new LocalIpcClient(homeRelay.endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: homeBinding.kernelId,
  })
  const workerClient = new LocalIpcClient(workerRelay.endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: binding.kernelId,
  })
  try {
    const result = await runSelkiesProviders({
      homeClient,
      workerClient,
      requestApi,
      request: {
        displayBackend: "selkies",
        binding,
        providerProfiles,
        providerRunIds,
        providerPromptIds: fixtureProviderPromptIds(),
      },
    })
    assert.equal(result.providerStateEvidence.externalProviderImportsObserved.codex, true)
    assert.equal(result.providerStateEvidence.providerThreads.codex.continuity, true)
    assert.equal(Object.hasOwn(result, "providerStateCopied"), false)
  } finally {
    await closeClientsAndRelays(homeClient, workerClient, homeRelay, workerRelay)
  }
})

test("selkies.providers fails closed without a public SubmitPrompt identity", async () => {
  const requests = []
  const client = { send: async (request) => { requests.push(request); throw new Error("unexpected public request") } }
  const requestApi = minimalRequestApi()

  await assert.rejects(
    () => runSelkiesProviders({
      homeClient: client,
      workerClient: client,
      requestApi,
      request: {
        displayBackend: "selkies",
        binding,
        providerProfiles,
        providerRunIds,
      },
    }),
    /requires prompt identities returned by public SubmitPrompt/,
  )
  assert.deepEqual(requests, [])
})

test("selkies.providers rejects durable history that omits one provider round trip", async () => {
  const modules = await loadPublicClientModules()
  const { LocalIpcClient } = modules.kernelClient
  const requestApi = modules.requestApi
  const homeRelay = await createControlledRelay({ requestApi, role: "home" })
  const workerRelay = await createControlledRelay({ requestApi, role: "worker", omitProvider: "claude" })
  const homeClient = new LocalIpcClient(homeRelay.endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: homeBinding.kernelId,
  })
  const workerClient = new LocalIpcClient(workerRelay.endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: binding.kernelId,
  })
  try {
    await assert.rejects(
      () => runSelkiesProviders({
        homeClient,
        workerClient,
        requestApi,
        request: {
          displayBackend: "selkies",
          binding,
          providerProfiles,
          providerRunIds,
          providerPromptIds: fixtureProviderPromptIds(),
        },
      }),
      /requires a completed worker prompt\/tool\/final turn for claude/,
    )
  } finally {
    await closeClientsAndRelays(homeClient, workerClient, homeRelay, workerRelay)
  }
})

test("selkies.providers rejects a completed output without a persisted provider tool", async () => {
  const modules = await loadPublicClientModules()
  const { LocalIpcClient } = modules.kernelClient
  const requestApi = modules.requestApi
  const homeRelay = await createControlledRelay({ requestApi, role: "home" })
  const workerRelay = await createControlledRelay({
    requestApi,
    role: "worker",
    execution: true,
    omitToolProvider: "codex",
  })
  const homeClient = new LocalIpcClient(homeRelay.endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: homeBinding.kernelId,
  })
  const workerClient = new LocalIpcClient(workerRelay.endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: binding.kernelId,
  })
  try {
    await assert.rejects(
      () => runSelkiesProviderAcceptance({
        homeClient,
        workerClient,
        requestApi,
        request: {
          displayBackend: "selkies",
          binding,
          providerProfiles,
        },
      }),
      /requires a completed durable provider tool call for codex/,
    )
  } finally {
    await closeClientsAndRelays(homeClient, workerClient, homeRelay, workerRelay)
  }
})

test("selkies.providers rejects a changed provider thread across worker runs", async () => {
  const modules = await loadPublicClientModules()
  const { LocalIpcClient } = modules.kernelClient
  const requestApi = modules.requestApi
  const homeRelay = await createControlledRelay({ requestApi, role: "home" })
  const workerRelay = await createControlledRelay({ requestApi, role: "worker", mismatchProvider: "opencode" })
  const homeClient = new LocalIpcClient(homeRelay.endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: homeBinding.kernelId,
  })
  const workerClient = new LocalIpcClient(workerRelay.endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: binding.kernelId,
  })
  try {
    await assert.rejects(
      () => runSelkiesProviders({
        homeClient,
        workerClient,
        requestApi,
        request: {
          displayBackend: "selkies",
          binding,
          providerProfiles,
          providerRunIds,
          providerPromptIds: fixtureProviderPromptIds(),
        },
      }),
      /provider thread changed across runs for opencode/,
    )
  } finally {
    await closeClientsAndRelays(homeClient, workerClient, homeRelay, workerRelay)
  }
})

async function loadPublicClientModules() {
  try {
    const [{ LocalIpcClient }, requestApi] = await Promise.all([
      import(kernelClientDistUrl.href),
      import(kernelRequestsDistUrl.href),
      import(relayCryptoDistUrl.href),
    ])
    return { kernelClient: { LocalIpcClient }, requestApi }
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error)
    throw new Error("public LocalIpcClient acceptance requires existing kernel-client dist: " + detail, { cause: error })
  }
}

async function closeClientsAndRelays(homeClient, workerClient, homeRelay, workerRelay) {
  await homeClient?.close().catch(() => {})
  await workerClient?.close().catch(() => {})
  await homeRelay?.close()
  await workerRelay?.close()
}

async function createControlledRelay({
  requestApi,
  role,
  execution = false,
  importedProviderRunId = null,
  providerRunState = "Running",
  omitProvider = null,
  omitToolProvider = null,
  mismatchProvider = null,
}) {
  const { decryptRelayPayload, encryptRelayPayload } = await import(relayCryptoDistUrl.href)
  const { WebSocketServer } = createRequire(fileURLToPath(kernelClientDistUrl))("ws")
  const server = new WebSocketServer({ port: 0 })
  await once(server, "listening")
  const address = server.address()
  assert.ok(address && typeof address === "object")
  const target = role === "home" ? homeBinding : binding
  const requests = []
  let launchRequests = 0
  let credentialRequests = 0
  let submitRequests = 0
  let serverError
  const executionState = execution ? createExecutionFixtureState({ omitToolProvider }) : null

  server.on("connection", (socket) => {
    socket.on("message", (raw) => {
      try {
        const frame = JSON.parse(raw.toString())
        if (frame.kind === "client_connect") {
          assert.equal(frame.auth_token, "operator-test-token")
          assert.deepEqual(frame.target, { daemon_id: target.kernelId, daemon_alias: null })
          const daemon = createECDH("prime256v1")
          const daemonPublicKey = daemon.generateKeys().toString("base64")
          socket.daemon = daemon
          socket.send(JSON.stringify({
            kind: "client_connected",
            target: frame.target,
            daemon_public_key: daemonPublicKey,
          }))
          return
        }
        if (frame.kind !== "client_request") return
        assert.ok(socket.daemon)
        const envelope = JSON.parse(decryptRelayPayload(socket.daemon.getPrivateKey(), frame.encrypted_request))
        requests.push(envelope.request)
        const request = envelope.request
        if ("LaunchProviderRun" in request || "LaunchProviderRuns" in request) launchRequests += 1
        if ("SubmitPrompt" in request) submitRequests += 1
        if ("GetProviderAccountProfile" in request || "SetProviderAccountCredential" in request) credentialRequests += 1
        if (role === "worker" && "GetRoomEnvironmentState" in request) {
          throw new Error("worker must not own the Room environment query")
        }
        if (role === "home" && !("GetRoomEnvironmentState" in request)) {
          throw new Error("home must not provide worker identity or provider evidence")
        }
        const response = responseFor(request, requestApi, {
          role,
          executionState,
          importedProviderRunId,
          providerRunState,
          omitProvider,
          omitToolProvider,
          mismatchProvider,
        })
        const encryptedResponse = encryptRelayPayload(
          frame.encrypted_request.sender_public_key,
          Buffer.from(JSON.stringify(response), "utf8"),
        ).payload
        socket.send(JSON.stringify({
          kind: "client_response",
          request_id: frame.request_id,
          encrypted_response: encryptedResponse,
        }))
      } catch (error) {
        serverError = error
        socket.close()
      }
    })
  })

  return {
    endpoint: "ws://127.0.0.1:" + address.port,
    requests,
    get launchRequests() {
      return launchRequests
    },
    get credentialRequests() {
      return credentialRequests
    },
    get submitRequests() {
      return submitRequests
    },
    get spawnedAgentIds() {
      return executionState ? [...executionState.agents.keys()] : []
    },
    get serverError() {
      return serverError
    },
    async close() {
      const closePromise = new Promise((resolve) => server.close(resolve))
      await closePromise
      assert.ifError(serverError)
    },
  }
}

function createExecutionFixtureState({ omitToolProvider = null } = {}) {
  return {
    omitToolProvider,
    agents: new Map(),
    runs: new Map(),
    prompts: new Map(),
    actions: [],
    nextSequence: 1,
  }
}

function executionResponseFor(request, requestApi, state) {
  if ("RelayStatus" in request) {
    return {
      RelayStatus: {
        status: {
          configured: true,
          connected: true,
          daemon_id: binding.kernelId,
          machine_id: binding.machineId,
          daemon_alias: null,
          machine_alias: null,
          relay_url: "ws://relay.test",
          relay_token_configured: true,
        },
      },
    }
  }
  if ("GetRoomEnvironmentState" in request) {
    throw new Error("execution worker must not provide the Room environment query")
  }
  if ("GetProviderCommandCatalogs" in request) {
    return { ProviderCommandCatalogs: { catalogs: shippedCatalogs() } }
  }
  if ("GetProviderCatalog" in request) {
    const provider = request.GetProviderCatalog.provider
    return { ProviderCatalog: { catalog: executionCatalog(provider) } }
  }
  if ("GetProviderAuthStatus" in request) {
    const value = request.GetProviderAuthStatus
    return {
      ProviderAuthStatus: {
        status: {
          provider: value.provider,
          auth_state: "authenticated",
          account_profile: value.account_profile,
          identity_summary: null,
          plan: null,
          login_hint: null,
          detected_version: "fixture",
        },
      },
    }
  }
  if ("SpawnAgent" in request) {
    const value = request.SpawnAgent
    const provider = value.provider
    const model = value.model ?? `${provider}-model`
    const agentId = `execution-agent-${provider}`
    const runId = `execution-run-${provider}`
    const threadId = `execution-thread-${provider}`
    const agent = {
      id: agentId,
      agent_ref: `@${provider}`,
      session_id: binding.roomId,
      alias: value.alias ?? null,
      provider,
      model,
      account_profile: value.account_profile ?? "default",
      state: "Idle",
      is_processing: false,
      worktree_id: null,
      workspace_id: null,
      remote_execution: null,
    }
    state.agents.set(agentId, agent)
    state.runs.set(runId, executionRun({ provider, agentId, runId, threadId, model, accountProfile: agent.account_profile }))
    return { AgentSpawned: { agent } }
  }
  if ("AttachToSession" in request) {
    return {
      SessionAttached: {
        attachment: {
          id: `execution-attachment-${state.agents.size}`,
          session_id: binding.roomId,
          client_id: request.AttachToSession.client_id,
          capability_level: "FullTerminal",
        },
      },
    }
  }
  if ("SubmitPrompt" in request) {
    const value = request.SubmitPrompt
    const agent = state.agents.get(value.target_agent_id)
    if (!agent) throw new Error(`unknown execution fixture agent ${value.target_agent_id}`)
    const run = [...state.runs.values()].find((candidate) => candidate.agent_instance_id === agent.id)
    if (!run) throw new Error(`unknown execution fixture run for ${agent.id}`)
    const promptId = `execution-prompt-${run.provider}`
    state.prompts.set(agent.id, { promptId, run, prompt: value.prompt })
    state.actions.push(executionAction(agent.id, state.nextSequence++))
    return {
      PromptSubmitted: {
        outcome: { Started: { prompt: { id: promptId } } },
        session: executionSession(state),
        agent_activity: {},
        agent_activity_revision: 1,
      },
    }
  }
  if ("ListRoomEnvironmentActionHistory" in request) {
    return {
      RoomEnvironmentActionHistoryListed: {
        page: { actions: state.actions },
      },
    }
  }
  if ("GetSessionState" in request) {
    return { SessionState: { session: executionSession(state) } }
  }
  if ("GetSessionHistoryOutline" in request) {
    const requestedAgentIds = request.GetSessionHistoryOutline.agent_ids ?? [...state.agents.keys()]
    return {
      SessionHistoryOutline: {
        agents: requestedAgentIds.map((agentId) => ({
          agent_id: agentId,
          turns: executionTurns(state, agentId),
        })),
      },
    }
  }
  if ("GetSessionHistoryBlobContent" in request
    && request.GetSessionHistoryBlobContent.blob_id.startsWith("history-tool-")) {
    assert.equal(role, "worker")
    const blobId = request.GetSessionHistoryBlobContent.blob_id
    const match = /^history-tool-(codex|opencode|claude)-(previous|current)$/.exec(blobId)
    assert.ok(match)
    const [, provider, phase] = match
    return {
      SessionHistoryBlobContent: {
        blob_id: blobId,
        entries: [historyToolEntry({ provider, phase })],
      },
    }
  }
  if ("GetSessionHistoryBlobContent" in request) {
    const value = request.GetSessionHistoryBlobContent
    const promptState = state.prompts.get(value.agent_id)
    if (!promptState || state.omitToolProvider === promptState.run.provider) {
      return { SessionHistoryBlobContent: { blob_id: value.blob_id, entries: [] } }
    }
    return {
      SessionHistoryBlobContent: {
        blob_id: value.blob_id,
        entries: [executionToolEntry(promptState.run, value.blob_id)],
      },
    }
  }
  if ("GetProviderRun" in request) {
    const run = state.runs.get(request.GetProviderRun.provider_run_id)
    if (!run) throw new Error(`unknown execution fixture run ${request.GetProviderRun.provider_run_id}`)
    return { ProviderRun: { provider_run: run } }
  }
  throw new Error("unexpected execution public request: " + JSON.stringify(request))
}

function executionCatalog(provider) {
  return {
    all: [{
      id: provider,
      name: provider,
      models: {
        [`${provider}-model`]: { id: `${provider}-model`, name: `${provider} model`, status: "active", variants: {} },
      },
    }],
    default: { [provider]: `${provider}-model` },
    connected: [provider],
  }
}

function executionRun({ provider, agentId, runId, threadId, model, accountProfile }) {
  return {
    id: runId,
    session_id: binding.roomId,
    agent_instance_id: agentId,
    owner_user_id: "user-1",
    adapter_key: provider,
    provider,
    account_profile: accountProfile,
    model,
    variant: null,
    usage_tokens_total: null,
    state: "Running",
    endpoint_mode: "managed",
    client_interface: "chariox",
    process_label: `${provider}-official-fixture-process`,
    pty_target: null,
    pty_program: null,
    pty_args: [],
    pty_env: {},
    pty_env_remove: [],
    working_directory: null,
    structured_endpoint: null,
    runtime_mcp_server_url: null,
    mcp_servers: [],
    remote_extension_manifest: {},
    provider_config_overrides: {},
    write_access_mode: "unrestricted",
    execution_mode: "build",
    permission_level: "yolo",
    control_capabilities: [],
    resume_state: {},
    external_provider_import: null,
    provider_session_id: threadId,
    started_at_ms: 1,
    last_activity_at_ms: 2,
  }
}

function executionSession(state) {
  return {
    id: binding.roomId,
    project_id: "project-1",
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    owner_user_id: "user-1",
    created_at_ms: 1,
    status: "active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 16,
    agents: [...state.agents.values()],
  }
}

function executionAction(agentId, sequence) {
  return {
    action_id: `execution-action-${agentId}`,
    sequence,
    idempotency_key: null,
    actor_id: `agent:${agentId}`,
    runtime_generation: 1,
    mode: "computer",
    kind: "pointer_click",
    arguments: { kind: "pointer_click", x: 640, y: 400, button: "left", click_count: 1, viewport_revision: 1 },
    targets: [{ kind: "desktop" }],
    state: "completed",
    cancellation_requested: false,
    submitted_at_ms: 1,
    started_at_ms: 1,
    finished_at_ms: 2,
    outcome: { status: "completed" },
  }
}

function executionTurns(state, agentId) {
  const promptState = state.prompts.get(agentId)
  if (!promptState) return []
  const { run, promptId } = promptState
  const turnId = `execution-turn-${run.provider}`
  const providerTurnId = `execution-provider-turn-${run.provider}`
  const pageEntry = (kind, text, timestampMs) => ({
    entry_index: kind === "user_prompt" ? 0 : 1,
    fragment_start: 0,
    fragment_end: text.length,
    total_chars: text.length,
    entry: {
      session_id: binding.roomId,
      provider_run_id: run.id,
      agent_id: agentId,
      source_attachment_id: null,
      prompt_origin: "chariox",
      kind,
      merge_key: kind === "provider_tool" ? `tool-call-${run.provider}` : null,
      source: null,
      external_provider: run.provider,
      external_provider_session_id: run.provider_session_id,
      external_provider_turn_id: providerTurnId,
      observed_at_ms: timestampMs,
      external_observation: null,
      attachments: [],
      text,
      timestamp_ms: timestampMs,
    },
  })
  const turn = {
    turn_id: turnId,
    prompt_id: promptId,
    prompt_origin: "chariox",
    external_provider: run.provider,
    external_provider_session_id: run.provider_session_id,
    external_provider_turn_id: providerTurnId,
    started_at_ms: 1,
    lifecycle: "completed",
    completed_at_ms: 3,
    user_prompt: pageEntry("user_prompt", promptState.prompt, 1),
    entries: [pageEntry("provider_output", `completed ${run.provider} execution`, 3)],
    summary: null,
    blobs: state.omitToolProvider === run.provider ? [] : [{
      blob_id: `execution-tool-${run.provider}`,
      kind: "provider_tool",
      title: "slice_mouse · COMPLETED",
      summary: "official provider tool round trip",
      sequence_start: 2,
      sequence_end: 2,
      entry_count: 1,
      total_chars: 100,
      timestamp_ms: 2,
    }],
  }
  return [turn]
}

function executionToolEntry(run, blobId) {
  const text = JSON.stringify({
    id: `tool-call-${run.provider}`,
    tool: "slice_mouse",
    status: "completed",
    input: { action: "click", x: 640, y: 400, button: "left" },
    output: "completed",
  })
  return {
    entry_index: 1,
    fragment_start: 0,
    fragment_end: text.length,
    total_chars: text.length,
    entry: {
      session_id: binding.roomId,
      provider_run_id: run.id,
      agent_id: run.agent_instance_id,
      source_attachment_id: null,
      prompt_origin: "chariox",
      kind: "provider_tool",
      merge_key: `tool-call-${run.provider}`,
      source: null,
      external_provider: run.provider,
      external_provider_session_id: run.provider_session_id,
      external_provider_turn_id: `execution-provider-turn-${run.provider}`,
      observed_at_ms: 2,
      external_observation: null,
      attachments: [],
      text,
      timestamp_ms: 2,
    },
  }
}

function responseFor(request, requestApi, {
  role = "worker",
  executionState = null,
  importedProviderRunId = null,
  providerRunState = "Running",
  omitProvider = null,
  omitToolProvider = null,
  mismatchProvider = null,
} = {}) {
  if (executionState) return executionResponseFor(request, requestApi, executionState)
  if ("RelayStatus" in request) {
    assert.equal(role, "worker")
    return {
      RelayStatus: {
        status: {
          configured: true,
          connected: true,
          daemon_id: binding.kernelId,
          machine_id: binding.machineId,
          daemon_alias: null,
          machine_alias: null,
          relay_url: "ws://relay.test",
          relay_token_configured: true,
        },
      },
    }
  }
  if ("GetRoomEnvironmentState" in request) {
    assert.equal(role, "home")
    assert.equal(request.GetRoomEnvironmentState.session_id, binding.roomId)
    return { RoomEnvironmentState: { environment: { session_id: binding.roomId, environment_id: binding.environmentId } } }
  }
  if ("GetSessionHistoryOutline" in request) {
    assert.equal(role, "worker")
    assert.equal(request.GetSessionHistoryOutline.session_id, binding.roomId)
    assert.deepEqual(request.GetSessionHistoryOutline.agent_ids, ["agent-1"])
    assert.equal(request.GetSessionHistoryOutline.latest_prompt_count, 20)
    return {
      SessionHistoryOutline: {
        agents: [{
          agent_id: "agent-1",
          turns: completedProviderTurns({ omitProvider, omitToolProvider, mismatchProvider }),
        }],
      },
    }
  }
  if ("GetProviderCommandCatalogs" in request) {
    assert.equal(role, "worker")
    return { ProviderCommandCatalogs: { catalogs: shippedCatalogs() } }
  }
  if ("GetProviderCatalog" in request) {
    assert.equal(role, "worker")
    const value = request.GetProviderCatalog
    assert.deepEqual(value.execution_location, { kind: "worker", kernel_ref: binding.kernelId })
    assert.equal(Object.keys(value.account_profiles).length, 1)
    return { ProviderCatalog: { catalog: advertisedCatalog() } }
  }
  if ("GetProviderAuthStatus" in request) {
    assert.equal(role, "worker")
    const value = request.GetProviderAuthStatus
    return {
      ProviderAuthStatus: {
        status: {
          provider: value.provider,
          auth_state: "authenticated",
          account_profile: value.account_profile,
          identity_summary: null,
          plan: null,
          login_hint: null,
          detected_version: "fixture",
        },
      },
    }
  }
  if ("GetSessionHistoryBlobContent" in request) {
    assert.equal(role, "worker")
    const blobId = request.GetSessionHistoryBlobContent.blob_id
    const match = /^history-tool-(codex|opencode|claude)-(previous|current)$/.exec(blobId)
    assert.ok(match)
    const [, provider, phase] = match
    return {
      SessionHistoryBlobContent: {
        blob_id: blobId,
        entries: [historyToolEntry({ provider, phase })],
      },
    }
  }
  if ("GetProviderRun" in request) {
    assert.equal(role, "worker")
    const providerRunId = request.GetProviderRun.provider_run_id
    const providerEntry = Object.entries(providerRunIds).find(([, refs]) => (
      refs.current === providerRunId || refs.previous === providerRunId
    ))
    assert.ok(providerEntry)
    const [provider, refs] = providerEntry
    const current = refs.current === providerRunId
    const threadId = mismatchProvider === provider && current
      ? "changed-" + provider
      : providerThreadIds[provider]
    return {
      ProviderRun: {
        provider_run: {
          id: providerRunId,
          session_id: binding.roomId,
          agent_instance_id: "agent-1",
          owner_user_id: "user-1",
          adapter_key: provider,
          provider,
          account_profile: providerProfiles[provider],
          model: provider + "-model",
          variant: null,
          usage_tokens_total: null,
          state: current ? providerRunState : "Ended",
          endpoint_mode: "managed",
          client_interface: "chariox",
          process_label: provider + "-fixture-process",
          pty_target: null,
          pty_program: null,
          pty_args: [],
          pty_env: {},
          pty_env_remove: [],
          working_directory: null,
          structured_endpoint: null,
          runtime_mcp_server_url: null,
          mcp_servers: [],
          remote_extension_manifest: {},
          provider_config_overrides: {},
          write_access_mode: "unrestricted",
          execution_mode: "build",
          permission_level: "yolo",
          control_capabilities: [],
          resume_state: {},
          external_provider_import: current && providerRunId === importedProviderRunId
            ? {
              external_provider_session_id: threadId,
              external_provider: provider,
              external_provider_session_provider_id: provider + "-session",
              account_profile: providerProfiles[provider],
              observed_cursor: {},
              imported_at_ms: 1,
            }
            : null,
          provider_session_id: threadId,
          started_at_ms: 1,
          last_activity_at_ms: 2,
        },
      },
    }
  }
  throw new Error("unexpected public request: " + JSON.stringify(request))
}

function expectedWorkerRequests(requestApi) {
  const expected = [requestApi.relayStatusRequest(), requestApi.getProviderCommandCatalogsRequest()]
  for (const provider of ["codex", "opencode", "claude"]) {
    expected.push(requestApi.getProviderCatalogRequest({
      provider,
      accountProfile: providerProfiles[provider],
      executionLocation: { kind: "worker", kernel_ref: binding.kernelId },
    }))
    expected.push(requestApi.getProviderAuthStatusRequest(provider, providerProfiles[provider]))
    expected.push(requestApi.getProviderRunRequest(providerRunIds[provider].current))
    expected.push(requestApi.getProviderRunRequest(providerRunIds[provider].previous))
  }
  expected.push(requestApi.getSessionHistoryOutlineRequest(binding.roomId, ["agent-1"], 20))
  for (const provider of ["codex", "opencode", "claude"]) {
    expected.push(requestApi.getSessionHistoryBlobContentRequest(
      binding.roomId,
      "agent-1",
      `history-tool-${provider}-current`,
    ))
    expected.push(requestApi.getSessionHistoryBlobContentRequest(
      binding.roomId,
      "agent-1",
      `history-tool-${provider}-previous`,
    ))
  }
  return expected
}

function fixtureProviderPromptIds() {
  return Object.fromEntries(["codex", "opencode", "claude"].map((provider) => [
    provider,
    "fixture-prompt-" + provider + "-current",
  ]))
}

function minimalRequestApi() {
  return {
    relayStatusRequest: () => ({ RelayStatus: null }),
    getRoomEnvironmentStateRequest: (sessionId) => ({ GetRoomEnvironmentState: { session_id: sessionId } }),
    getProviderCommandCatalogsRequest: () => ({ GetProviderCommandCatalogs: null }),
    getProviderCatalogRequest: (options) => ({ GetProviderCatalog: options }),
    getProviderAuthStatusRequest: (provider, accountProfile) => ({
      GetProviderAuthStatus: { provider, account_profile: accountProfile },
    }),
    getProviderRunRequest: (providerRunId) => ({ GetProviderRun: { provider_run_id: providerRunId } }),
    getSessionHistoryOutlineRequest: (sessionId, agentIds, latestPromptCount) => ({
      GetSessionHistoryOutline: {
        session_id: sessionId,
        agent_ids: agentIds,
        latest_prompt_count: latestPromptCount,
      },
    }),
    getSessionHistoryBlobContentRequest: (sessionId, agentId, blobId) => ({
      GetSessionHistoryBlobContent: { session_id: sessionId, agent_id: agentId, blob_id: blobId },
    }),
  }
}

function completedProviderTurns({ omitProvider = null, omitToolProvider = null, mismatchProvider = null } = {}) {
  const turns = []
  for (const provider of ["codex", "opencode", "claude"]) {
    if (provider === omitProvider) continue
    for (const phase of ["previous", "current"]) {
      const providerRunId = providerRunIds[provider][phase]
      const threadId = mismatchProvider === provider && phase === "current"
        ? "changed-" + provider
        : providerThreadIds[provider]
      const turnId = "fixture-turn-" + provider + "-" + phase
      turns.push({
        turn_id: turnId,
        prompt_id: "fixture-prompt-" + provider + "-" + phase,
        prompt_origin: "chariox",
        external_provider: provider,
        external_provider_session_id: threadId,
        external_provider_turn_id: "fixture-provider-turn-" + provider + "-" + phase,
        started_at_ms: 10,
        lifecycle: "completed",
        completed_at_ms: 20,
        user_prompt: historyPageEntry({
          provider,
          phase,
          providerRunId,
          threadId,
          kind: "user_prompt",
          text: "fixture prompt " + provider + " " + phase,
          timestampMs: 10,
        }),
        entries: [historyPageEntry({
          provider,
          phase,
          providerRunId,
          threadId,
          kind: "provider_output",
          text: "fixture completed output " + provider + " " + phase,
          timestampMs: 20,
        })],
        summary: null,
        blobs: provider === omitToolProvider ? [] : [historyToolBlob({ provider, phase })],
      })
    }
  }
  return turns
}

function historyToolBlob({ provider, phase }) {
  return {
    blob_id: "history-tool-" + provider + "-" + phase,
    kind: "provider_tool",
    title: "fixture tool · COMPLETED",
    summary: "fixture tool round trip",
    sequence_start: 11,
    sequence_end: 11,
    entry_count: 1,
    total_chars: 32,
    timestamp_ms: 15,
  }
}

function historyToolEntry({ provider, phase }) {
  const text = JSON.stringify({
    id: `fixture-tool-${provider}-${phase}`,
    tool: "slice_mouse",
    status: "completed",
    output: "completed",
  })
  return {
    entry_index: 1,
    fragment_start: 0,
    fragment_end: text.length,
    total_chars: text.length,
    entry: {
      session_id: binding.roomId,
      provider_run_id: providerRunIds[provider][phase],
      agent_id: "agent-1",
      source_attachment_id: null,
      prompt_origin: "chariox",
      kind: "provider_tool",
      merge_key: `fixture-tool-${provider}-${phase}`,
      source: null,
      external_provider: provider,
      external_provider_session_id: providerThreadIds[provider],
      external_provider_turn_id: `fixture-provider-turn-${provider}-${phase}`,
      observed_at_ms: 15,
      external_observation: null,
      attachments: [],
      text,
      timestamp_ms: 15,
    },
  }
}

function historyPageEntry({ provider, phase, providerRunId, threadId, kind, text, timestampMs }) {
  return {
    entry_index: 0,
    fragment_start: 0,
    fragment_end: text.length,
    total_chars: text.length,
    entry: {
      session_id: binding.roomId,
      provider_run_id: providerRunId,
      agent_id: "agent-1",
      source_attachment_id: null,
      prompt_origin: "chariox",
      kind,
      merge_key: null,
      source: null,
      external_provider: provider ?? null,
      external_provider_session_id: threadId,
      external_provider_turn_id: provider && phase
        ? `fixture-provider-turn-${provider}-${phase}`
        : "fixture-provider-turn",
      observed_at_ms: timestampMs,
      external_observation: null,
      attachments: [],
      text,
      timestamp_ms: timestampMs,
    },
  }
}

function shippedCatalogs() {
  return {
    codex: { provider: "codex", source: "shipped", discovery: "none", commands: [] },
    opencode: { provider: "opencode", source: "shipped", discovery: "none", commands: [] },
    claude: { provider: "claude", source: "shipped", discovery: "none", commands: [] },
  }
}

function advertisedCatalog() {
  return {
    all: [{
      id: "fixture-provider",
      name: "Fixture provider",
      models: { "fixture-model": { id: "fixture-model", name: "Fixture model", status: "active", variants: {} } },
    }],
    default: { "fixture-provider": "fixture-model" },
    connected: ["fixture-provider"],
  }
}
