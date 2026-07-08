#!/usr/bin/env node
import { mkdir } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"
import {
  addWorkflowEdgeRequest,
  addWorkflowNodeRequest,
  assert,
  buildKernelIfNeeded,
  createMinimalCommandDeps,
  createWorkflowEndpointRequest,
  expectReject,
  issueSessionScopedClientToken,
  log,
  loginCloudDrillUser,
  makePorts,
  parseCloudClientTokenNotice,
  postJson,
  removePersistedCloudSessionToken,
  removeWorkflowEdgeRequest,
  run,
  spawnProcess,
  terminateChild,
  unwrap,
  updateWorkflowNodeInstructionsRequest,
  waitForCloudRelayTarget,
  waitForHttp,
  waitForLocalDaemon,
  waitForRelayTarget,
} from "./lib/live-cloud-relay-drill-helpers.mjs"

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
const DEV_AUTH_SECRET = "arroba-cloud-live-drill-dev-auth-secret"
const machineCredentialOnly = process.env.ARROBA_CLOUD_MACHINE_CREDENTIAL_ONLY === "1"

async function main() {
  const ports = makePorts()
  const runId = `cloud-relay-${process.pid}-${Date.now()}`
  const rootDir = path.join(repoRoot, ".artifacts", "live-cloud-relay-drill", runId)
  const workspace = path.join(rootDir, "workspace")
  const home = path.join(rootDir, "home")
  const configHome = path.join(rootDir, "xdg-config")
  const daemonId = `cloud-daemon-${process.pid}-${Date.now()}`
  const daemonAlias = `cloud-home-${process.pid}`
  const clientId = `cloud-cli-${process.pid}-${Date.now()}`
  const apiUrl = `http://127.0.0.1:${ports.cloudPort}`
  await prepareDrillArtifacts(rootDir)
  await mkdir(workspace, { recursive: true })

  const relayEnv = {
    ...process.env,
    ARROBA_RELAY_HOST: "127.0.0.1",
    ARROBA_RELAY_PORT: String(ports.relayPort),
    ARROBA_RELAY_SCOPED_ISSUER: CLOUD_ISSUER,
    ARROBA_RELAY_SCOPED_HMAC_SECRET: CLOUD_SECRET,
  }
  const daemonEnv = {
    ...process.env,
    HOME: home,
    XDG_CONFIG_HOME: configHome,
    XDG_STATE_HOME: path.join(rootDir, "xdg-state"),
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
    ARROBA_CLOUD_TEST_AUTH0_IDENTITY_HEADER: "1",
    ARROBA_CLOUD_DEV_AUTH_SECRET: DEV_AUTH_SECRET,
  }

  let relay = null
  let daemon = null
  let cloudServer = null
  let localClient = null
  let remoteClient = null
  const profileRef = { current: null }
  const notices = []
  let db = null
  let succeeded = false
  let failure = null

  try {
    const [{ LocalIpcClient }, requests, cloudRelay, commandActions, cloudDb] = await Promise.all([
      import("../../../packages/kernel-client/dist/ipc.js"),
      import("../../../packages/kernel-client/dist/ipc-requests.js"),
      import("../dist/cloud-relay.js"),
      import("../dist/command-actions.js"),
      import(path.join(cloudRoot, "packages/db/dist/index.js")),
    ])
    const kernelPath = await buildKernelIfNeeded()
    db = cloudDb.createCloudDatabase({ databaseUrl: DATABASE_URL })

    log("build-cli")
    const cliBuild = await run("pnpm", ["run", "build"], { cwd: cliRoot, env: process.env })
    if (cliBuild.code !== 0) {
      throw new Error(`arroba cli build failed\n${cliBuild.stdout}\n${cliBuild.stderr}`)
    }

    log("build-cloud-db")
    const cloudDbBuild = await run("pnpm", ["--filter", "@arroba-cloud/db", "run", "build"], { cwd: cloudRoot, env: cloudEnv })
    if (cloudDbBuild.code !== 0) {
      throw new Error(`arroba-cloud db build failed\n${cloudDbBuild.stdout}\n${cloudDbBuild.stderr}`)
    }
    log("build-cloud-api")
    const cloudApiBuild = await run("pnpm", ["--filter", "@arroba-cloud/api", "run", "build"], { cwd: cloudRoot, env: cloudEnv })
    if (cloudApiBuild.code !== 0) {
      throw new Error(`arroba-cloud api build failed\n${cloudApiBuild.stdout}\n${cloudApiBuild.stderr}`)
    }
    const migrate = await run("pnpm", ["--filter", "@arroba-cloud/db", "run", "prisma:migrate"], {
      cwd: cloudRoot,
      env: cloudEnv,
    })
    if (migrate.code !== 0) {
      throw new Error(`arroba-cloud migrate failed\n${migrate.stdout}\n${migrate.stderr}`)
    }

    log("start-cloud")
    cloudServer = spawnProcess("node", [path.join(cloudRoot, "apps/api/dist/node-server.js")], {
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

    let handlers = commandActions.createCommandActionHandlers(createMinimalCommandDeps({
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
    assert(profileRef.current?.machineCredential, "cloud login should save the machine credential", profileRef.current)

    log("command-cloud-pair")
    await handlers.handleRelayCommand({
      kind: "relay",
      raw: "/relay cloud pair drill-cli",
      args: ["cloud", "pair", "drill-cli"],
    })
    assert(profileRef.current?.clientId === clientId, "cloud pair command should save client id", profileRef.current)

    log("command-cloud-pair-machine")
    const linkedMachineId = profileRef.current?.machineId
    assert(linkedMachineId, "cloud login should link the local machine id before pair-machine", profileRef.current)
    await handlers.handleRelayCommand({
      kind: "relay",
      raw: `/relay cloud pair-machine ${linkedMachineId} drill-machine`,
      args: ["cloud", "pair-machine", linkedMachineId, "drill-machine"],
    })
    assert(profileRef.current?.machineId === linkedMachineId, "cloud pair-machine command should preserve machine id", profileRef.current)
    assert(profileRef.current?.machineCredential, "cloud pair-machine should preserve the machine credential", profileRef.current)

    log("command-cloud-connect")
    await handlers.handleRelayCommand({
      kind: "relay",
      raw: "/relay cloud connect",
      args: ["cloud", "connect"],
    })
    const onlineTarget = await waitForCloudRelayTarget(apiUrl, {
      accountId: profileRef.current.accountId,
      realmId: profileRef.current.realmId,
      daemonId,
      status: "ONLINE",
    })
    assert(onlineTarget.machineId === linkedMachineId, "cloud presence should associate the target with the linked machine", onlineTarget)

    if (machineCredentialOnly) {
      log("cloud-machine-credential-restart")
      await localClient.close().catch(() => {})
      localClient = null
      await terminateChild(daemon, "SIGINT")
      daemon = null
      log("cloud-target-offline")
      const offlineTarget = await waitForCloudRelayTarget(apiUrl, {
        accountId: profileRef.current.accountId,
        realmId: profileRef.current.realmId,
        daemonId,
        status: "OFFLINE",
      })
      assert(offlineTarget.machineId === linkedMachineId, "cloud target status should expose the disconnected linked kernel", offlineTarget)
      const strippedConfigPath = await removePersistedCloudSessionToken(configHome)
      log("cloud-session-token-removed", { configPath: strippedConfigPath })
      daemon = spawnProcess(kernelPath, [], { cwd: repoRoot, env: daemonEnv, name: "kernel" })
      await waitForLocalDaemon(LocalIpcClient, requests, kernelUrl, workspace)
      localClient = new LocalIpcClient(kernelUrl)
      log("cloud-target-reonline")
      const reonlineTarget = await waitForCloudRelayTarget(apiUrl, {
        accountId: profileRef.current.accountId,
        realmId: profileRef.current.realmId,
        daemonId,
        status: "ONLINE",
      })
      assert(reonlineTarget.machineId === linkedMachineId, "cloud target status should expose the reconnected linked kernel", reonlineTarget)
      const restartedRelayStatus = unwrap(
        await localClient.send(requests.relayStatusRequest()),
        "RelayStatus",
      ).status
      assert(restartedRelayStatus.connected, "restarted kernel should reconnect to cloud relay using machine credential", restartedRelayStatus)
      handlers = commandActions.createCommandActionHandlers(createMinimalCommandDeps({
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
    }

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

    if (machineCredentialOnly) {
      console.log("live cloud machine credential drill passed")
      return
    }

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

    succeeded = true
    console.log("live cloud relay drill passed")
  } catch (error) {
    failure = error
    throw error
  } finally {
    const accountId = profileRef?.current?.accountId
    const realmId = profileRef?.current?.realmId
    await remoteClient?.close().catch(() => {})
    await localClient?.close().catch(() => {})
    await terminateChild(daemon, "SIGINT")
    if (accountId && realmId) {
      await waitForCloudRelayTarget(apiUrl, {
        accountId,
        realmId,
        daemonId,
        status: "OFFLINE",
      }, 10_000).catch((error) => log("cloud-offline-presence-timeout", { message: error.message }))
    }
    await terminateChild(relay)
    await terminateChild(cloudServer)
    await db?.account.deleteMany({ where: { slug: { in: [runId, `${runId}-peer`, `${runId}-third`] } } }).catch(() => {})
    await db?.user.deleteMany({ where: { email: { in: [`${runId}@example.com`, `${runId}-peer@example.com`, `${runId}-third@example.com`] } } }).catch(() => {})
    await db?.$disconnect().catch(() => {})
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: true,
      failure,
      metadata: {
        drill: "live-cloud-relay",
        runId,
        apiUrl,
        relayUrl: `ws://127.0.0.1:${ports.relayPort}`,
        daemonId,
        daemonAlias,
        clientId,
        cloudRoot,
        databaseUrl: DATABASE_URL.replace(/:\/\/([^:]+):([^@]+)@/, "://$1:***@"),
        machineCredentialOnly,
        profile: profileRef.current ? {
          accountId: profileRef.current.accountId,
          userId: profileRef.current.userId,
          machineId: profileRef.current.machineId,
          clientId: profileRef.current.clientId,
          relayUrl: profileRef.current.relayUrl,
        } : null,
        noticeCount: notices.length,
      },
    })
  }
}

await main()
