import assert from "node:assert/strict"
import { createECDH } from "node:crypto"
import { once } from "node:events"
import { createRequire } from "node:module"
import test from "node:test"
import { fileURLToPath } from "node:url"

import { runSelkiesProviders } from "./managed-browser-computer-parity-providers.mjs"

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
  codex: "provider-run-codex",
  opencode: "provider-run-opencode",
  claude: "provider-run-claude",
})

test("selkies.providers uses target-bound public catalog, auth, and runtime observations", async () => {
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
      },
    })

    assert.deepEqual(result.providers, {
      codex: "official",
      opencode: "official",
      claude: "official",
    })
    assert.deepEqual(result.kernelId, binding.kernelId)
    assert.deepEqual(result.machineId, binding.machineId)
    assert.deepEqual(result.roomId, binding.roomId)
    assert.deepEqual(result.environmentId, binding.environmentId)
    assert.deepEqual(homeRelay.requests, [requestApi.getRoomEnvironmentStateRequest(binding.roomId)])
    assert.deepEqual(workerRelay.requests, expectedWorkerRequests(requestApi))
    assert.equal(workerRelay.launchRequests, 0, "provider capability must not launch a provider")
    assert.equal(workerRelay.credentialRequests, 0, "provider capability must not read or mutate credentials")
    assert.deepEqual(Object.keys(result.providerExecutionEvidence), ["codex", "opencode", "claude"])
    assert.ok(result.providerExecutionEvidence.codex.outputObserved)
    assert.equal(result.providerEvidence.codex.runtime.endpointMode, "Managed")
    assert.equal(result.providerEvidence.codex.runtime.providerSessionId, "codex-thread-1")
    assert.equal(result.providerExecutionEvidence.codex.providerSessionId, "codex-thread-1")
  } finally {
    await homeClient.close()
    await workerClient.close()
    await homeRelay.close()
    await workerRelay.close()
  }
})

test("selkies.providers does not accept an advertised catalog without authenticated runtime proof", async () => {
  const requests = []
  const requestApi = {
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
  }
  const homeClient = {
    async send(request) {
      requests.push(request)
      if ("GetRoomEnvironmentState" in request) {
        return { RoomEnvironmentState: { environment: { session_id: binding.roomId, environment_id: binding.environmentId } } }
      }
      throw new Error(`home received worker-only request: ${JSON.stringify(request)}`)
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
      throw new Error(`unexpected request: ${JSON.stringify(request)}`)
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
      },
    }),
    /requires authenticated provider runtime observation/,
  )
  assert.equal(requests.filter((request) => "GetProviderRun" in request).length, 0)
})

test("selkies.providers does not treat a Starting process label as provider execution", async () => {
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
        },
      }),
      /requires an active target provider runtime observation/,
    )
    assert.equal(workerRelay.requests.filter((request) => "GetSessionHistoryOutline" in request).length, 0)
  } finally {
    await homeClient.close()
    await workerClient.close()
    await homeRelay.close()
    await workerRelay.close()
  }
})

test("selkies.providers allows supported transfer metadata only with managed worker execution", async () => {
  const modules = await loadPublicClientModules()
  const { LocalIpcClient } = modules.kernelClient
  const requestApi = modules.requestApi
  const homeRelay = await createControlledRelay({ requestApi, role: "home" })
  const workerRelay = await createControlledRelay({
    requestApi, role: "worker",
    importedProviderRunId: providerRunIds.codex,
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
      },
    })
    assert.equal(result.providerEvidence.codex.runtime.endpointMode, "Managed")
    assert.equal(result.providerExecutionEvidence.codex.outputObserved, true)
    assert.equal(workerRelay.launchRequests, 0)
    assert.equal(workerRelay.credentialRequests, 0)
  } finally {
    await homeClient.close()
    await workerClient.close()
    await homeRelay.close()
    await workerRelay.close()
  }
})

test("selkies.providers rejects an external provider endpoint", async () => {
  const modules = await loadPublicClientModules()
  const { LocalIpcClient } = modules.kernelClient
  const requestApi = modules.requestApi
  const homeRelay = await createControlledRelay({ requestApi, role: "home" })
  const workerRelay = await createControlledRelay({ requestApi, role: "worker", providerEndpointMode: "External" })
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
        },
      }),
      /active target provider runtime observation/,
    )
  } finally {
    await homeClient.close()
    await workerClient.close()
    await homeRelay.close()
    await workerRelay.close()
  }
})

test("selkies.providers requires completed worker history, not only a managed runtime", async () => {
  const modules = await loadPublicClientModules()
  const { LocalIpcClient } = modules.kernelClient
  const requestApi = modules.requestApi
  const homeRelay = await createControlledRelay({ requestApi, role: "home" })
  const workerRelay = await createControlledRelay({ requestApi, role: "worker", executionEvidence: false })
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
        },
      }),
      /requires a completed worker provider turn for codex/,
    )
  } finally {
    await homeClient.close()
    await workerClient.close()
    await homeRelay.close()
    await workerRelay.close()
  }
})

async function loadPublicClientModules() {
  try {
    const [{ LocalIpcClient }, requestApi, relayCrypto] = await Promise.all([
      import(kernelClientDistUrl.href),
      import(kernelRequestsDistUrl.href),
      import(relayCryptoDistUrl.href),
    ])
    return { kernelClient: { LocalIpcClient }, requestApi, relayCrypto }
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error)
    throw new Error(`public LocalIpcClient regression requires existing kernel-client dist: ${detail}`, { cause: error })
  }
}

async function createControlledRelay({
  requestApi,
  role,
  importedProviderRunId = null,
  providerRunState = "Running",
  providerEndpointMode = "Managed",
  executionEvidence = true,
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
  let serverError

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
        if ("GetProviderAccountProfile" in request || "SetProviderAccountCredential" in request) credentialRequests += 1
        if (role === "worker" && "GetRoomEnvironmentState" in request) {
          throw new Error("worker must not own the Room environment query")
        }
        if (role === "home" && !("GetRoomEnvironmentState" in request)) {
          throw new Error("home must not provide worker identity or provider evidence")
        }
        const response = responseFor(request, requestApi, {
          role,
          importedProviderRunId,
          providerRunState,
          providerEndpointMode,
          executionEvidence,
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
    endpoint: `ws://127.0.0.1:${address.port}`,
    requests,
    get launchRequests() {
      return launchRequests
    },
    get credentialRequests() {
      return credentialRequests
    },
    get serverError() {
      return serverError
    },
    async close() {
      assert.ifError(serverError)
      await new Promise((resolve) => server.close(resolve))
    },
  }
}

function responseFor(request, requestApi, {
  role = "worker",
  importedProviderRunId = null,
  providerRunState = "Running",
  providerEndpointMode = "Managed",
  executionEvidence = true,
} = {}) {
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
        agents: [{ agent_id: "agent-1", turns: executionEvidence ? completedProviderTurns() : [] }],
      },
    }
  }
  if ("GetProviderCommandCatalogs" in request) {
    return { ProviderCommandCatalogs: { catalogs: shippedCatalogs() } }
  }
  if ("GetProviderCatalog" in request) {
    const value = request.GetProviderCatalog
    assert.deepEqual(value.execution_location, { kind: "worker", kernel_ref: binding.kernelId })
    assert.equal(Object.keys(value.account_profiles).length, 1)
    return { ProviderCatalog: { catalog: advertisedCatalog() } }
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
  if ("GetProviderRun" in request) {
    const providerRunId = request.GetProviderRun.provider_run_id
    const provider = Object.entries(providerRunIds).find(([, id]) => id === providerRunId)?.[0]
    assert.ok(provider)
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
          model: `${provider}-model`,
          variant: null,
          usage_tokens_total: null,
          state: providerRunState,
          endpoint_mode: providerEndpointMode,
          client_interface: "chariox",
          process_label: `${provider}-fixture-process`,
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
          external_provider_import: providerRunId === importedProviderRunId
            ? {
              external_provider_session_id: `${provider}-thread-1`,
              external_provider: provider,
              external_provider_session_provider_id: `${provider}-thread-1`,
              observed_cursor: {},
              imported_at_ms: 1,
            }
            : null,
          provider_session_id: `${provider}-thread-1`,
          started_at_ms: 1,
          last_activity_at_ms: 2,
        },
      },
    }
  }
  throw new Error(`unexpected public request: ${JSON.stringify(request)}`)
}

function expectedPublicRequests(requestApi) {
  const expected = [
    requestApi.getProviderCommandCatalogsRequest(),
  ]
  for (const provider of ["codex", "opencode", "claude"]) {
    expected.push(requestApi.getProviderCatalogRequest({
      provider,
      accountProfile: providerProfiles[provider],
      executionLocation: { kind: "worker", kernel_ref: binding.kernelId },
    }))
    expected.push(requestApi.getProviderAuthStatusRequest(provider, providerProfiles[provider]))
    expected.push(requestApi.getProviderRunRequest(providerRunIds[provider]))
  }
  expected.push(requestApi.getSessionHistoryOutlineRequest(binding.roomId, ["agent-1"], 20))
  return expected
}

function expectedWorkerRequests(requestApi) {
  return [requestApi.relayStatusRequest(), ...expectedPublicRequests(requestApi)]
}

function completedProviderTurns() {
  return ["codex", "opencode", "claude"].map((provider, index) => ({
    turn_id: `fixture-turn-${provider}`,
    prompt_id: `fixture-prompt-${provider}`,
    prompt_origin: "chariox",
    external_provider: provider,
    external_provider_session_id: `${provider}-thread-1`,
    external_provider_turn_id: `${provider}-turn-1`,
    started_at_ms: 10 + index,
    lifecycle: "completed",
    completed_at_ms: 20 + index,
    user_prompt: historyPageEntry({
      providerRunId: providerRunIds[provider],
      kind: "user_prompt",
      text: `fixture prompt ${provider}`,
      timestampMs: 10 + index,
    }),
    entries: [historyPageEntry({
      providerRunId: providerRunIds[provider],
      kind: "provider_output",
      text: `fixture completed output ${provider}`,
      timestampMs: 20 + index,
      externalProvider: provider,
      externalProviderSessionId: `${provider}-thread-1`,
      externalProviderTurnId: `${provider}-turn-1`,
    })],
    summary: null,
    blobs: [],
  }))
}

function historyPageEntry({
  providerRunId,
  kind,
  text,
  timestampMs,
  externalProvider = null,
  externalProviderSessionId = null,
  externalProviderTurnId = null,
}) {
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
      external_provider: externalProvider,
      external_provider_session_id: externalProviderSessionId,
      external_provider_turn_id: externalProviderTurnId,
      observed_at_ms: null,
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
    all: [{ id: "fixture-provider", name: "Fixture provider", models: { "fixture-model": { id: "fixture-model", name: "Fixture model", status: "active", variants: {} } } }],
    default: { "fixture-provider": "fixture-model" },
    connected: ["fixture-provider"],
  }
}
