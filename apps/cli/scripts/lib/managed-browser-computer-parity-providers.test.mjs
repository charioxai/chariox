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

const stateProvenance = Object.freeze({
  schema: "chariox.browser_computer.provider_state_proof.v1",
  source: "public-worker-runtime",
  observedOnTarget: true,
  copyDetected: false,
  checks: ["fresh-target-context", "no-provider-state-import"],
})

test("selkies.providers uses target-bound public catalog, auth, and runtime observations", async () => {
  const modules = await loadPublicClientModules()
  if (!modules) return

  const { LocalIpcClient } = modules.kernelClient
  const requestApi = modules.requestApi
  const relay = await createControlledRelay({ requestApi })
  const client = new LocalIpcClient(relay.endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: binding.kernelId,
  })
  try {
    const result = await runSelkiesProviders({
      client,
      identityClient: client,
      requestApi,
      request: {
        displayBackend: "selkies",
        binding,
        providerProfiles,
        providerRunIds,
        stateProvenance,
      },
    })

    assert.deepEqual(result.providers, {
      codex: "official",
      opencode: "official",
      claude: "official",
    })
    assert.equal(result.providerStateCopied, false)
    assert.deepEqual(result.providerStateProof, stateProvenance)
    assert.deepEqual(result.kernelId, binding.kernelId)
    assert.deepEqual(result.machineId, binding.machineId)
    assert.deepEqual(result.roomId, binding.roomId)
    assert.deepEqual(result.environmentId, binding.environmentId)
    assert.deepEqual(relay.requests, expectedPublicRequests(requestApi))
    assert.equal(relay.launchRequests, 0, "provider capability must not launch a provider")
    assert.equal(relay.credentialRequests, 0, "provider capability must not read or mutate credentials")
  } finally {
    await client.close()
    await relay.close()
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
  }
  const client = {
    async send(request) {
      requests.push(request)
      if ("RelayStatus" in request) {
        return { RelayStatus: { status: { configured: true, connected: true, daemon_id: binding.kernelId, machine_id: binding.machineId } } }
      }
      if ("GetRoomEnvironmentState" in request) {
        return { RoomEnvironmentState: { environment: { session_id: binding.roomId, environment_id: binding.environmentId } } }
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
      client,
      requestApi,
      request: {
        displayBackend: "selkies",
        binding,
        providerProfiles,
        providerRunIds,
        stateProvenance,
      },
    }),
    /authenticated provider runtime observation required/,
  )
  assert.equal(requests.filter((request) => "GetProviderRun" in request).length, 0)
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
    test.skip(`public LocalIpcClient regression requires existing kernel-client dist: ${detail}`)
    return null
  }
}

async function createControlledRelay({ requestApi }) {
  const { decryptRelayPayload, encryptRelayPayload } = await import(relayCryptoDistUrl.href)
  const { WebSocketServer } = createRequire(fileURLToPath(kernelClientDistUrl))("ws")
  const server = new WebSocketServer({ port: 0 })
  await once(server, "listening")
  const address = server.address()
  assert.ok(address && typeof address === "object")
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
          assert.deepEqual(frame.target, { daemon_id: binding.kernelId, daemon_alias: null })
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
        const response = responseFor(request, requestApi)
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

function responseFor(request, requestApi) {
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
          state: "running",
          endpoint_mode: "managed",
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
          external_provider_import: null,
          provider_session_id: null,
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
    requestApi.relayStatusRequest(),
    requestApi.getRoomEnvironmentStateRequest(binding.roomId),
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
  return expected
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
