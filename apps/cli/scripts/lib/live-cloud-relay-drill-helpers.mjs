import { spawn } from "node:child_process"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { resolveBuiltBinary } from "./drill-runtime-helpers.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..", "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const DEV_AUTH_SECRET = "chariox-cloud-live-drill-dev-auth-secret"

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

export function log(name, details = null) {
  if (details == null) console.log(`[MP-08/MP-10/MP-11 cloud-relay-drill] ${name}`)
  else console.log(`[MP-08/MP-10/MP-11 cloud-relay-drill] ${name}`, JSON.stringify(details))
}

export function assert(condition, message) {
  if (!condition) {
    throw new Error(message) // Responses can contain credentials; never print details.
  }
}

export async function run(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ["ignore", "pipe", "pipe"],
    })
    let stdout = ""
    let stderr = ""
    child.stdout.on("data", (chunk) => { stdout += chunk.toString() })
    child.stderr.on("data", (chunk) => { stderr += chunk.toString() })
    child.on("error", reject)
    child.on("close", (code, signal) => resolve({ code, signal, stdout, stderr }))
  })
}

export function spawnProcess(command, args, options) {
  const child = spawn(command, args, {
    ...options,
    stdio: ["ignore", "pipe", "pipe"],
  })
  const name = options.name ?? path.basename(command)
  // Service output is private runtime data, not evidence. Do not forward it.
  child.stdout.resume()
  child.stderr.resume()
  child.on("exit", (code, signal) => {
    log(`${name}:exit`, { code, signal })
  })
  return child
}

export async function waitForHttp(url, timeoutMs = 30_000) {
  const started = Date.now()
  let lastError = null
  while (Date.now() - started < timeoutMs) {
    try {
      const response = await fetch(url)
      if (response.ok) return
      lastError = new Error(`status ${response.status}`)
    } catch (error) {
      lastError = error
    }
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${url}: ${lastError instanceof Error ? lastError.message : String(lastError)}`)
}

export async function buildRustBinary(manifest, binName) {
  const args = ["build", "--manifest-path", manifest, "--bin", binName]
  const lock = process.env.CHARIOX_RUST_COMPILE_LOCK
  const result = await run(lock ? "flock" : "cargo", lock ? [lock, "cargo", ...args] : args, {
    env: { ...process.env, CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? "4" },
  })
  if (result.code !== 0) throw new Error(`${binName} build failed\n${result.stdout}\n${result.stderr}`)
  return resolveBuiltBinary(path.join(path.dirname(manifest), "target/debug", binName), manifest, binName)
}

export async function buildKernelIfNeeded() {
  return buildRustBinary(path.join(repoRoot, "apps/kernel/Cargo.toml"), "chariox-kernel")
}

export async function terminateChild(child, signal = "SIGTERM") {
  if (!child || child.exitCode != null || child.signalCode != null) return
  if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error("refusing to signal an invalid child PID")
  child.kill(signal)
  await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(5_000)])
  if (child.exitCode == null && child.signalCode == null) {
    if (!Number.isSafeInteger(child.pid) || child.pid <= 1) throw new Error("refusing to signal an invalid child PID")
    child.kill("SIGKILL")
    await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(2_000)])
  }
  if (child.exitCode == null && child.signalCode == null) throw new Error("owned child did not exit during cleanup")
}

export function makePorts() {
  const base = 52000 + Math.floor(Math.random() * 1000)
  return {
    relayPort: base,
    kernelPort: base + 1000,
    mcpPort: base + 2000,
    opencodePort: base + 3000,
    codexPort: base + 3001,
    cloudPort: base + 4000,
  }
}

export async function waitForLocalDaemon(LocalIpcClient, requests, kernelUrl, workspace, localAuthEnvironment) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl, { localAuthEnvironment })
    try {
      const created = unwrap(
        await probe.send(requests.createSessionRequest(workspace, workspace)),
        "SessionCreated",
      )
      await probe.send(requests.endSessionRequest(created.session.id)).catch(() => {})
      await probe.close()
      return
    } catch (error) {
      lastError = error
      await probe.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`local daemon did not become ready: ${lastError instanceof Error ? lastError.message : String(lastError)}`)
}

export function unwrap(resp, key) {
  return resp?.[key] ?? resp
}

export async function expectReject(promise, label, expectedText) {
  try {
    await promise
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    if (expectedText && !message.includes(expectedText)) {
      throw new Error(`${label} rejected with unexpected error: ${message}`)
    }
    return message
  }
  throw new Error(`${label} unexpectedly succeeded`)
}

export function addWorkflowNodeRequest(sessionId, workflowRef, agentId, expectedRevision = null) {
  return {
    AddWorkflowNode: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      agent_id: agentId,
      expected_workflow_revision: expectedRevision,
    },
  }
}

export function updateWorkflowNodeInstructionsRequest(sessionId, workflowRef, nodeId, instructions, expectedRevision = null) {
  return {
    UpdateWorkflowNodeInstructions: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      node_id: nodeId,
      instructions,
      expected_workflow_revision: expectedRevision,
    },
  }
}

export function createWorkflowEndpointRequest(sessionId, workflowRef, entryNodeId, alias, expectedRevision = null) {
  return {
    CreateWorkflowEndpoint: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      entry_node_id: entryNodeId,
      alias,
      expected_workflow_revision: expectedRevision,
    },
  }
}

export function addWorkflowEdgeRequest(sessionId, workflowRef, fromNodeId, toNodeId, expectedRevision = null) {
  return {
    AddWorkflowEdge: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      from_node_id: fromNodeId,
      to_node_id: toNodeId,
      output_schema_ref: null,
      validation_policy: null,
      expected_workflow_revision: expectedRevision,
    },
  }
}

export function removeWorkflowEdgeRequest(sessionId, workflowRef, edgeId, expectedRevision = null) {
  return {
    RemoveWorkflowEdge: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      edge_id: edgeId,
      expected_workflow_revision: expectedRevision,
    },
  }
}

/** MP-08 / MP-10 / MP-11: load exactly the same built modules as the live command. */
export async function loadCloudRelayDrillModules() {
  const [ipc, requests, cloud, credentials, identities, relayApi, commands, clientCommands, http] = await Promise.all([
    import("../../../../packages/kernel-client/dist/ipc.js"),
    import("../../../../packages/kernel-client/dist/ipc-requests.js"),
    import("../../dist/cloud-client.js"),
    import("../../dist/cloud-client-credential-store.js"),
    import("../../dist/cli-relay-identity-store.js"),
    import("../../dist/relay-api.js"),
    import("../../dist/command-actions.js"),
    import("../../dist/cloud-client-command.js"),
    import("../../dist/cloud-client-http.js"),
  ])
  return { ...ipc, requests, ...cloud, ...credentials, ...identities, relayApi, ...commands, ...clientCommands, ...http }
}

export function createCloudDrillClient(modules, stateRoot, name) {
  const root = path.join(stateRoot, name)
  const identity = modules.createCliRelayIdentityStore(path.join(root, "relay", "cli-identity-v1.json")).getOrCreate()
  const store = new modules.CloudClientCredentialStore(path.join(root, "relay", "cloud-client.json"))
  return { identity, client: new modules.CloudClient(store, () => identity) }
}

export async function approveCloudDrillDevice(apiUrl, userCode, { email, accountSlug }) {
  await postJsonWithHeaders(`${apiUrl}/auth/dev/device/approve`, {
    userCode, accountSlug, providerSubject: `auth0|${accountSlug}`,
    email, emailVerified: true, displayName: accountSlug,
  }, { "x-chariox-dev-auth-secret": DEV_AUTH_SECRET })
}

export async function loginCloudDrillUser(apiUrl, { modules, stateRoot, name, email, accountSlug }) {
  const terminal = createCloudDrillClient(modules, stateRoot, name)
  try {
    const profile = await terminal.client.login(apiUrl, verification => approveCloudDrillDevice(apiUrl, verification.userCode, { email, accountSlug }))
    return { ...terminal, profile }
  } catch (error) { terminal.client.stop(); throw error }
}

/** Session-scoped collaboration uses the receiving terminal's private CLIENT
 * authority and key. The kernel never supplies human credentials. */
export async function issueSessionScopedClientToken(modules, terminal, { accountId, realmId, sessionId, targetDaemonId }) {
  let credential = await terminal.client.store.session(terminal.identity.publicKeyThumbprint)
  const issue = () => modules.cloudClientRequest(credential.profile.apiUrl, "/relay/token", { body: {
    sessionToken: credential.accessToken, accountId, realmId,
    subject: credential.clientId, subjectKind: "client", userId: credential.profile.userId,
    clientId: credential.clientId, publicKeyThumbprint: terminal.identity.publicKeyThumbprint,
    sessionId, allowedTargets: [targetDaemonId], ttlMs: 300_000,
  } })
  let grant
  try { grant = await issue() }
  catch (error) {
    if (!(error instanceof modules.CloudClientAuthError) || error.code !== "session_invalid") throw error
    credential = await terminal.client.store.session(terminal.identity.publicKeyThumbprint, false, credential.accessToken)
    grant = await issue()
  }
  modules.relayApi.requireRelayTokenKeyBinding(grant.token, terminal.identity.publicKeyThumbprint, "Cloud relay drill session grant")
  const expiresAtMs = Date.parse(grant.expiresAt)
  assert(Number.isFinite(expiresAtMs) && expiresAtMs > Date.now(), "session grant must have a valid expiry")
  return { token: grant.token, expiresAtMs }
}

export async function connectSessionScopedCloudClient(modules, terminal, scope) {
  const issue = () => issueSessionScopedClientToken(modules, terminal, scope)
  const grant = await issue()
  const profile = await terminal.client.profile()
  const client = new modules.LocalIpcClient(profile.relayUrl, {
    relayAuthToken: grant.token, relayIdentity: terminal.identity, targetDaemonId: scope.targetDaemonId,
    kernelPingIntervalMs: 60_000, kernelMaxMissedPongs: 10,
  })
  client.startRelayAuthRenewal(grant.expiresAtMs, issue)
  return client
}

export async function postJsonWithHeaders(url, body, headers = {}) {
  const response = await fetch(url, {
    method: "POST", headers: { "content-type": "application/json", ...headers },
    body: JSON.stringify(body), redirect: "error", signal: AbortSignal.timeout(20_000),
  })
  if (!response.ok) throw new Error(`Cloud drill request failed (HTTP ${response.status})`)
  return response.status === 204 ? null : response.json()
}

export async function waitForCloudRelayTarget(cloudClient, { daemonId, status }, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const target = (await cloudClient.directory()).find(entry => entry.daemonId === daemonId)
    if (target?.status === status) return target
    await sleep(250)
  }
  throw new Error(`timed out waiting for cloud relay target ${daemonId} to become ${status}`)
}

export function createMinimalCommandDeps({ apiUrl, runId, workspace, localClient, modules, terminal, profileRef, notices }) {
  const identity = { email: `${runId}@example.com`, accountSlug: runId }
  const openUrl = async url => {
    const userCode = new URL(url).searchParams.get("user_code")
    if (userCode) await approveCloudDrillDevice(apiUrl, userCode, identity)
    return false
  }
  const saveProfile = async profile => { profileRef.current = profile }
  return {
    workspace, worktree: workspace, clientId: `cli:${terminal.identity.publicKeyThumbprint}`,
    isAttached: () => false,
    sessionState: () => ({ id: null, agents: [], workflows: [] }),
    flashFooter: (message, tone) => {
      if (tone === "error") throw new Error(message)
      log("command-footer", { tone, message })
    },
    appendNotice: message => { notices.push(message); log("command-notice", { firstLine: message.split("\n")[0] }) },
    formatError: error => error instanceof Error ? error.message : String(error),
    cloudRelayApiUrl: apiUrl,
    getCloudRelayProfile: () => modules.relayApi.getKernelCloudRelayProfile(localClient),
    getCloudControlProfile: () => terminal.client.humanProfile(),
    saveCloudRelayProfile: saveProfile,
    startCloudDeviceLogin: (url, input) => modules.relayApi.startCloudRelayLogin(localClient, url, input),
    pollCloudDeviceLogin: (url, deviceCode) => modules.relayApi.pollCloudRelayLogin(localClient, url, deviceCode),
    connectCloudRelay: () => modules.relayApi.connectKernelCloudRelay(localClient),
    getRelayStatus: () => modules.relayApi.getRelayStatus(localClient),
    openExternalUrl: openUrl,
    handleClientCloudCommand: modules.createCloudClientCommands({
      client: terminal.client, isKernelConnected: () => true, apiUrl: () => apiUrl,
      notice: message => { notices.push(message); log("command-notice", { firstLine: message.split("\n")[0] }) },
      saveProfile: async () => {}, // Kernel profile and terminal profile are distinct.
      accountId: async () => (await modules.relayApi.getKernelCloudRelayProfile(localClient))?.accountId,
      refresh: async () => {}, openUrl,
    }),
    createCloudSessionInvite: (sessionId, options) => terminal.client.collaboration.createSessionInvite(sessionId, options),
    acceptCloudSessionInvite: token => terminal.client.collaboration.acceptSessionInvite(token),
    listCloudSessionMembers: sessionId => terminal.client.collaboration.sessionMembers(sessionId),
    listCloudCollaborators: () => terminal.client.collaboration.collaborators(),
    refreshWaitingRoomData: async () => {},
  }
}
