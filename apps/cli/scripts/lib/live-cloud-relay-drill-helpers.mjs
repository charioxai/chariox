import { spawn } from "node:child_process"
import { readFile, writeFile } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { resolveBuiltBinary } from "./drill-runtime-helpers.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..", "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const DEV_AUTH_SECRET = "arroba-cloud-live-drill-dev-auth-secret"

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

export function log(name, details = null) {
  if (details == null) console.log(`[cloud-relay-drill] ${name}`)
  else console.log(`[cloud-relay-drill] ${name}`, JSON.stringify(details))
}

export function assert(condition, message, details = null) {
  if (!condition) {
    throw new Error(`${message}${details == null ? "" : `\n${JSON.stringify(details, null, 2)}`}`)
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
  child.stdout.on("data", (chunk) => {
    for (const line of chunk.toString().trimEnd().split("\n").filter(Boolean)) {
      log(`${name}:stdout`, line)
    }
  })
  child.stderr.on("data", (chunk) => {
    for (const line of chunk.toString().trimEnd().split("\n").filter(Boolean)) {
      log(`${name}:stderr`, line)
    }
  })
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

export async function buildKernelIfNeeded() {
  const manifest = path.join(repoRoot, "apps/kernel/Cargo.toml")
  const binary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
  const result = await run("cargo", ["build", "--manifest-path", manifest, "--bin", "arroba-kernel"])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return await resolveBuiltBinary(binary, manifest, "arroba-kernel")
}

export async function terminateChild(child, signal = "SIGTERM") {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(5_000)])
  if (child.exitCode == null) {
    child.kill("SIGKILL")
    await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(2_000)])
  }
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

export async function waitForLocalDaemon(LocalIpcClient, requests, kernelUrl, workspace) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl)
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

export async function waitForRelayTarget(LocalIpcClient, requests, relayUrl, relayToken, targetDaemonAlias) {
  let lastError = null
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const client = new LocalIpcClient(relayUrl, {
      relayAuthToken: relayToken,
      targetDaemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      await Promise.race([
        client.send(requests.listSessionsRequest()),
        sleep(2_000).then(() => { throw new Error("probe timeout") }),
      ])
      await client.close()
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`relay target did not become reachable: ${lastError instanceof Error ? lastError.message : String(lastError)}`)
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

export async function loginCloudDrillUser(apiUrl, { email, accountSlug, clientId, clientAlias }) {
  const started = await postJson(`${apiUrl}/auth/device/start`, {
    clientId,
    clientAlias,
  })
  await postJsonWithHeaders(`${apiUrl}/auth/dev/device/approve`, {
    userCode: started.userCode,
    accountSlug,
    provider: "auth0",
    providerSubject: `auth0|${accountSlug}`,
    email,
    emailVerified: true,
    displayName: accountSlug,
  }, devDeviceApprovalHeaders())
  const polled = await postJson(`${apiUrl}/auth/device/poll`, {
    deviceCode: started.deviceCode,
  })
  assert(polled.status === "approved", "cloud drill login should be approved", polled)
  assert(polled.profile?.userId && polled.cloudSessionToken, "cloud drill login should return profile and session", polled)
  return {
    profile: polled.profile,
    cloudSessionToken: polled.cloudSessionToken,
  }
}

export async function issueSessionScopedClientToken(apiUrl, {
  sessionToken,
  accountId,
  realmId,
  subject,
  userId,
  clientId,
  sessionId,
  targetDaemonAlias,
}) {
  const runtime = await postJson(`${apiUrl}/relay/token`, {
    sessionToken,
    accountId,
    subject,
    subjectKind: "client",
    realmId,
    userId,
    clientId,
    sessionId,
    allowedTargets: [targetDaemonAlias],
  })
  assert(runtime.token, "session-scoped relay token should be returned", runtime)
  return runtime.token
}

export async function postJson(url, body) {
  return postJsonWithHeaders(url, body)
}

export async function postJsonWithHeaders(url, body, headers = {}) {
  const response = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json", ...headers },
    body: JSON.stringify(body),
  })
  if (!response.ok) {
    throw new Error(`POST ${url} failed with ${response.status}: ${await response.text()}`)
  }
  return response.json().catch(() => null)
}

export async function getJsonWithHeaders(url, headers = {}) {
  const response = await fetch(url, { headers })
  if (!response.ok) {
    throw new Error(`GET ${url} failed with ${response.status}: ${await response.text()}`)
  }
  return {
    body: await response.json(),
    headers: response.headers,
  }
}

export async function getJson(url) {
  return (await getJsonWithHeaders(url)).body
}

export async function waitForCloudRelayTarget(apiUrl, { accountId, realmId, daemonId, status }, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  let lastTargets = null
  while (Date.now() < deadline) {
    const listed = await getJson(
      `${apiUrl}/relay/targets?accountId=${encodeURIComponent(accountId)}&realmId=${encodeURIComponent(realmId)}`,
    )
    lastTargets = listed.targets ?? []
    const target = lastTargets.find((entry) => entry.daemonId === daemonId)
    if (target?.status === status) return target
    await sleep(250)
  }
  throw new Error(`timed out waiting for cloud relay target ${daemonId} to become ${status}\n${JSON.stringify(lastTargets, null, 2)}`)
}

export async function browserMutationHeaders(apiUrl, identity) {
  const csrf = await getJsonWithHeaders(`${apiUrl}/auth/csrf`)
  const csrfCookie = csrf.headers.get("set-cookie")
  assert(csrfCookie, "cloud csrf response should set a csrf cookie", csrf)
  return {
    cookie: csrfCookie,
    "csrf-token": csrf.body.csrfToken,
    "x-arroba-test-auth0-identity": JSON.stringify(identity),
  }
}

export function devDeviceApprovalHeaders() {
  return {
    "x-arroba-dev-auth-secret": DEV_AUTH_SECRET,
  }
}

export async function removePersistedCloudSessionToken(configHome) {
  const configPath = path.join(configHome, "arroba", "daemon", "config.json")
  const config = JSON.parse(await readFile(configPath, "utf8"))
  assert(config.cloud_relay?.machine_credential, "persisted cloud profile should include machine credential before token removal", config.cloud_relay)
  delete config.cloud_relay.cloud_session_token
  delete config.cloud_relay.cloud_session_expires_at_ms
  delete config.cloud_relay.token_expires_at_ms
  delete config.relay_url
  delete config.relay_token
  await writeFile(configPath, `${JSON.stringify(config, null, 2)}\n`, "utf8")
  return configPath
}

export function profileFromKernel(profile, expiresAt) {
  assert(profile, "kernel cloud login approval should return a profile")
  return {
    apiUrl: profile.api_url,
    email: profile.email,
    accountId: profile.account_id,
    userId: profile.user_id,
    accountSlug: profile.account_slug,
    realmId: profile.realm_id,
    relayUrl: profile.relay_url,
    issuerId: profile.issuer_id,
    ...(profile.client_id ? { clientId: profile.client_id } : {}),
    ...(profile.client_alias ? { clientAlias: profile.client_alias } : {}),
    ...(profile.machine_id ? { machineId: profile.machine_id } : {}),
    ...(profile.machine_alias ? { machineAlias: profile.machine_alias } : {}),
    ...(profile.machine_credential ? { machineCredential: profile.machine_credential } : {}),
    ...(profile.cloud_session_token ? { cloudSessionToken: profile.cloud_session_token } : {}),
    ...(expiresAt ? { cloudSessionExpiresAtMs: Date.parse(expiresAt) } : {}),
  }
}

export function tokenFromKernel(token, profile) {
  assert(token, "kernel cloud token response should include a token")
  return {
    relayUrl: token.relay_url,
    relayToken: token.relay_token,
    tokenExpiresAtMs: Date.parse(token.token_expires_at),
    profile: profile ? profileFromKernel(profile) : undefined,
  }
}

export function parseCloudClientTokenNotice(notices) {
  const notice = [...notices].reverse().find((item) => (
    item.startsWith("cloud relay client token\n") || item.startsWith("cloud client token\n")
  ))
  assert(notice, "cloud relay client-token command should append a token notice", notices)
  const fields = Object.fromEntries(
    notice
      .split("\n")
      .slice(1)
      .map((line) => {
        const index = line.indexOf("=")
        return index === -1 ? [line, ""] : [line.slice(0, index), line.slice(index + 1)]
      }),
  )
  const relayUrl = fields.relay_url ?? fields.transport
  assert(relayUrl, "client token notice should include relay_url or transport", fields)
  assert(fields.command, "client token notice should include command", fields)
  const tokenMatch = fields.command.match(/\s--relay-token\s+(\S+)/)
  assert(tokenMatch?.[1], "client token command should include --relay-token", fields.command)
  return {
    relayUrl,
    relayToken: tokenMatch[1],
  }
}

export function createMinimalCommandDeps({
  apiUrl,
  runId,
  workspace,
  clientId,
  localClient,
  requests,
  cloudRelay,
  profileRef,
  notices,
}) {
  return {
    workspace,
    worktree: workspace,
    clientId,
    isAttached: () => false,
    sessionState: () => ({ id: null, agents: [], workflows: [] }),
    attachmentState: () => null,
    providerRunState: () => null,
    currentModelId: () => "gpt-5.2",
    currentVariantId: () => "low",
    currentProviderId: () => "codex",
    focusedAgentId: () => null,
    multiAgentResponseLayout: () => "individual",
    maxAgentsPerScreen: () => 3,
    flashFooter: (message, tone) => log("command-footer", { tone, message }),
    appendNotice: (message) => {
      notices.push(message)
      log("command-notice", { firstLine: message.split("\n")[0] })
    },
    formatError: (error) => error instanceof Error ? error.message : String(error),
    cloudRelayApiUrl: apiUrl,
    getCloudRelayProfile: () => profileRef.current,
    saveCloudRelayProfile: async (profile) => {
      profileRef.current = profile
    },
    bootstrapCloudRelay: (apiUrl, email, accountSlug) => cloudRelay.bootstrapCloudRelayProfile({
      apiUrl,
      email,
      ...(accountSlug ? { accountSlug } : {}),
    }),
    pairCloudRelayClient: async (_profile, nextClientId, alias) => {
      const paired = unwrap(
        await localClient.send(requests.pairCloudRelayClientRequest(nextClientId, alias)),
        "CloudRelayClientPaired",
      )
      return profileFromKernel(paired.profile)
    },
    pairCloudRelayMachine: async (_profile, machineId, alias) => {
      const paired = unwrap(
        await localClient.send(requests.pairCloudRelayMachineRequest(machineId, alias)),
        "CloudRelayMachinePaired",
      )
      return profileFromKernel(paired.profile)
    },
    getRelayStatus: async () => unwrap(
      await localClient.send(requests.relayStatusRequest()),
      "RelayStatus",
    ).status,
    configureRelay: async (relayUrl, relayToken) => unwrap(
      await localClient.send(requests.configureRelayRequest(relayUrl, relayToken)),
      "RelayConfigured",
    ).status,
    startCloudDeviceLogin: async (nextApiUrl, input) => {
      log("kernel-cloud-login-start")
      const login = unwrap(
        await localClient.send(requests.startCloudRelayLoginRequest(nextApiUrl, input)),
        "CloudRelayLoginStarted",
      ).login
      profileRef.deviceUserCode = login.user_code
      return {
        apiUrl: login.api_url,
        deviceCode: login.device_code,
        userCode: login.user_code,
        verificationUrl: login.verification_url,
        expiresAtMs: Date.parse(login.expires_at),
        intervalSeconds: login.interval_seconds,
      }
    },
    pollCloudDeviceLogin: async (nextApiUrl, deviceCode) => {
      if (!profileRef.deviceApproved) {
        profileRef.deviceApproved = true
        log("cloud-device-approve")
        await postJsonWithHeaders(`${nextApiUrl}/auth/dev/device/approve`, {
          userCode: profileRef.deviceUserCode,
          accountSlug: runId,
          provider: "auth0",
          providerSubject: `auth0|${runId}`,
          email: `${runId}@example.com`,
          emailVerified: true,
          displayName: runId,
        }, devDeviceApprovalHeaders())
      }
      log("kernel-cloud-login-poll")
      const result = unwrap(
        await localClient.send(requests.pollCloudRelayLoginRequest(nextApiUrl, deviceCode)),
        "CloudRelayLoginPolled",
      ).result
      log("kernel-cloud-login-poll-result", { status: result.status })
      if (result.status === "authorization_pending") {
        return {
          status: "authorization_pending",
          intervalSeconds: result.interval_seconds ?? 1,
          expiresAtMs: result.expires_at ? Date.parse(result.expires_at) : Date.now() + 30_000,
        }
      }
      if (result.status === "expired_token") {
        return { status: "expired_token" }
      }
      return {
        status: "approved",
        profile: profileFromKernel(result.profile, result.expires_at),
      }
    },
    issueCloudKernelRelayToken: async () => {
      const connected = unwrap(
        await localClient.send(requests.connectCloudRelayRequest()),
        "CloudRelayConnected",
      )
      return tokenFromKernel(connected.token, connected.profile)
    },
    issueCloudMachineRelayToken: async () => {
      const connected = unwrap(
        await localClient.send(requests.connectCloudRelayRequest()),
        "CloudRelayConnected",
      )
      return tokenFromKernel(connected.token, connected.profile)
    },
    issueCloudClientRelayToken: async (_profile, targetDaemonAlias, options = {}) => {
      const issued = unwrap(
        await localClient.send(requests.issueCloudRelayClientTokenRequest(targetDaemonAlias, clientId, options.sessionId)),
        "CloudRelayClientTokenIssued",
      )
      return tokenFromKernel(issued.token, issued.profile)
    },
    refreshWaitingRoomData: async () => {},
  }
}
