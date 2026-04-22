#!/usr/bin/env node
import { spawn } from "node:child_process"
import { mkdir, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const apiUrl = (process.env.ARROBA_CLOUD_HOSTED_API_URL ?? "https://arroba-cloud-staging.osc-fr1.scalingo.io").replace(/\/$/, "")
const pollTimeoutMs = Number(process.env.ARROBA_CLOUD_HOSTED_POLL_TIMEOUT_MS ?? 10 * 60 * 1000)
const runMultiUser = process.env.ARROBA_CLOUD_HOSTED_MULTI_USER === "1"
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

async function terminateChild(child, signal = "SIGTERM") {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(5_000)])
  if (child.exitCode == null) {
    child.kill("SIGKILL")
    await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(2_000)])
  }
}

async function buildKernelIfNeeded() {
  const binary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
  const result = await run("cargo", ["build", "--manifest-path", path.join(repoRoot, "apps/kernel/Cargo.toml"), "--bin", "arroba-kernel"])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return binary
}

function makePorts() {
  const base = 56000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort: base,
    mcpPort: base + 1000,
    opencodePort: base + 2000,
    codexPort: base + 2001,
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
  const tokenMatch = fields.command?.match(/\s--relay-token\s+(\S+)/)
  assert(tokenMatch?.[1], "client token command should include --relay-token", fields.command)
  return {
    relayUrl: fields.relay_url,
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
  ownerProfile,
  ownerClientId,
  workspace,
  daemonAlias,
  session,
}) {
  log("multi-user-cloud-invites")
  const localInvite = unwrap(
    await localClient.send(requests.createSessionInviteRequest(session.id, null, 2)),
    "SessionInviteCreated",
  )
  const cloudInvite = unwrap(
    await localClient.send(requests.createCloudSessionInviteRequest(session.id, {
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
  const ownerScopedClient = new LocalIpcClient(ownerProfile.relayUrl, {
    relayAuthToken: ownerScopedToken,
    targetDaemonAlias: daemonAlias,
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })

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

    peerRemoteClient = new LocalIpcClient(ownerProfile.relayUrl, {
      relayAuthToken: peerRelayToken,
      targetDaemonAlias: daemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    thirdRemoteClient = new LocalIpcClient(ownerProfile.relayUrl, {
      relayAuthToken: thirdRelayToken,
      targetDaemonAlias: daemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })

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
  const ports = makePorts()
  const runId = `hosted-cloud-relay-${process.pid}-${Date.now()}`
  const rootDir = path.join(os.tmpdir(), runId)
  const workspace = path.join(rootDir, "workspace")
  const daemonId = `hosted-daemon-${process.pid}-${Date.now()}`
  const daemonAlias = `hosted-home-${process.pid}`
  const clientId = `hosted-cli-${process.pid}-${Date.now()}`
  const ownerAccountSlug = `hosted-owner-${process.pid}-${Date.now()}`

  await rm(rootDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(workspace, { recursive: true })

  let daemon = null
  let localClient = null
  let remoteClient = null

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
      ARROBA_KERNEL_PORT: String(ports.kernelPort),
      ARROBA_MCP_PORT: String(ports.mcpPort),
      ARROBA_OPENCODE_PORT: String(ports.opencodePort),
      ARROBA_CODEX_PORT: String(ports.codexPort),
      ARROBA_DAEMON_ID: daemonId,
      ARROBA_DAEMON_ALIAS: daemonAlias,
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
    await handlers.handleRelayCommand({
      kind: "relay",
      raw: "/relay cloud login",
      args: ["cloud", "login"],
    })
    assert(profileRef.current?.cloudSessionToken, "hosted cloud login should save an authenticated profile", profileRef.current)

    log("command-cloud-pair")
    await handlers.handleRelayCommand({
      kind: "relay",
      raw: "/relay cloud pair hosted-drill-cli",
      args: ["cloud", "pair", "hosted-drill-cli"],
    })
    assert(profileRef.current?.clientId === clientId, "hosted cloud pair should save client id", profileRef.current)

    log("command-cloud-pair-machine")
    await handlers.handleRelayCommand({
      kind: "relay",
      raw: `/relay cloud pair-machine ${daemonId} hosted-drill-machine`,
      args: ["cloud", "pair-machine", daemonId, "hosted-drill-machine"],
    })
    assert(profileRef.current?.machineId === daemonId, "hosted cloud pair-machine should save machine id", profileRef.current)

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

    log("relay-target-probe", { relayUrl: clientRelay.relayUrl, daemonAlias })
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

    const listed = unwrap(
      await remoteClient.send(requests.listSessionsRequest()),
      "SessionsListed",
    )
    assert(
      Array.isArray(listed.sessions) && listed.sessions.some((session) => session.id === created.session.id),
      "remote cloud client should list the created session",
      listed,
    )

    if (runMultiUser) {
      await runHostedMultiUserAssertions({
        LocalIpcClient,
        requests,
        localClient,
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

    log("pass", {
      apiUrl,
      relayUrl: clientRelay.relayUrl,
      accountSlug: profileRef.current.accountSlug,
      sessionId: created.session.id,
      multiUser: runMultiUser,
    })
  } finally {
    await remoteClient?.close().catch(() => {})
    await localClient?.close().catch(() => {})
    await terminateChild(daemon)
    await rm(rootDir, { recursive: true, force: true }).catch(() => {})
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
