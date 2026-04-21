#!/usr/bin/env node
import { spawn } from "node:child_process"
import { mkdir, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const cloudRoot = process.env.ARROBA_CLOUD_REPO
  ? path.resolve(process.env.ARROBA_CLOUD_REPO)
  : path.resolve(repoRoot, "..", "arroba-cloud")
const DATABASE_URL =
  process.env.DATABASE_URL ?? "postgresql://arroba:arroba@localhost:5432/arroba_cloud"
const CLOUD_SECRET = "arroba-cloud-live-drill-secret"
const CLOUD_ISSUER = "arroba-cloud-live-drill"

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function log(name, details = null) {
  if (details == null) console.log(`[cloud-relay-drill] ${name}`)
  else console.log(`[cloud-relay-drill] ${name}`, JSON.stringify(details))
}

function assert(condition, message, details = null) {
  if (!condition) {
    throw new Error(`${message}${details == null ? "" : `\n${JSON.stringify(details, null, 2)}`}`)
  }
}

async function run(command, args, options = {}) {
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

function spawnProcess(command, args, options) {
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

async function waitForHttp(url, timeoutMs = 30_000) {
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

async function buildKernelIfNeeded() {
  const binary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
  const result = await run("cargo", ["build", "--manifest-path", path.join(repoRoot, "apps/kernel/Cargo.toml"), "--bin", "arroba-kernel"])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return binary
}

async function terminateChild(child, signal = "SIGTERM") {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(5_000)])
  if (child.exitCode == null) {
    child.kill("SIGKILL")
    await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(2_000)])
  }
}

function makePorts() {
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

async function waitForLocalDaemon(LocalIpcClient, requests, kernelUrl, workspace) {
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

async function waitForRelayTarget(LocalIpcClient, requests, relayUrl, relayToken, targetDaemonAlias) {
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

function unwrap(resp, key) {
  return resp?.[key] ?? resp
}

async function expectReject(promise, label, expectedText) {
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

function addWorkflowNodeRequest(sessionId, workflowRef, agentId, expectedRevision = null) {
  return {
    AddWorkflowNode: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      agent_id: agentId,
      expected_workflow_revision: expectedRevision,
    },
  }
}

function updateWorkflowNodeInstructionsRequest(sessionId, workflowRef, nodeId, instructions, expectedRevision = null) {
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

function createWorkflowEndpointRequest(sessionId, workflowRef, entryNodeId, alias, expectedRevision = null) {
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

function addWorkflowEdgeRequest(sessionId, workflowRef, fromNodeId, toNodeId, expectedRevision = null) {
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

function removeWorkflowEdgeRequest(sessionId, workflowRef, edgeId, expectedRevision = null) {
  return {
    RemoveWorkflowEdge: {
      session_id: sessionId,
      workflow_ref: workflowRef,
      edge_id: edgeId,
      expected_workflow_revision: expectedRevision,
    },
  }
}

async function loginCloudDrillUser(apiUrl, { email, accountSlug, clientId, clientAlias }) {
  const started = await postJson(`${apiUrl}/auth/device/start`, {
    clientId,
    clientAlias,
  })
  await postJson(`${apiUrl}/auth/device/approve`, {
    userCode: started.userCode,
    email,
    accountSlug,
  })
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

async function issueSessionScopedClientToken(apiUrl, {
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

async function postJson(url, body) {
  const response = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  })
  if (!response.ok) {
    throw new Error(`POST ${url} failed with ${response.status}: ${await response.text()}`)
  }
  return response.json().catch(() => null)
}

function profileFromKernel(profile, expiresAt) {
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
    ...(profile.cloud_session_token ? { cloudSessionToken: profile.cloud_session_token } : {}),
    ...(expiresAt ? { cloudSessionExpiresAtMs: Date.parse(expiresAt) } : {}),
  }
}

function tokenFromKernel(token, profile) {
  assert(token, "kernel cloud token response should include a token")
  return {
    relayUrl: token.relay_url,
    relayToken: token.relay_token,
    tokenExpiresAtMs: Date.parse(token.token_expires_at),
    profile: profile ? profileFromKernel(profile) : undefined,
  }
}

function parseCloudClientTokenNotice(notices) {
  const notice = [...notices].reverse().find((item) => item.startsWith("cloud relay client token\n"))
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
  assert(fields.relay_url, "client token notice should include relay_url", fields)
  assert(fields.command, "client token notice should include command", fields)
  const tokenMatch = fields.command.match(/\s--relay-token\s+(\S+)/)
  assert(tokenMatch?.[1], "client token command should include --relay-token", fields.command)
  return {
    relayUrl: fields.relay_url,
    relayToken: tokenMatch[1],
  }
}

function createMinimalCommandDeps({
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
        await postJson(`${nextApiUrl}/auth/device/approve`, {
          userCode: profileRef.deviceUserCode,
          email: `${runId}@example.com`,
          accountSlug: runId,
        })
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

async function main() {
  const ports = makePorts()
  const runId = `cloud-relay-${process.pid}-${Date.now()}`
  const rootDir = path.join(os.tmpdir(), runId)
  const workspace = path.join(rootDir, "workspace")
  const daemonId = `cloud-daemon-${process.pid}-${Date.now()}`
  const daemonAlias = `cloud-home-${process.pid}`
  const clientId = `cloud-cli-${process.pid}-${Date.now()}`
  const apiUrl = `http://127.0.0.1:${ports.cloudPort}`
  await rm(rootDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(workspace, { recursive: true })

  const [{ LocalIpcClient }, requests, cloudRelay, commandActions, cloudDb] = await Promise.all([
    import("../../../packages/kernel-client/dist/ipc.js"),
    import("../../../packages/kernel-client/dist/ipc-requests.js"),
    import("../dist/cloud-relay.js"),
    import("../dist/command-actions.js"),
    import(path.join(cloudRoot, "packages/db/dist/index.js")),
  ])

  const kernelPath = await buildKernelIfNeeded()
  const relayEnv = {
    ...process.env,
    ARROBA_RELAY_HOST: "127.0.0.1",
    ARROBA_RELAY_PORT: String(ports.relayPort),
    ARROBA_RELAY_SCOPED_ISSUER: CLOUD_ISSUER,
    ARROBA_RELAY_SCOPED_HMAC_SECRET: CLOUD_SECRET,
  }
  const daemonEnv = {
    ...process.env,
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: daemonId,
    ARROBA_DAEMON_ALIAS: daemonAlias,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, "daemon.sock"),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, "session-history"),
  }
  const cloudEnv = {
    ...process.env,
    HOST: "127.0.0.1",
    PORT: String(ports.cloudPort),
    DATABASE_URL,
    ARROBA_CLOUD_RELAY_URL: `ws://127.0.0.1:${ports.relayPort}`,
    ARROBA_CLOUD_ISSUER_ID: CLOUD_ISSUER,
    ARROBA_CLOUD_RELAY_TOKEN_SECRET: CLOUD_SECRET,
  }

  let relay = null
  let daemon = null
  let cloudServer = null
  let localClient = null
  let remoteClient = null
  const db = cloudDb.createCloudDatabase({ databaseUrl: DATABASE_URL })

  try {
    log("build-cli")
    const cliBuild = await run("pnpm", ["run", "build"], { cwd: cliRoot, env: process.env })
    if (cliBuild.code !== 0) {
      throw new Error(`arroba cli build failed\n${cliBuild.stdout}\n${cliBuild.stderr}`)
    }

    log("build-cloud")
    const cloudBuild = await run("pnpm", ["run", "build"], { cwd: cloudRoot, env: cloudEnv })
    if (cloudBuild.code !== 0) {
      throw new Error(`arroba-cloud build failed\n${cloudBuild.stdout}\n${cloudBuild.stderr}`)
    }
    const migrate = await run("pnpm", ["--filter", "@arroba-cloud/db", "run", "prisma:migrate"], {
      cwd: cloudRoot,
      env: cloudEnv,
    })
    if (migrate.code !== 0) {
      throw new Error(`arroba-cloud migrate failed\n${migrate.stdout}\n${migrate.stderr}`)
    }

    log("start-cloud")
    cloudServer = spawnProcess("node", [path.join(cloudRoot, "apps/api/dist/server.js")], {
      cwd: cloudRoot,
      env: cloudEnv,
      name: "cloud-api",
    })
    await waitForHttp(`${apiUrl}/health`)

    log("start-relay-and-kernel")
    relay = spawnProcess("cargo", ["run", "--manifest-path", path.join(repoRoot, "apps/relay/Cargo.toml"), "--bin", "arroba-relay"], {
      cwd: repoRoot,
      env: relayEnv,
      name: "relay",
    })
    daemon = spawnProcess(kernelPath, [], { cwd: repoRoot, env: daemonEnv, name: "kernel" })

    const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}/kernel`
    await waitForLocalDaemon(LocalIpcClient, requests, kernelUrl, workspace)
    localClient = new LocalIpcClient(kernelUrl)

    const profileRef = { current: null }
    const notices = []
    const handlers = commandActions.createCommandActionHandlers(createMinimalCommandDeps({
      apiUrl,
      runId,
      workspace,
      clientId,
      localClient,
      requests,
      cloudRelay,
      profileRef,
      notices,
    }))

    log("command-cloud-login")
    await handlers.handleRelayCommand({
      kind: "relay",
      raw: "/relay cloud login",
      args: ["cloud", "login"],
    })
    assert(profileRef.current?.accountSlug === runId, "cloud login command should save the profile", profileRef.current)

    log("command-cloud-pair")
    await handlers.handleRelayCommand({
      kind: "relay",
      raw: "/relay cloud pair drill-cli",
      args: ["cloud", "pair", "drill-cli"],
    })
    assert(profileRef.current?.clientId === clientId, "cloud pair command should save client id", profileRef.current)

    log("command-cloud-pair-machine")
    await handlers.handleRelayCommand({
      kind: "relay",
      raw: `/relay cloud pair-machine ${daemonId} drill-machine`,
      args: ["cloud", "pair-machine", daemonId, "drill-machine"],
    })
    assert(profileRef.current?.machineId === daemonId, "cloud pair-machine command should save machine id", profileRef.current)

    log("command-cloud-connect")
    await handlers.handleRelayCommand({
      kind: "relay",
      raw: "/relay cloud connect",
      args: ["cloud", "connect"],
    })

    log("command-cloud-client-token")
    await handlers.handleRelayCommand({
      kind: "relay",
      raw: `/relay cloud client-token ${daemonAlias}`,
      args: ["cloud", "client-token", daemonAlias],
    })
    const clientRelay = parseCloudClientTokenNotice(notices)

    await waitForRelayTarget(
      LocalIpcClient,
      requests,
      clientRelay.relayUrl,
      clientRelay.relayToken,
      daemonAlias,
    )
    remoteClient = new LocalIpcClient(clientRelay.relayUrl, {
      relayAuthToken: clientRelay.relayToken,
      targetDaemonAlias: daemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })

    log("remote-session-create")
    const created = unwrap(
      await remoteClient.send(requests.createSessionRequest(workspace, workspace)),
      "SessionCreated",
    )
    assert(created.session?.id, "remote cloud session creation should return a session", created)

    const attached = unwrap(
      await remoteClient.send(requests.attachToSessionRequest(created.session.id, `${clientId}-remote`)),
      "SessionAttached",
    )
    assert(attached.attachment?.session_id === created.session.id, "remote cloud attach should bind to the created session", attached)

    const listed = unwrap(
      await remoteClient.send(requests.listSessionsRequest()),
      "SessionsListed",
    )
    assert(
      Array.isArray(listed.sessions) && listed.sessions.some((session) => session.id === created.session.id),
      "remote cloud client should list the created session",
      listed,
    )

    log("cloud-shared-session-invite")
    const localInvite = unwrap(
      await remoteClient.send(requests.createSessionInviteRequest(created.session.id, null, 3)),
      "SessionInviteCreated",
    )
    const cloudInvite = unwrap(
      await localClient.send(requests.createCloudSessionInviteRequest(created.session.id, {
        displayName: "Cloud relay shared session drill",
        maxUses: 3,
      })),
      "CloudSessionInviteCreated",
    )
    const localInviteToken = localInvite.invite?.invite_token
    const cloudInviteToken = cloudInvite.invite?.invite_token
    assert(localInviteToken, "local session invite token should be returned", localInvite)
    assert(cloudInviteToken, "cloud session invite token should be returned", cloudInvite)

    log("cloud-owner-session-scoped-token")
    const ownerScopedToken = await issueSessionScopedClientToken(apiUrl, {
      sessionToken: profileRef.current.cloudSessionToken,
      accountId: profileRef.current.accountId,
      realmId: profileRef.current.realmId,
      subject: clientId,
      userId: profileRef.current.userId,
      clientId,
      sessionId: created.session.id,
      targetDaemonAlias: daemonAlias,
    })
    const ownerScopedClient = new LocalIpcClient(profileRef.current.relayUrl, {
      relayAuthToken: ownerScopedToken,
      targetDaemonAlias: daemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })

    log("cloud-peer-login")
    const peerClientId = `${clientId}-peer`
    const peerLogin = await loginCloudDrillUser(apiUrl, {
      email: `${runId}-peer@example.com`,
      accountSlug: `${runId}-peer`,
      clientId: peerClientId,
      clientAlias: "drill-peer-cli",
    })
    const peerProfile = peerLogin.profile
    const peerCloudSessionToken = peerLogin.cloudSessionToken

    log("cloud-third-login")
    const thirdClientId = `${clientId}-third`
    const thirdLogin = await loginCloudDrillUser(apiUrl, {
      email: `${runId}-third@example.com`,
      accountSlug: `${runId}-third`,
      clientId: thirdClientId,
      clientAlias: "drill-third-cli",
    })
    const thirdProfile = thirdLogin.profile
    const thirdCloudSessionToken = thirdLogin.cloudSessionToken

    log("cloud-peer-accept-invite")
    const peerAcceptance = await postJson(`${apiUrl}/sessions/invites/${encodeURIComponent(cloudInviteToken)}/accept`, {
      sessionToken: peerCloudSessionToken,
    })
    assert(peerAcceptance.userId === peerProfile.userId, "peer should accept the cloud invite as itself", peerAcceptance)

    log("cloud-third-accept-invite")
    const thirdAcceptance = await postJson(`${apiUrl}/sessions/invites/${encodeURIComponent(cloudInviteToken)}/accept`, {
      sessionToken: thirdCloudSessionToken,
    })
    assert(thirdAcceptance.userId === thirdProfile.userId, "third user should accept the cloud invite as itself", thirdAcceptance)

    log("cloud-peer-session-scoped-token")
    const peerRelayToken = await issueSessionScopedClientToken(apiUrl, {
      sessionToken: peerCloudSessionToken,
      accountId: profileRef.current.accountId,
      realmId: profileRef.current.realmId,
      subject: peerClientId,
      userId: peerProfile.userId,
      clientId: peerClientId,
      sessionId: created.session.id,
      targetDaemonAlias: daemonAlias,
    })

    log("cloud-third-session-scoped-token")
    const thirdRelayToken = await issueSessionScopedClientToken(apiUrl, {
      sessionToken: thirdCloudSessionToken,
      accountId: profileRef.current.accountId,
      realmId: profileRef.current.realmId,
      subject: thirdClientId,
      userId: thirdProfile.userId,
      clientId: thirdClientId,
      sessionId: created.session.id,
      targetDaemonAlias: daemonAlias,
    })

    log("cloud-peer-relay-join")
    const peerRemoteClient = new LocalIpcClient(profileRef.current.relayUrl, {
      relayAuthToken: peerRelayToken,
      targetDaemonAlias: daemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    const thirdRemoteClient = new LocalIpcClient(profileRef.current.relayUrl, {
      relayAuthToken: thirdRelayToken,
      targetDaemonAlias: daemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      await peerRemoteClient.send(requests.joinSessionInviteRequest(localInviteToken, peerProfile.userId))
      await thirdRemoteClient.send(requests.joinSessionInviteRequest(localInviteToken, thirdProfile.userId))
      const peerAttached = unwrap(
        await peerRemoteClient.send(requests.attachToSessionRequest(created.session.id, `${peerClientId}-remote`)),
        "SessionAttached",
      )
      assert(peerAttached.attachment?.session_id === created.session.id, "peer should attach to joined session", peerAttached)
      const thirdAttached = unwrap(
        await thirdRemoteClient.send(requests.attachToSessionRequest(created.session.id, `${thirdClientId}-remote`)),
        "SessionAttached",
      )
      assert(thirdAttached.attachment?.session_id === created.session.id, "third user should attach to joined session", thirdAttached)
      const members = unwrap(
        await peerRemoteClient.send(requests.listSessionMembersRequest(created.session.id)),
        "SessionMembersListed",
      )
      assert(
        members.members?.some((member) => member.user_id === peerProfile.userId),
        "peer should appear in kernel session members after relay join",
        members,
      )
      assert(
        members.members?.some((member) => member.user_id === thirdProfile.userId),
        "third user should appear in kernel session members after relay join",
        members,
      )

      log("cloud-session-scoped-workflow-assertions")
      const ownerAgent = unwrap(
        await ownerScopedClient.send(requests.spawnAgentRequest(created.session.id, "dev-stub", "owner-agent", "multi-user-drill", workspace, "low")),
        "AgentSpawned",
      ).agent
      const peerAgent = unwrap(
        await peerRemoteClient.send(requests.spawnAgentRequest(created.session.id, "dev-stub", "peer-agent", "multi-user-drill", workspace, "low")),
        "AgentSpawned",
      ).agent
      assert(ownerAgent.owner_user_id === profileRef.current.userId, "owner agent should use owner cloud user id", ownerAgent)
      assert(peerAgent.owner_user_id === peerProfile.userId, "peer agent should use peer cloud user id", peerAgent)

      const peerAgents = unwrap(
        await peerRemoteClient.send(requests.listAgentsRequest(created.session.id)),
        "AgentsListed",
      ).agents
      assert(
        peerAgents.length === 1 && peerAgents[0].id === peerAgent.id,
        "peer should only list its own providers/agents through cloud-scoped relay token",
        peerAgents,
      )

      const workflow = unwrap(
        await ownerScopedClient.send(requests.createWorkflowRequest(created.session.id, "cloud-session-scoped-live-flow")),
        "WorkflowCreated",
      ).workflow
      const ownerNode = unwrap(
        await ownerScopedClient.send(addWorkflowNodeRequest(created.session.id, workflow.id, ownerAgent.id, workflow.revision)),
        "WorkflowNodeAdded",
      ).node
      await ownerScopedClient.send(updateWorkflowNodeInstructionsRequest(
        created.session.id,
        workflow.id,
        ownerNode.id,
        "private cloud owner prompt",
      ))

      await expectReject(
        peerRemoteClient.send(addWorkflowNodeRequest(created.session.id, workflow.id, ownerAgent.id)),
        "peer adding owner agent as workflow node through cloud-scoped relay token",
        "owned by",
      )

      const beforePeerNode = unwrap(
        await peerRemoteClient.send(requests.resolveWorkflowRequest(created.session.id, workflow.id)),
        "WorkflowResolved",
      ).workflow
      const peerNode = unwrap(
        await peerRemoteClient.send(addWorkflowNodeRequest(created.session.id, workflow.id, peerAgent.id, beforePeerNode.revision)),
        "WorkflowNodeAdded",
      ).node
      await peerRemoteClient.send(updateWorkflowNodeInstructionsRequest(
        created.session.id,
        workflow.id,
        peerNode.id,
        "private cloud peer prompt",
      ))

      const endpoint = unwrap(
        await ownerScopedClient.send(createWorkflowEndpointRequest(created.session.id, workflow.id, ownerNode.id, "owner-cloud-entry")),
        "WorkflowEndpointCreated",
      ).endpoint
      await expectReject(
        peerRemoteClient.send(requests.invokeWorkflowEndpointRequest(created.session.id, workflow.id, endpoint.id, "should be denied")),
        "peer invoking owner endpoint through cloud-scoped relay token",
        "owned by",
      )

      const beforeEdge = unwrap(
        await peerRemoteClient.send(requests.resolveWorkflowRequest(created.session.id, workflow.id)),
        "WorkflowResolved",
      ).workflow
      const edge = unwrap(
        await peerRemoteClient.send(addWorkflowEdgeRequest(created.session.id, workflow.id, ownerNode.id, peerNode.id, beforeEdge.revision)),
        "WorkflowEdgeAdded",
      ).edge
      assert(edge.created_by_user_id === peerProfile.userId, "cross-owner edge should record peer cloud user id", edge)

      await expectReject(
        thirdRemoteClient.send(removeWorkflowEdgeRequest(created.session.id, workflow.id, edge.id)),
        "third user removing edge unrelated to its nodes through cloud-scoped relay token",
        "cannot perform",
      )

      const beforeStaleMutation = unwrap(
        await ownerScopedClient.send(requests.resolveWorkflowRequest(created.session.id, workflow.id)),
        "WorkflowResolved",
      ).workflow
      await peerRemoteClient.send(updateWorkflowNodeInstructionsRequest(
        created.session.id,
        workflow.id,
        peerNode.id,
        "private cloud peer prompt after revision bump",
      ))
      await expectReject(
        ownerScopedClient.send(updateWorkflowNodeInstructionsRequest(
          created.session.id,
          workflow.id,
          ownerNode.id,
          "stale private cloud owner prompt",
          beforeStaleMutation.revision,
        )),
        "stale workflow revision mutation through cloud-scoped relay token",
        "expected",
      )

      const freshWorkflow = unwrap(
        await ownerScopedClient.send(requests.resolveWorkflowRequest(created.session.id, workflow.id)),
        "WorkflowResolved",
      ).workflow
      const removedWorkflow = unwrap(
        await ownerScopedClient.send(removeWorkflowEdgeRequest(created.session.id, workflow.id, edge.id, freshWorkflow.revision)),
        "WorkflowEdgeRemoved",
      ).workflow
      assert(removedWorkflow.edges.length === 0, "owner should remove edge incident to its own node", removedWorkflow)

      const peerStatePayload = unwrap(
        await peerRemoteClient.send(requests.getSessionStateRequest(created.session.id)),
        "SessionState",
      )
      const peerState = peerStatePayload.session ?? peerStatePayload.state ?? peerStatePayload
      assert(peerState.agents.length === 1 && peerState.agents[0].id === peerAgent.id, "peer state should redact owner agent", peerState.agents)
      const redactedWorkflow = peerState.workflows.find((entry) => entry.id === workflow.id)
      assert(redactedWorkflow, "peer should see shared workflow graph", peerState.workflows)
      const redactedOwnerNode = redactedWorkflow.nodes.find((node) => node.id === ownerNode.id)
      const visiblePeerNode = redactedWorkflow.nodes.find((node) => node.id === peerNode.id)
      assert(redactedOwnerNode, "peer should see owner node shell", redactedWorkflow)
      assert(visiblePeerNode, "peer should see own node", redactedWorkflow)
      assert(redactedOwnerNode.instructions == null, "owner node instructions should be redacted from peer", redactedOwnerNode)
      assert(
        visiblePeerNode.instructions === "private cloud peer prompt after revision bump",
        "peer node instructions should remain visible to owner",
        visiblePeerNode,
      )
    } finally {
      await thirdRemoteClient.close().catch(() => {})
      await peerRemoteClient.close().catch(() => {})
      await ownerScopedClient.close().catch(() => {})
    }

    console.log("live cloud relay drill passed")
  } finally {
    await remoteClient?.close().catch(() => {})
    await localClient?.close().catch(() => {})
    await terminateChild(daemon)
    await terminateChild(relay)
    await terminateChild(cloudServer)
    await db.account.deleteMany({ where: { slug: { in: [runId, `${runId}-peer`, `${runId}-third`] } } }).catch(() => {})
    await db.user.deleteMany({ where: { email: { in: [`${runId}@example.com`, `${runId}-peer@example.com`, `${runId}-third@example.com`] } } }).catch(() => {})
    await db.$disconnect().catch(() => {})
    await rm(rootDir, { recursive: true, force: true }).catch(() => {})
  }
}

await main()
