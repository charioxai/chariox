#!/usr/bin/env node
import { spawn } from "node:child_process"
import { mkdir, rm } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const apiUrl = (process.env.ARROBA_CLOUD_HOSTED_API_URL ?? "https://arroba-cloud-staging.osc-fr1.scalingo.io").replace(/\/$/, "")
const pollTimeoutMs = Number(process.env.ARROBA_CLOUD_HOSTED_POLL_TIMEOUT_MS ?? 10 * 60 * 1000)
const runMultiUser = process.env.ARROBA_CLOUD_HOSTED_MULTI_USER === "1"
const runSecondKernel = process.env.ARROBA_CLOUD_HOSTED_SECOND_KERNEL === "1"
const runRemoteCli = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI === "1"
const runRemoteCliPairing = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_PAIRING === "1"
const remoteCliPairingProvider = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_PROVIDER ?? "codex"
const remoteCliPairingModel = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_MODEL ?? "gpt-5.2-codex"
const remoteCliPairingEffort = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_EFFORT ?? "low"
const remoteCliHost = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_HOST ?? "root@195.201.123.115"
const remoteCliKey = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_KEY ?? path.join(os.homedir(), ".ssh/arroba_hetzner_staging")
const remoteCliRepo = process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_REPO ?? "/opt/arroba-cli-drill"
const devAuthSecret = process.env.ARROBA_CLOUD_DEV_AUTH_SECRET ?? ""

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function log(name, details = null) {
  if (details == null) console.log(`[hosted-cloud-relay-drill] ${name}`)
  else console.log(`[hosted-cloud-relay-drill] ${name}`, JSON.stringify(details))
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
  if (options.logStdout !== false) {
    child.stdout.on("data", (chunk) => {
      for (const line of chunk.toString().trimEnd().split("\n").filter(Boolean)) {
        log(`${name}:stdout`, line)
      }
    })
  }
  if (options.logStderr !== false) {
    child.stderr.on("data", (chunk) => {
      for (const line of chunk.toString().trimEnd().split("\n").filter(Boolean)) {
        log(`${name}:stderr`, line)
      }
    })
  }
  child.on("exit", (code, signal) => {
    log(`${name}:exit`, { code, signal })
  })
  return child
}

function shellQuote(value) {
  return `'${String(value).replace(/'/g, "'\\''")}'`
}

function sshArgs(command, options = {}) {
  const args = [
    "-i",
    options.key ?? remoteCliKey,
    "-o",
    "IdentitiesOnly=yes",
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=10",
  ]
  if (options.tty) args.push("-tt")
  args.push(options.host ?? remoteCliHost, command)
  return args
}

async function runSsh(command, options = {}) {
  return await run("ssh", sshArgs(command, options), {
    cwd: options.cwd ?? repoRoot,
    env: options.env ?? process.env,
  })
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

async function closeClient(client, label) {
  if (!client) return
  let timedOut = false
  await Promise.race([
    client.close().catch(() => {}),
    sleep(2_000).then(() => {
      timedOut = true
      log("client-close-timeout", { label })
    }),
  ])
  if (timedOut) {
    client.controlWebsocket?.terminate?.()
    client.eventWebsocket?.terminate?.()
  }
}

async function buildKernelIfNeeded() {
  const binary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
  const result = await run("cargo", ["build", "--manifest-path", path.join(repoRoot, "apps/kernel/Cargo.toml"), "--bin", "arroba-kernel"])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return binary
}

async function getFreePort() {
  return await new Promise((resolve, reject) => {
    const server = net.createServer()
    server.on("error", reject)
    server.listen(0, "127.0.0.1", () => {
      const address = server.address()
      server.close(() => {
        if (address && typeof address === "object") {
          resolve(address.port)
        } else {
          reject(new Error("failed to allocate a free port"))
        }
      })
    })
  })
}

async function makePorts() {
  return {
    kernelPort: await getFreePort(),
    mcpPort: await getFreePort(),
    opencodePort: await getFreePort(),
    codexPort: await getFreePort(),
  }
}

async function makeWorkerPorts() {
  return {
    kernelPort: await getFreePort(),
    mcpPort: await getFreePort(),
    opencodePort: await getFreePort(),
    codexPort: await getFreePort(),
  }
}

function unwrap(resp, key) {
  return resp?.[key] ?? resp
}

function profileFromKernel(profile, expiresAt) {
  assert(profile, "kernel cloud response should include a profile")
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
  const tokenMatch = fields.command?.match(/\s--relay-token\s+(\S+)/)
  assert(tokenMatch?.[1], "client token command should include --relay-token", fields.command)
  return {
    relayUrl,
    relayToken: tokenMatch[1],
  }
}

async function postJson(url, body, headers = {}) {
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

async function createPairingToken({ accountId, userId, subjectKind }) {
  const response = await postJson(`${apiUrl}/pairing-tokens`, {
    accountId,
    createdByUserId: userId,
    subjectKind,
  })
  assert(response?.token, "cloud pairing token should be returned", response)
  return response.token
}

async function pairCloudMachineDirect({ profile, machineId, alias }) {
  const token = await createPairingToken({
    accountId: profile.accountId,
    userId: profile.userId,
    subjectKind: "machine",
  })
  const response = await postJson(`${apiUrl}/machines/pair`, {
    accountId: profile.accountId,
    token,
    machineId,
    userId: profile.userId,
    alias,
  })
  assert(response?.machineId === machineId, "cloud machine pair should return the paired machine id", response)
  return response
}

async function issueMachineRelayToken({ profile, machineId }) {
  const response = await postJson(`${apiUrl}/relay/token`, {
    sessionToken: profile.cloudSessionToken,
    accountId: profile.accountId,
    subject: machineId,
    subjectKind: "machine",
    realmId: profile.realmId,
    userId: profile.userId,
    machineId,
  })
  assert(response?.token, "machine relay token should be returned", response)
  return response.token
}

async function approveDevDeviceLogin({ role, userCode, accountSlug }) {
  if (!devAuthSecret) return false
  const slug = accountSlug ?? `hosted-${role}-${process.pid}-${Date.now()}`
  const email = `${slug}@arroba.local`
  log(`${role}-dev-approve-cloud-login`, { accountSlug: slug, email })
  await postJson(`${apiUrl}/auth/dev/device/approve`, {
    userCode,
    email,
    accountSlug: slug,
    displayName: `Hosted ${role} drill`,
    providerSubject: `dev|${slug}`,
  }, {
    "x-arroba-dev-auth-secret": devAuthSecret,
  })
  return true
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

async function allowDevStubProvider(client, requests, label) {
  log("allow-dev-stub-provider", { label })
  await client.send(requests.setUserConfigValueRequest("providers.managed_io", "unrestricted"))
}

async function manualCloudDeviceLogin({ role, clientId, clientAlias, localClient, requests }) {
  log(`${role}-cloud-login-start`, { apiUrl })
  const login = unwrap(
    await localClient.send(requests.startCloudRelayLoginRequest(apiUrl, {
      clientId,
      clientAlias,
    })),
    "CloudRelayLoginStarted",
  ).login
  const expiresAtMs = Math.min(Date.parse(login.expires_at), Date.now() + pollTimeoutMs)
  log(`${role}-approve-cloud-login`, {
    verificationUrl: login.verification_url,
    userCode: login.user_code,
    expiresAt: login.expires_at,
  })
  await approveDevDeviceLogin({ role, userCode: login.user_code })
  while (Date.now() < expiresAtMs) {
    const result = unwrap(
      await localClient.send(requests.pollCloudRelayLoginRequest(apiUrl, login.device_code)),
      "CloudRelayLoginPolled",
    ).result
    log(`${role}-cloud-login-poll-result`, { status: result.status })
    if (result.status === "approved") {
      assert(result.profile?.cloud_session_token, `${role} cloud login should return a cloud session token`, result)
      return {
        profile: profileFromKernel(result.profile, result.expires_at),
        cloudSessionToken: result.profile.cloud_session_token,
      }
    }
    if (result.status === "expired_token") {
      throw new Error(`${role} cloud login expired`)
    }
    await sleep(Math.max(result.interval_seconds ?? 2, 1) * 1000)
  }
  throw new Error(`${role} cloud login timed out`)
}

async function runHostedMultiUserAssertions({
  LocalIpcClient,
  requests,
  localClient,
  ownerRemoteClient,
  ownerProfile,
  ownerClientId,
  workspace,
  daemonAlias,
  session,
}) {
  log("multi-user-cloud-invites")
  const localInvite = unwrap(
    await ownerRemoteClient.send(requests.createSessionInviteRequest(session.id, null, 2)),
    "SessionInviteCreated",
  )
  const cloudInvite = unwrap(
    await ownerRemoteClient.send(requests.createCloudSessionInviteRequest(session.id, {
      displayName: "Hosted cloud relay multi-user drill",
      maxUses: 2,
    })),
    "CloudSessionInviteCreated",
  )
  const localInviteToken = localInvite.invite?.invite_token
  const cloudInviteToken = cloudInvite.invite?.invite_token
  assert(localInviteToken, "local session invite token should be returned", localInvite)
  assert(cloudInviteToken, "cloud session invite token should be returned", cloudInvite)

  const ownerScopedToken = await issueSessionScopedClientToken(apiUrl, {
    sessionToken: ownerProfile.cloudSessionToken,
    accountId: ownerProfile.accountId,
    realmId: ownerProfile.realmId,
    subject: ownerClientId,
    userId: ownerProfile.userId,
    clientId: ownerClientId,
    sessionId: session.id,
    targetDaemonAlias: daemonAlias,
  })
  const ownerScopedClient = installSendRetry(new LocalIpcClient(ownerProfile.relayUrl, {
    relayAuthToken: ownerScopedToken,
    targetDaemonAlias: daemonAlias,
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  }), "owner-scoped-relay")

  const peerClientId = `${ownerClientId}-peer`
  const thirdClientId = `${ownerClientId}-third`
  let peerRemoteClient = null
  let thirdRemoteClient = null
  try {
    const peerLogin = await manualCloudDeviceLogin({
      role: "peer",
      clientId: peerClientId,
      clientAlias: "hosted-peer-cli",
      localClient,
      requests,
    })
    const thirdLogin = await manualCloudDeviceLogin({
      role: "third",
      clientId: thirdClientId,
      clientAlias: "hosted-third-cli",
      localClient,
      requests,
    })
    const peerProfile = peerLogin.profile
    const thirdProfile = thirdLogin.profile
    assert(peerProfile.userId !== ownerProfile.userId, "peer login must use a different Auth0 user from owner", {
      ownerUserId: ownerProfile.userId,
      peerUserId: peerProfile.userId,
    })
    assert(thirdProfile.userId !== ownerProfile.userId && thirdProfile.userId !== peerProfile.userId, "third login must use a distinct Auth0 user", {
      ownerUserId: ownerProfile.userId,
      peerUserId: peerProfile.userId,
      thirdUserId: thirdProfile.userId,
    })

    log("peer-accept-cloud-invite")
    const peerAcceptance = await postJson(`${apiUrl}/sessions/invites/${encodeURIComponent(cloudInviteToken)}/accept`, {
      sessionToken: peerLogin.cloudSessionToken,
    })
    assert(peerAcceptance.userId === peerProfile.userId, "peer should accept the cloud invite as itself", peerAcceptance)

    log("third-accept-cloud-invite")
    const thirdAcceptance = await postJson(`${apiUrl}/sessions/invites/${encodeURIComponent(cloudInviteToken)}/accept`, {
      sessionToken: thirdLogin.cloudSessionToken,
    })
    assert(thirdAcceptance.userId === thirdProfile.userId, "third user should accept the cloud invite as itself", thirdAcceptance)

    const peerRelayToken = await issueSessionScopedClientToken(apiUrl, {
      sessionToken: peerLogin.cloudSessionToken,
      accountId: ownerProfile.accountId,
      realmId: ownerProfile.realmId,
      subject: peerClientId,
      userId: peerProfile.userId,
      clientId: peerClientId,
      sessionId: session.id,
      targetDaemonAlias: daemonAlias,
    })
    const thirdRelayToken = await issueSessionScopedClientToken(apiUrl, {
      sessionToken: thirdLogin.cloudSessionToken,
      accountId: ownerProfile.accountId,
      realmId: ownerProfile.realmId,
      subject: thirdClientId,
      userId: thirdProfile.userId,
      clientId: thirdClientId,
      sessionId: session.id,
      targetDaemonAlias: daemonAlias,
    })

    peerRemoteClient = installSendRetry(new LocalIpcClient(ownerProfile.relayUrl, {
      relayAuthToken: peerRelayToken,
      targetDaemonAlias: daemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    }), "peer-relay")
    thirdRemoteClient = installSendRetry(new LocalIpcClient(ownerProfile.relayUrl, {
      relayAuthToken: thirdRelayToken,
      targetDaemonAlias: daemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    }), "third-relay")

    await peerRemoteClient.send(requests.joinSessionInviteRequest(localInviteToken, peerProfile.userId))
    await thirdRemoteClient.send(requests.joinSessionInviteRequest(localInviteToken, thirdProfile.userId))
    const peerAttached = unwrap(
      await peerRemoteClient.send(requests.attachToSessionRequest(session.id, `${peerClientId}-remote`)),
      "SessionAttached",
    )
    assert(peerAttached.attachment?.session_id === session.id, "peer should attach to joined session", peerAttached)
    const thirdAttached = unwrap(
      await thirdRemoteClient.send(requests.attachToSessionRequest(session.id, `${thirdClientId}-remote`)),
      "SessionAttached",
    )
    assert(thirdAttached.attachment?.session_id === session.id, "third user should attach to joined session", thirdAttached)
    const members = unwrap(
      await peerRemoteClient.send(requests.listSessionMembersRequest(session.id)),
      "SessionMembersListed",
    )
    assert(members.members?.some((member) => member.user_id === peerProfile.userId), "peer should appear in kernel session members", members)
    assert(members.members?.some((member) => member.user_id === thirdProfile.userId), "third should appear in kernel session members", members)

    const ownerAgent = unwrap(
      await ownerScopedClient.send(requests.spawnAgentRequest(session.id, "dev-stub", "owner-agent", "multi-user-drill", workspace, "low")),
      "AgentSpawned",
    ).agent
    const peerAgent = unwrap(
      await peerRemoteClient.send(requests.spawnAgentRequest(session.id, "dev-stub", "peer-agent", "multi-user-drill", workspace, "low")),
      "AgentSpawned",
    ).agent
    assert(ownerAgent.owner_user_id === ownerProfile.userId, "owner agent should use owner cloud user id", ownerAgent)
    assert(peerAgent.owner_user_id === peerProfile.userId, "peer agent should use peer cloud user id", peerAgent)

    const peerAgents = unwrap(
      await peerRemoteClient.send(requests.listAgentsRequest(session.id)),
      "AgentsListed",
    ).agents
    assert(peerAgents.length === 1 && peerAgents[0].id === peerAgent.id, "peer should only list its own agents", peerAgents)

    const workflow = unwrap(
      await ownerScopedClient.send(requests.createWorkflowRequest(session.id, "hosted-cloud-session-scoped-flow")),
      "WorkflowCreated",
    ).workflow
    const ownerNode = unwrap(
      await ownerScopedClient.send(addWorkflowNodeRequest(session.id, workflow.id, ownerAgent.id, workflow.revision)),
      "WorkflowNodeAdded",
    ).node
    await ownerScopedClient.send(updateWorkflowNodeInstructionsRequest(
      session.id,
      workflow.id,
      ownerNode.id,
      "private hosted owner prompt",
    ))
    await expectReject(
      peerRemoteClient.send(addWorkflowNodeRequest(session.id, workflow.id, ownerAgent.id)),
      "peer adding owner agent as workflow node",
      "owned by",
    )

    const beforePeerNode = unwrap(
      await peerRemoteClient.send(requests.resolveWorkflowRequest(session.id, workflow.id)),
      "WorkflowResolved",
    ).workflow
    const peerNode = unwrap(
      await peerRemoteClient.send(addWorkflowNodeRequest(session.id, workflow.id, peerAgent.id, beforePeerNode.revision)),
      "WorkflowNodeAdded",
    ).node
    await peerRemoteClient.send(updateWorkflowNodeInstructionsRequest(
      session.id,
      workflow.id,
      peerNode.id,
      "private hosted peer prompt",
    ))
    const endpoint = unwrap(
      await ownerScopedClient.send(createWorkflowEndpointRequest(session.id, workflow.id, ownerNode.id, "owner-hosted-entry")),
      "WorkflowEndpointCreated",
    ).endpoint
    await expectReject(
      peerRemoteClient.send(requests.invokeWorkflowEndpointRequest(session.id, workflow.id, endpoint.id, "should be denied")),
      "peer invoking owner endpoint",
      "owned by",
    )

    const beforeEdge = unwrap(
      await peerRemoteClient.send(requests.resolveWorkflowRequest(session.id, workflow.id)),
      "WorkflowResolved",
    ).workflow
    const edge = unwrap(
      await peerRemoteClient.send(addWorkflowEdgeRequest(session.id, workflow.id, ownerNode.id, peerNode.id, beforeEdge.revision)),
      "WorkflowEdgeAdded",
    ).edge
    assert(edge.created_by_user_id === peerProfile.userId, "cross-owner edge should record peer cloud user id", edge)
    await expectReject(
      thirdRemoteClient.send(removeWorkflowEdgeRequest(session.id, workflow.id, edge.id)),
      "third user removing unrelated edge",
      "cannot perform",
    )

    const beforeStaleMutation = unwrap(
      await ownerScopedClient.send(requests.resolveWorkflowRequest(session.id, workflow.id)),
      "WorkflowResolved",
    ).workflow
    await peerRemoteClient.send(updateWorkflowNodeInstructionsRequest(
      session.id,
      workflow.id,
      peerNode.id,
      "private hosted peer prompt after revision bump",
    ))
    await expectReject(
      ownerScopedClient.send(updateWorkflowNodeInstructionsRequest(
        session.id,
        workflow.id,
        ownerNode.id,
        "stale private hosted owner prompt",
        beforeStaleMutation.revision,
      )),
      "stale workflow revision mutation",
      "expected",
    )

    const freshWorkflow = unwrap(
      await ownerScopedClient.send(requests.resolveWorkflowRequest(session.id, workflow.id)),
      "WorkflowResolved",
    ).workflow
    const removedWorkflow = unwrap(
      await ownerScopedClient.send(removeWorkflowEdgeRequest(session.id, workflow.id, edge.id, freshWorkflow.revision)),
      "WorkflowEdgeRemoved",
    ).workflow
    assert(removedWorkflow.edges.length === 0, "owner should remove edge incident to its own node", removedWorkflow)

    const peerStatePayload = unwrap(
      await peerRemoteClient.send(requests.getSessionStateRequest(session.id)),
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
      visiblePeerNode.instructions === "private hosted peer prompt after revision bump",
      "peer node instructions should remain visible to peer",
      visiblePeerNode,
    )
  } finally {
    await thirdRemoteClient?.close().catch(() => {})
    await peerRemoteClient?.close().catch(() => {})
    await ownerScopedClient.close().catch(() => {})
  }
}

async function waitForRelayTarget(LocalIpcClient, requests, relayUrl, relayToken, targetDaemonAlias) {
  let lastError = null
  for (let attempt = 0; attempt < 30; attempt += 1) {
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
      if (attempt === 0 || attempt % 10 === 9) {
        log("relay-target-wait-retry", {
          targetDaemonAlias,
          attempt: attempt + 1,
          error: error instanceof Error ? error.message : String(error),
        })
      }
      await sleep(250)
    }
  }
  throw new Error(`relay target did not become reachable: ${lastError instanceof Error ? lastError.message : String(lastError)}`)
}

async function sendWithRetry(client, request, label, attempts = 5) {
  let lastError = null
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      return await client.send(request)
    } catch (error) {
      lastError = error
      if (!isRetryableClientSendError(error) || attempt === attempts - 1) {
        break
      }
      log("client-send-retry", {
        label,
        attempt: attempt + 1,
        error: error instanceof Error ? error.message : String(error),
      })
      await sleep(1_000 * (attempt + 1))
    }
  }
  throw lastError
}

function isRetryableClientSendError(error) {
  if (error?.retryable === true) {
    return true
  }
  const message = error instanceof Error ? error.message : String(error)
  return /ETIMEDOUT|ECONNRESET|ECONNREFUSED|socket hang up|websocket closed|connection_closed/i.test(message)
}

function installSendRetry(client, label) {
  const send = client.send.bind(client)
  client.send = (request) => sendWithRetry({ send }, request, label)
  return client
}

async function handleRelayCommandWithRetry(handlers, command, label, attempts = 3) {
  let lastError = null
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      return await handlers.handleRelayCommand(command)
    } catch (error) {
      lastError = error
      if (attempt === attempts - 1) {
        break
      }
      log("relay-command-retry", {
        label,
        attempt: attempt + 1,
        error: error instanceof Error ? error.message : String(error),
      })
      await sleep(1_000 * (attempt + 1))
    }
  }
  throw lastError
}

async function waitForRemoteMachine(client, requests, machineRef) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    try {
      const listed = unwrap(
        await Promise.race([
          client.send(requests.listRemoteMachineKernelsRequest(machineRef)),
          sleep(15_000).then(() => { throw new Error("remote machine kernel list timeout") }),
        ]),
        "RemoteMachineKernelsListed",
      )
      if ((listed.kernels ?? []).some((kernel) => kernel.accepting_remote_leases)) {
        return listed
      }
    } catch (error) {
      lastError = error
      if (attempt === 0 || attempt % 10 === 9) {
        log("remote-machine-wait-retry", {
          machineRef,
          attempt: attempt + 1,
          error: error instanceof Error ? error.message : String(error),
        })
      }
    }
    await sleep(500)
  }
  throw new Error(`remote machine ${machineRef} did not become ready: ${lastError instanceof Error ? lastError.message : String(lastError)}`)
}

async function waitForCompletion(eventLog, timeoutMs, baselineCount = 0) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const completions = eventLog.filter((event) => event.event === "assistant_message_completed")
    if (completions.length > baselineCount) {
      return completions[completions.length - 1]
    }
    await sleep(100)
  }
  throw new Error("timed out waiting for assistant completion")
}

async function waitForHistoryText(client, requests, sessionId, agentId, needle, timeoutMs, pollMs = 2_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const history = unwrap(
      await client.send(requests.getSessionHistoryRequest(sessionId, 60, 120_000, null, agentId ?? null)),
      "SessionHistory",
    )
    const text = (history.entries ?? [])
      .map((entry) => String(entry.entry?.text ?? entry.text ?? ""))
      .join("")
    if (text.includes(needle)) return text
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for history text ${needle}`)
}

async function waitForSession(client, requests, sessionId, timeoutMs = 20_000, pollMs = 500) {
  const deadline = Date.now() + timeoutMs
  let lastListed = null
  while (Date.now() < deadline) {
    const listed = unwrap(
      await client.send(requests.listSessionsRequest()),
      "SessionsListed",
    )
    lastListed = listed
    const session = (listed.sessions ?? []).find((candidate) => candidate.id === sessionId)
    if (session) return session
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for session ${sessionId}\n${JSON.stringify(lastListed, null, 2)}`)
}

async function waitForLocalSocket(socketPath, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const socket = net.createConnection(socketPath)
      await new Promise((resolve, reject) => {
        socket.once("connect", resolve)
        socket.once("error", reject)
      })
      socket.destroy()
      return
    } catch (error) {
      lastError = error
      await sleep(250)
    }
  }
  throw new Error(`local automation socket did not become ready: ${lastError instanceof Error ? lastError.message : String(lastError)}`)
}

function createAutomationClient(socketPath) {
  const socket = net.createConnection(socketPath)
  socket.setEncoding("utf8")
  let nextId = 1
  let buffer = ""
  const pending = new Map()
  socket.on("data", (chunk) => {
    buffer += chunk
    while (buffer.includes("\n")) {
      const newline = buffer.indexOf("\n")
      const line = buffer.slice(0, newline).trim()
      buffer = buffer.slice(newline + 1)
      if (!line) continue
      const response = JSON.parse(line)
      const deferred = pending.get(response.id)
      if (!deferred) continue
      pending.delete(response.id)
      if (response.ok) deferred.resolve(response.data)
      else deferred.reject(new Error(response.error ?? "automation command failed"))
    }
  })
  socket.on("error", (error) => {
    for (const deferred of pending.values()) deferred.reject(error)
    pending.clear()
  })
  return {
    send(action, fields = {}) {
      const id = nextId++
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject })
        socket.write(`${JSON.stringify({ id, action, ...fields })}\n`)
      })
    },
    close() {
      socket.destroy()
    },
  }
}

async function waitForRemoteSocket(socketPath, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    const result = await runSsh(`[ -S ${shellQuote(socketPath)} ]`)
    if (result.code === 0) return
    lastError = result.stderr || result.stdout || `exit ${result.code}`
    await sleep(250)
  }
  throw new Error(`remote automation socket did not become ready: ${lastError}`)
}

async function remoteAutomation(socketPath, action, fields = {}) {
  const request = JSON.stringify({ id: 1, action, ...fields })
  const code = `
const net = require("node:net");
const socketPath = process.argv[1];
const request = JSON.parse(process.argv[2]);
const socket = net.createConnection(socketPath);
socket.setEncoding("utf8");
let buffer = "";
socket.on("data", (chunk) => {
  buffer += chunk;
  const newline = buffer.indexOf("\\n");
  if (newline === -1) return;
  console.log(buffer.slice(0, newline));
  socket.end();
});
socket.on("error", (error) => {
  console.error(error.message);
  process.exit(1);
});
socket.write(JSON.stringify(request) + "\\n");
`
  const result = await runSsh(
    `export PATH=/root/.bun/bin:/opt/node-v22/bin:$PATH; node -e ${shellQuote(code)} ${shellQuote(socketPath)} ${shellQuote(request)}`,
  )
  if (result.code !== 0) {
    throw new Error(`remote automation ${action} failed\n${result.stdout}\n${result.stderr}`)
  }
  const line = result.stdout.trim().split("\n").filter(Boolean).at(-1)
  assert(line, `remote automation ${action} should return a response`, result)
  const response = JSON.parse(line)
  if (!response.ok) {
    throw new Error(`remote automation ${action} rejected: ${response.error ?? "unknown error"}`)
  }
  return response.data
}

async function runHostedRemoteCliPairingAssertions({
  requests,
  homeClient,
  verificationClient,
  workspace,
  kernelUrl,
}) {
  const remoteId = `${process.pid}-${Date.now()}`
  const localAlias = `hosted-pairing-local-cli-${remoteId}`
  const remoteAlias = `hosted-pairing-cli-${remoteId}`
  const remoteWorkspace = `/tmp/arroba-hosted-pairing-cli-${remoteId}`
  const localSocket = path.join(os.tmpdir(), `arroba-hosted-local-cli-${remoteId}.sock`)
  const remoteSocket = `/tmp/arroba-hosted-pairing-cli-${remoteId}.sock`
  const localMarker = `HOSTED_PAIRING_LOCAL_CLI_OK_${remoteId.replace(/[^a-zA-Z0-9]/g, "_")}`
  const remoteMarker = `HOSTED_PAIRING_REMOTE_CLI_OK_${remoteId.replace(/[^a-zA-Z0-9]/g, "_")}`
  const pairing = unwrap(
    await homeClient.send(requests.createTerminalPairingLinkRequest("cli", remoteAlias, 15 * 60 * 1000)),
    "TerminalPairingLinkCreated",
  ).pairing
  assert(pairing?.pairing_link, "terminal pairing link should be created", pairing)
  assert(pairing.terminal_id, "terminal pairing should include terminal id", pairing)

  const remoteCommand = [
    "set -e",
    "export PATH=/root/.bun/bin:/opt/node-v22/bin:$PATH",
    "export ARROBA_TEST_TUI=1",
    `mkdir -p ${shellQuote(remoteWorkspace)}`,
    `cd ${shellQuote(path.posix.join(remoteCliRepo, "apps/cli"))}`,
    [
      "bun",
      "dist/index.js",
      "--terminal-pairing-link",
      shellQuote(pairing.pairing_link),
      "--automation-socket",
      shellQuote(remoteSocket),
      "--create-session",
      "--alias",
      shellQuote(remoteAlias),
      "--workspace",
      shellQuote(workspace),
      "--worktree",
      shellQuote(workspace),
      "--provider",
      shellQuote(remoteCliPairingProvider),
      "--model",
      shellQuote(remoteCliPairingModel),
      "--effort",
      shellQuote(remoteCliPairingEffort),
    ].join(" "),
  ].join("; ")

  let localCli = null
  let localAutomation = null
  let remoteCli = null
  try {
    log("local-cli-pairing-start", {
      alias: localAlias,
      provider: remoteCliPairingProvider,
      model: remoteCliPairingModel,
    })
    localCli = spawnProcess("script", [
      "-q",
      "/dev/null",
      "env",
      "ARROBA_TEST_TUI=1",
      "bun",
      path.join(cliRoot, "dist/index.js"),
      "--kernel-url",
      kernelUrl,
      "--automation-socket",
      localSocket,
      "--create-session",
      "--alias",
      localAlias,
      "--workspace",
      workspace,
      "--worktree",
      workspace,
      "--provider",
      remoteCliPairingProvider,
      "--model",
      remoteCliPairingModel,
      "--effort",
      remoteCliPairingEffort,
      "--client-id",
      `hosted-local-pairing-cli-${remoteId}`,
    ], {
      cwd: repoRoot,
      env: process.env,
      name: "local-cli-pairing",
      logStdout: false,
    })
    await waitForLocalSocket(localSocket)
    localAutomation = createAutomationClient(localSocket)
    await localAutomation.send("ping")
    const localSnapshot = await localAutomation.send("wait_for", { screen: "agents", timeoutMs: 10_000 })
    assert(localSnapshot.session?.id, "local TUI should create and attach to a session", localSnapshot)
    assert(
      localSnapshot.session?.focusedAgentId,
      "local TUI should create a focused real-provider agent",
      localSnapshot,
    )

    log("remote-cli-pairing-start", {
      host: remoteCliHost,
      repo: remoteCliRepo,
      alias: remoteAlias,
      terminalId: pairing.terminal_id,
      provider: remoteCliPairingProvider,
      model: remoteCliPairingModel,
    })
    remoteCli = spawnProcess("ssh", sshArgs(remoteCommand, { tty: true }), {
      cwd: repoRoot,
      env: process.env,
      name: "remote-cli-pairing",
      logStdout: false,
    })
    try {
      await waitForRemoteSocket(remoteSocket)
    } catch (error) {
      throw new Error(`${error instanceof Error ? error.message : String(error)}\nremote alias: ${remoteAlias}`)
    }

    await remoteAutomation(remoteSocket, "ping")
    const remoteSnapshot = await remoteAutomation(remoteSocket, "wait_for", { screen: "agents", timeoutMs: 10_000 })
    assert(remoteSnapshot.session?.id, "paired orphan CLI should attach to a session", remoteSnapshot)
    assert(
      remoteSnapshot.session?.focusedAgentId,
      "paired orphan CLI should create a focused real-provider agent",
      remoteSnapshot,
    )

    const terminals = unwrap(
      await homeClient.send(requests.listTerminalsRequest()),
      "TerminalsListed",
    ).terminals ?? []
    assert(
      terminals.some((terminal) => terminal.terminal_id === pairing.terminal_id && terminal.terminal_type === "cli"),
      "home kernel should list the paired CLI terminal",
      { pairing, terminals },
    )

    await waitForSession(homeClient, requests, localSnapshot.session.id)
    await waitForSession(verificationClient, requests, localSnapshot.session.id)
    await waitForSession(homeClient, requests, remoteSnapshot.session.id)
    await waitForSession(verificationClient, requests, remoteSnapshot.session.id)

    const listed = unwrap(
      await verificationClient.send(requests.listSessionsRequest()),
      "SessionsListed",
    )
    const localSession = listed.sessions?.find((session) => session.id === localSnapshot.session.id)
    const remoteSession = listed.sessions?.find((session) => session.alias === remoteAlias)
    assert(localSession, "hosted relay client should list the session created by the local TUI", {
      localAlias,
      sessions: listed.sessions,
    })
    assert(remoteSession, "home kernel should list the session created by the paired orphan CLI", {
      remoteAlias,
      sessions: listed.sessions,
    })
    assert(remoteSession.id === remoteSnapshot.session.id, "paired CLI snapshot should match home kernel session", {
      snapshotSession: remoteSnapshot.session,
      remoteSession,
    })

    const localAgents = unwrap(
      await verificationClient.send(requests.listAgentsRequest(localSession.id)),
      "AgentsListed",
    ).agents ?? []
    const localFocusedAgent = localAgents.find((agent) => agent.id === localSnapshot.session.focusedAgentId)
    assert(
      localFocusedAgent && localFocusedAgent.provider === remoteCliPairingProvider && localFocusedAgent.provider !== "dev-stub",
      "local TUI should use the configured real provider, not dev-stub",
      { localFocusedAgent, localAgents, expectedProvider: remoteCliPairingProvider },
    )

    const remoteAgents = unwrap(
      await verificationClient.send(requests.listAgentsRequest(remoteSession.id)),
      "AgentsListed",
    ).agents ?? []
    const remoteFocusedAgent = remoteAgents.find((agent) => agent.id === remoteSnapshot.session.focusedAgentId)
    assert(
      remoteFocusedAgent && remoteFocusedAgent.provider === remoteCliPairingProvider && remoteFocusedAgent.provider !== "dev-stub",
      "paired orphan CLI should use the configured real provider, not dev-stub",
      { remoteFocusedAgent, remoteAgents, expectedProvider: remoteCliPairingProvider },
    )

    await localAutomation.send("submit_prompt", {
      prompt: `Reply with exactly ${localMarker} and nothing else.`,
    })
    await Promise.all([
      waitForHistoryText(homeClient, requests, localSession.id, localSnapshot.session.focusedAgentId, localMarker, pollTimeoutMs),
      waitForHistoryText(verificationClient, requests, localSession.id, localSnapshot.session.focusedAgentId, localMarker, pollTimeoutMs),
    ])

    await remoteAutomation(remoteSocket, "submit_prompt", {
      prompt: `Reply with exactly ${remoteMarker} and nothing else.`,
      timeoutMs: pollTimeoutMs,
    })
    await Promise.all([
      waitForHistoryText(homeClient, requests, remoteSession.id, remoteSnapshot.session.focusedAgentId, remoteMarker, pollTimeoutMs),
      waitForHistoryText(verificationClient, requests, remoteSession.id, remoteSnapshot.session.focusedAgentId, remoteMarker, pollTimeoutMs),
    ])

    await localAutomation.send("exit").catch(() => {})
    await remoteAutomation(remoteSocket, "exit").catch(() => {})
    await homeClient.send(requests.endSessionRequest(localSession.id)).catch(() => {})
    await homeClient.send(requests.endSessionRequest(remoteSession.id)).catch(() => {})
    log("remote-cli-pairing-pass", {
      host: remoteCliHost,
      localSessionId: localSession.id,
      remoteSessionId: remoteSession.id,
      localAlias,
      remoteAlias,
      terminalId: pairing.terminal_id,
      provider: remoteCliPairingProvider,
      model: remoteCliPairingModel,
      localMarker,
      remoteMarker,
    })
  } finally {
    localAutomation?.close()
    await terminateChild(localCli)
    await terminateChild(remoteCli)
    await rm(localSocket, { force: true }).catch(() => {})
    await runSsh(`rm -f ${shellQuote(remoteSocket)}; rm -rf ${shellQuote(remoteWorkspace)}`).catch(() => {})
  }
}

async function runHostedRemoteCliAssertions({
  requests,
  homeClient,
  verificationClient,
  relayUrl,
  relayToken,
  targetDaemonAlias,
}) {
  const remoteId = `${process.pid}-${Date.now()}`
  const remoteAlias = `hosted-remote-cli-${remoteId}`
  const remoteClientId = `hosted-remote-client-${remoteId}`
  const remoteWorkspace = `/tmp/arroba-hosted-remote-cli-${remoteId}`
  const remoteSocket = `/tmp/arroba-hosted-remote-cli-${remoteId}.sock`
  const remoteCommand = [
    "set -e",
    "export PATH=/root/.bun/bin:/opt/node-v22/bin:$PATH",
    "export ARROBA_TEST_TUI=1",
    `mkdir -p ${shellQuote(remoteWorkspace)}`,
    `cd ${shellQuote(path.posix.join(remoteCliRepo, "apps/cli"))}`,
    [
      "bun",
      "dist/index.js",
      "--relay-url",
      shellQuote(relayUrl),
      "--relay-token",
      shellQuote(relayToken),
      "--target-daemon-alias",
      shellQuote(targetDaemonAlias),
      "--automation-socket",
      shellQuote(remoteSocket),
      "--create-session",
      "--alias",
      shellQuote(remoteAlias),
      "--workspace",
      shellQuote(remoteWorkspace),
      "--worktree",
      shellQuote(remoteWorkspace),
      "--client-id",
      shellQuote(remoteClientId),
      "--provider",
      "dev-stub",
      "--model",
      "remote-cli-drill",
      "--effort",
      "low",
    ].join(" "),
  ].join("; ")

  let remoteCli = null
  try {
    await allowDevStubProvider(homeClient, requests, "remote-cli-home-kernel")
    log("remote-cli-start", { host: remoteCliHost, repo: remoteCliRepo, alias: remoteAlias })
    remoteCli = spawnProcess("ssh", sshArgs(remoteCommand, { tty: true }), {
      cwd: repoRoot,
      env: process.env,
      name: "remote-cli",
    })
    try {
      await waitForRemoteSocket(remoteSocket)
    } catch (error) {
      throw new Error(`${error instanceof Error ? error.message : String(error)}\nremote alias: ${remoteAlias}`)
    }
    await remoteAutomation(remoteSocket, "ping")
    const snapshot = await remoteAutomation(remoteSocket, "wait_for", { screen: "agents", timeoutMs: 10_000 })
    assert(snapshot.session?.id, "remote CLI should attach to a session", snapshot)
    const listed = unwrap(
      await verificationClient.send(requests.listSessionsRequest()),
      "SessionsListed",
    )
    const remoteSession = listed.sessions?.find((session) => session.alias === remoteAlias)
    assert(remoteSession, "home kernel should list the session created by the remote CLI", {
      remoteAlias,
      sessions: listed.sessions,
    })
    assert(remoteSession.id === snapshot.session.id, "remote CLI snapshot should match home kernel session", {
      snapshotSession: snapshot.session,
      remoteSession,
    })
    await remoteAutomation(remoteSocket, "exit").catch(() => {})
    await verificationClient.send(requests.endSessionRequest(remoteSession.id)).catch(() => {})
    log("remote-cli-pass", {
      host: remoteCliHost,
      sessionId: remoteSession.id,
      alias: remoteAlias,
    })
  } finally {
    await terminateChild(remoteCli)
    await runSsh(`rm -f ${shellQuote(remoteSocket)}; rm -rf ${shellQuote(remoteWorkspace)}`).catch(() => {})
  }
}

async function runHostedSecondKernelAssertions({
  LocalIpcClient,
  requests,
  kernelPath,
  rootDir,
  workspace,
  homeClient,
  ownerProfile,
  ownerClientId,
}) {
  const workerPorts = await makeWorkerPorts()
  const workerDaemonId = `hosted-worker-daemon-${process.pid}-${Date.now()}`
  const workerAlias = `hosted-worker-${process.pid}`
  const workerHome = path.join(rootDir, "worker-home")
  const workerArrobaHome = path.join(workerHome, ".arroba")

  log("second-kernel-cloud-pair-machine", { machineId: workerDaemonId, alias: workerAlias })
  await pairCloudMachineDirect({
    profile: ownerProfile,
    machineId: workerDaemonId,
    alias: workerAlias,
  })
  const workerRelayToken = await issueMachineRelayToken({
    profile: ownerProfile,
    machineId: workerDaemonId,
  })
  await mkdir(workerArrobaHome, { recursive: true })
  const workerEnv = {
    ...process.env,
    HOME: workerHome,
    ARROBA_HOME: workerArrobaHome,
    ARROBA_KERNEL_PORT: String(workerPorts.kernelPort),
    ARROBA_MCP_PORT: String(workerPorts.mcpPort),
    ARROBA_OPENCODE_PORT: String(workerPorts.opencodePort),
    ARROBA_CODEX_PORT: String(workerPorts.codexPort),
    ARROBA_RELAY_URL: ownerProfile.relayUrl,
    ARROBA_RELAY_TOKEN: workerRelayToken,
    ARROBA_DAEMON_ID: workerDaemonId,
    ARROBA_DAEMON_ALIAS: workerAlias,
    ARROBA_MACHINE_ID: workerDaemonId,
    ARROBA_MACHINE_ALIAS: workerAlias,
    ARROBA_ACCEPT_REMOTE_LEASES: "1",
    ARROBA_DAEMON_SOCKET: path.join(rootDir, "worker-daemon.sock"),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, "worker-session-history"),
  }

  let worker = null
  let workerClient = null
  const eventLog = []
  try {
    log("start-second-kernel", { workerAlias })
    worker = spawnProcess(kernelPath, [], { cwd: repoRoot, env: workerEnv, name: "worker-kernel" })
    const workerKernelUrl = `ws://127.0.0.1:${workerPorts.kernelPort}/kernel`
    await waitForLocalDaemon(LocalIpcClient, requests, workerKernelUrl, workspace)
    workerClient = new LocalIpcClient(workerKernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    await allowDevStubProvider(homeClient, requests, "second-kernel-home")
    await allowDevStubProvider(workerClient, requests, "second-kernel-worker")

    log("second-kernel-client-token-request", { workerAlias })
    const workerClientToken = await issueSessionScopedClientToken(apiUrl, {
      sessionToken: ownerProfile.cloudSessionToken,
      accountId: ownerProfile.accountId,
      realmId: ownerProfile.realmId,
      subject: ownerClientId,
      userId: ownerProfile.userId,
      clientId: ownerClientId,
      targetDaemonAlias: workerAlias,
    })
    log("second-kernel-client-token-issued", { workerAlias })
    log("second-kernel-relay-target-probe", { workerAlias })
    await waitForRelayTarget(
      LocalIpcClient,
      requests,
      ownerProfile.relayUrl,
      workerClientToken,
      workerAlias,
    )
    log("second-kernel-relay-target-ready", { workerAlias })

    await waitForRemoteMachine(homeClient, requests, workerDaemonId)
    const created = unwrap(
      await homeClient.send(requests.createSessionRequest(workspace, workspace)),
      "SessionCreated",
    )
    const session = created.session
    const attachment = unwrap(
      await homeClient.send(requests.attachToSessionRequest(session.id, `hosted-second-kernel-${Date.now()}`)),
      "SessionAttached",
    ).attachment
    homeClient.onKernelEvent((event) => {
      eventLog.push({ ...event, observed_at_ms: Date.now() })
    })
    log("second-kernel-subscribe-start", { sessionId: session.id, attachmentId: attachment.id })
    await homeClient.subscribeToKernelEvents(session.id, attachment.id)
    log("second-kernel-subscribe-ready", { sessionId: session.id, attachmentId: attachment.id })

    log("second-kernel-spawn-agent-start", { workerDaemonId })
    const spawned = unwrap(
      await homeClient.send(requests.spawnAgentRequest(
        session.id,
        "dev-stub",
        "hosted-worker-agent",
        "hosted-second-kernel",
        workspace,
        "low",
        undefined,
        undefined,
        workerDaemonId,
      )),
      "AgentSpawned",
    )
    log("second-kernel-spawn-agent-ready", { workerDaemonId, agentId: spawned.agent?.id })
    assert(spawned.agent?.remote_execution?.worker_machine_id === workerDaemonId, "remote dev-stub agent should be leased to the second kernel", spawned)
    await homeClient.send(requests.submitPromptRequest(
      session.id,
      attachment.id,
      spawned.agent.id,
      "Reply with exactly HOSTED_SECOND_KERNEL_OK.",
      [],
    ))
    const completed = unwrap(
      await homeClient.send({ CompletePrompt: { session_id: session.id } }),
      "PromptCompleted",
    )
    await waitForCompletion(eventLog, pollTimeoutMs, 0)
    await homeClient.send(requests.endSessionRequest(session.id)).catch(() => {})
    log("second-kernel-pass", {
      machineId: workerDaemonId,
      workerAlias,
      agentId: spawned.agent.id,
      completedPromptId: completed.completion?.completed?.id ?? null,
    })
  } finally {
    await closeClient(workerClient, "worker")
    await terminateChild(worker)
  }
}

function createHostedCommandDeps({
  workspace,
  clientId,
  localClient,
  requests,
  profileRef,
  notices,
  ownerAccountSlug,
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
    bootstrapCloudRelay: async () => {
      throw new Error("hosted drill uses device login, not bootstrap")
    },
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
      log("kernel-cloud-login-start", { apiUrl: nextApiUrl })
      const login = unwrap(
        await localClient.send(requests.startCloudRelayLoginRequest(nextApiUrl, input)),
        "CloudRelayLoginStarted",
      ).login
      log("approve-cloud-login", {
        verificationUrl: login.verification_url,
        userCode: login.user_code,
        expiresAt: login.expires_at,
      })
      await approveDevDeviceLogin({
        role: "owner",
        userCode: login.user_code,
        accountSlug: ownerAccountSlug,
      })
      return {
        apiUrl: login.api_url,
        deviceCode: login.device_code,
        userCode: login.user_code,
        verificationUrl: login.verification_url,
        expiresAtMs: Math.min(Date.parse(login.expires_at), Date.now() + pollTimeoutMs),
        intervalSeconds: login.interval_seconds,
      }
    },
    pollCloudDeviceLogin: async (nextApiUrl, deviceCode) => {
      const result = unwrap(
        await localClient.send(requests.pollCloudRelayLoginRequest(nextApiUrl, deviceCode)),
        "CloudRelayLoginPolled",
      ).result
      log("kernel-cloud-login-poll-result", { status: result.status })
      if (result.status === "authorization_pending") {
        return {
          status: "authorization_pending",
          intervalSeconds: result.interval_seconds ?? 2,
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
    openExternalUrl: async () => false,
  }
}

async function main() {
  const ports = await makePorts()
  const runId = `hosted-cloud-relay-${process.pid}-${Date.now()}`
  const rootDir = path.join(os.tmpdir(), runId)
  const workspace = path.join(rootDir, "workspace")
  const homeDir = path.join(rootDir, "home")
  const arrobaHome = path.join(homeDir, ".arroba")
  const xdgConfigHome = path.join(homeDir, ".config")
  const xdgStateHome = path.join(homeDir, ".local", "state")
  const xdgRuntimeDir = path.join(homeDir, "run")
  const daemonId = `hosted-daemon-${process.pid}-${Date.now()}`
  const daemonAlias = `hosted-home-${process.pid}`
  const clientId = `hosted-cli-${process.pid}-${Date.now()}`
  const ownerAccountSlug = `hosted-owner-${process.pid}-${Date.now()}`

  await rm(rootDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(workspace, { recursive: true })
  await mkdir(arrobaHome, { recursive: true })
  await mkdir(xdgConfigHome, { recursive: true })
  await mkdir(xdgStateHome, { recursive: true })
  await mkdir(xdgRuntimeDir, { recursive: true })

  let daemon = null
  let localClient = null
  let remoteClient = null
  let passed = false

  try {
    log("build-cli")
    const cliBuild = await run("pnpm", ["run", "build"], { cwd: cliRoot, env: process.env })
    if (cliBuild.code !== 0) {
      throw new Error(`arroba cli build failed\n${cliBuild.stdout}\n${cliBuild.stderr}`)
    }

    log("build-kernel")
    const kernelPath = await buildKernelIfNeeded()

    const [{ LocalIpcClient }, requests, commandActions] = await Promise.all([
      import("../../../packages/kernel-client/dist/ipc.js"),
      import("../../../packages/kernel-client/dist/ipc-requests.js"),
      import("../dist/command-actions.js"),
    ])

    const daemonEnv = {
      ...process.env,
      HOME: os.homedir(),
      XDG_CONFIG_HOME: xdgConfigHome,
      XDG_STATE_HOME: xdgStateHome,
      XDG_RUNTIME_DIR: xdgRuntimeDir,
      ARROBA_HOME: arrobaHome,
      ARROBA_KERNEL_PORT: String(ports.kernelPort),
      ARROBA_MCP_PORT: String(ports.mcpPort),
      ARROBA_OPENCODE_PORT: String(ports.opencodePort),
      ARROBA_CODEX_PORT: String(ports.codexPort),
      ARROBA_DAEMON_ID: daemonId,
      ARROBA_DAEMON_ALIAS: daemonAlias,
      ARROBA_MACHINE_ID: daemonId,
      ARROBA_MACHINE_ALIAS: daemonAlias,
      ARROBA_DAEMON_SOCKET: path.join(rootDir, "daemon.sock"),
      ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, "session-history"),
    }

    log("start-kernel")
    daemon = spawnProcess(kernelPath, [], { cwd: repoRoot, env: daemonEnv, name: "kernel" })

    const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}/kernel`
    await waitForLocalDaemon(LocalIpcClient, requests, kernelUrl, workspace)
    localClient = new LocalIpcClient(kernelUrl)

    const profileRef = { current: null }
    const notices = []
    const handlers = commandActions.createCommandActionHandlers(createHostedCommandDeps({
      workspace,
      clientId,
      localClient,
      requests,
      profileRef,
      notices,
      ownerAccountSlug,
    }))

    log("command-cloud-login", { apiUrl })
    await handleRelayCommandWithRetry(handlers, {
      kind: "relay",
      raw: "/relay cloud login",
      args: ["cloud", "login"],
    }, "cloud-login")
    assert(profileRef.current?.cloudSessionToken, "hosted cloud login should save an authenticated profile", profileRef.current)

    log("command-cloud-pair")
    await handleRelayCommandWithRetry(handlers, {
      kind: "relay",
      raw: "/relay cloud pair hosted-drill-cli",
      args: ["cloud", "pair", "hosted-drill-cli"],
    }, "cloud-pair")
    assert(profileRef.current?.clientId === clientId, "hosted cloud pair should save client id", profileRef.current)

    log("command-cloud-pair-machine")
    await handleRelayCommandWithRetry(handlers, {
      kind: "relay",
      raw: `/relay cloud pair-machine ${daemonId} hosted-drill-machine`,
      args: ["cloud", "pair-machine", daemonId, "hosted-drill-machine"],
    }, "cloud-pair-machine")
    assert(profileRef.current?.machineId === daemonId, "hosted cloud pair-machine should save machine id", profileRef.current)

    log("command-cloud-connect")
    await handleRelayCommandWithRetry(handlers, {
      kind: "relay",
      raw: "/relay cloud connect",
      args: ["cloud", "connect"],
    }, "cloud-connect")

    log("command-cloud-client-token")
    await handleRelayCommandWithRetry(handlers, {
      kind: "relay",
      raw: `/relay cloud client-token ${daemonAlias}`,
      args: ["cloud", "client-token", daemonAlias],
    }, "cloud-client-token")
    const clientRelay = parseCloudClientTokenNotice(notices)

    log("relay-target-probe", { relayUrl: clientRelay.relayUrl, daemonAlias })
    await waitForRelayTarget(
      LocalIpcClient,
      requests,
      clientRelay.relayUrl,
      clientRelay.relayToken,
      daemonAlias,
    )

    remoteClient = installSendRetry(new LocalIpcClient(clientRelay.relayUrl, {
      relayAuthToken: clientRelay.relayToken,
      targetDaemonAlias: daemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    }), "owner-relay")

    log("remote-session-create")
    const created = unwrap(
      await sendWithRetry(remoteClient, requests.createSessionRequest(workspace, workspace), "remote-session-create"),
      "SessionCreated",
    )
    assert(created.session?.id, "remote cloud session creation should return a session", created)

    const listed = unwrap(
      await sendWithRetry(remoteClient, requests.listSessionsRequest(), "remote-session-list"),
      "SessionsListed",
    )
    assert(
      Array.isArray(listed.sessions) && listed.sessions.some((session) => session.id === created.session.id),
      "remote cloud client should list the created session",
      listed,
    )

    if (runRemoteCli) {
      await runHostedRemoteCliAssertions({
        requests,
        homeClient: localClient,
        verificationClient: remoteClient,
        relayUrl: clientRelay.relayUrl,
        relayToken: clientRelay.relayToken,
        targetDaemonAlias: daemonAlias,
      })
    }

    if (runRemoteCliPairing) {
      await runHostedRemoteCliPairingAssertions({
        requests,
        homeClient: localClient,
        verificationClient: remoteClient,
        workspace,
        kernelUrl,
      })
    }

    if (runMultiUser) {
      await runHostedMultiUserAssertions({
        LocalIpcClient,
        requests,
        localClient,
        ownerRemoteClient: remoteClient,
        ownerProfile: profileRef.current,
        ownerClientId: clientId,
        workspace,
        daemonAlias,
        session: created.session,
      })
    } else {
      log("multi-user-skipped", {
        reason: devAuthSecret
          ? "set ARROBA_CLOUD_HOSTED_MULTI_USER=1"
          : "set ARROBA_CLOUD_HOSTED_MULTI_USER=1 and approve owner, peer, and third browser logins, or set ARROBA_CLOUD_DEV_AUTH_SECRET",
      })
    }

    if (runSecondKernel) {
      await runHostedSecondKernelAssertions({
        LocalIpcClient,
        requests,
        kernelPath,
        rootDir,
        workspace,
        homeClient: localClient,
        ownerProfile: profileRef.current,
        ownerClientId: clientId,
      })
    }

    log("pass", {
      apiUrl,
      relayUrl: clientRelay.relayUrl,
      accountSlug: profileRef.current.accountSlug,
      sessionId: created.session.id,
      multiUser: runMultiUser,
      remoteCli: runRemoteCli,
      remoteCliPairing: runRemoteCliPairing,
      secondKernel: runSecondKernel,
    })
    passed = true
  } finally {
    await closeClient(remoteClient, "remote")
    await closeClient(localClient, "local")
    await terminateChild(daemon)
    if (passed) {
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    } else {
      log("preserved-failed-run", { rootDir })
    }
  }
}

main().then(() => {
  process.exit(0)
}).catch((error) => {
  console.error(error)
  process.exitCode = 1
})
