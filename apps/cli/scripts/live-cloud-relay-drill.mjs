#!/usr/bin/env node
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import { homedir, tmpdir } from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { createDrillInterruption } from "./lib/drill-interruption.mjs"
import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"
import {
  addWorkflowEdgeRequest,
  addWorkflowNodeRequest,
  assert,
  buildKernelIfNeeded,
  buildRustBinary,
  connectSessionScopedCloudClient,
  createCloudDrillClient,
  loadCloudRelayDrillModules,
  createMinimalCommandDeps,
  createWorkflowEndpointRequest,
  expectReject,
  log,
  loginCloudDrillUser,
  makePorts,
  removeWorkflowEdgeRequest,
  run,
  spawnProcess,
  terminateChild,
  unwrap,
  updateWorkflowNodeInstructionsRequest,
  waitForCloudRelayTarget,
  waitForHttp,
  waitForLocalDaemon,
} from "./lib/live-cloud-relay-drill-helpers.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const cloudRoot = process.env.CHARIOX_CLOUD_REPO
  ? path.resolve(process.env.CHARIOX_CLOUD_REPO)
  : path.resolve(repoRoot, "..", "chariox-cloud")
const DATABASE_URL =
  process.env.DATABASE_URL ?? "postgresql://chariox:chariox@localhost:5432/chariox_cloud"
const CLOUD_SECRET = "chariox-cloud-live-drill-secret"
const CLOUD_ISSUER = "chariox-cloud-live-drill"
const DEV_AUTH_SECRET = "chariox-cloud-live-drill-dev-auth-secret"
const kernelCredentialOnly = process.env.CHARIOX_CLOUD_KERNEL_CREDENTIAL_ONLY === "1"

async function main() {
  const ports = makePorts()
  const runId = `cloud-relay-${process.pid}-${Date.now()}`
  const rootDir = path.join(process.env.CHARIOX_DRILL_EVIDENCE_DIR ?? path.join(homedir(), ".codex", "evidence", "live-cloud-relay"), runId)
  const stateRoot = await mkdtemp(path.join(tmpdir(), "chariox-cloud-relay-"))
  const workspace = path.join(stateRoot, "workspace")
  const home = path.join(stateRoot, "home")
  const configHome = path.join(stateRoot, "xdg-config")
  const daemonId = `cloud-daemon-${process.pid}-${Date.now()}`
  const daemonAlias = `cloud-home-${process.pid}`

  const apiUrl = `http://127.0.0.1:${ports.cloudPort}`

  const relayEnv = {
    ...process.env,
    CHARIOX_RELAY_HOST: "127.0.0.1",
    CHARIOX_RELAY_PORT: String(ports.relayPort),
    CHARIOX_RELAY_SCOPED_ISSUER: CLOUD_ISSUER,
    CHARIOX_RELAY_SCOPED_HMAC_SECRET: CLOUD_SECRET,
  }
  const daemonEnv = {
    ...process.env,
    HOME: home,
    XDG_CONFIG_HOME: configHome,
    XDG_STATE_HOME: path.join(stateRoot, "xdg-state"),
    CHARIOX_HOME: path.join(stateRoot, "kernel"),
    CHARIOX_PROVIDER_DEV_STUB: "1",
    CHARIOX_KERNEL_PORT: String(ports.kernelPort),
    CHARIOX_MCP_PORT: String(ports.mcpPort),
    CHARIOX_OPENCODE_PORT: String(ports.opencodePort),
    CHARIOX_CODEX_PORT: String(ports.codexPort),
    CHARIOX_DAEMON_ID: daemonId,
    CHARIOX_DAEMON_ALIAS: daemonAlias,
    CHARIOX_DAEMON_SOCKET: path.join(stateRoot, "daemon.sock"),
    CHARIOX_SESSION_HISTORY_DIR: path.join(stateRoot, "session-history"),
  }
  const cloudEnv = {
    ...process.env,
    HOST: "127.0.0.1",
    PORT: String(ports.cloudPort),
    DATABASE_URL,
    CHARIOX_CLOUD_RELAY_URL: `ws://127.0.0.1:${ports.relayPort}`,
    CHARIOX_CLOUD_ISSUER_ID: CLOUD_ISSUER,
    CHARIOX_CLOUD_RELAY_TOKEN_SECRET: CLOUD_SECRET,
    CHARIOX_CLOUD_TEST_AUTH0_IDENTITY_HEADER: "1",
    CHARIOX_CLOUD_DEV_AUTH_SECRET: DEV_AUTH_SECRET,
  }

  let relay = null
  let daemon = null
  let cloudServer = null
  let localClient = null
  let remoteClient = null
  const profileRef = { current: null }
  const terminals = []
  const scopedClients = []
  let terminal = null
  let clientId = null
  const notices = []
  let db = null
  let succeeded = false
  let failure = null
  const sourceCommits = { oss: null, cloud: null }
  const lifecycle = createDrillInterruption()
  const stage = (name, details) => { lifecycle.check(); log(name, details) }

  await lifecycle.run(async () => {
    await prepareDrillArtifacts(rootDir)
    await mkdir(workspace, { recursive: true })
    for (const [name, cwd] of [["oss", repoRoot], ["cloud", cloudRoot]]) {
      const source = await run("git", ["rev-parse", "HEAD"], { cwd })
      assert(source.code === 0, "drill source identity must be available")
      sourceCommits[name] = source.stdout.trim()
    }
    const modules = await loadCloudRelayDrillModules()
    const { LocalIpcClient, requests } = modules
    const kernelPath = await buildKernelIfNeeded()
    const relayPath = await buildRustBinary(path.join(repoRoot, "apps/relay/Cargo.toml"), "chariox-relay")

    stage("build-cloud-db")
    const cloudDbBuild = await run("pnpm", ["--filter", "@chariox-cloud/db", "run", "build"], { cwd: cloudRoot, env: cloudEnv })
    if (cloudDbBuild.code !== 0) {
      throw new Error(`chariox-cloud db build failed\n${cloudDbBuild.stdout}\n${cloudDbBuild.stderr}`)
    }
    stage("build-cloud-api")
    const cloudApiBuild = await run("pnpm", ["--filter", "@chariox-cloud/api", "run", "build"], { cwd: cloudRoot, env: cloudEnv })
    if (cloudApiBuild.code !== 0) {
      throw new Error(`chariox-cloud api build failed\n${cloudApiBuild.stdout}\n${cloudApiBuild.stderr}`)
    }
    const migrate = await run("pnpm", ["--filter", "@chariox-cloud/db", "run", "prisma:migrate"], {
      cwd: cloudRoot,
      env: cloudEnv,
    })
    if (migrate.code !== 0) {
      throw new Error(`chariox-cloud migrate failed\n${migrate.stdout}\n${migrate.stderr}`)
    }

    const cloudDb = await import(path.join(cloudRoot, "packages/db/dist/index.js"))
    db = cloudDb.createCloudDatabase({ databaseUrl: DATABASE_URL })

    stage("start-cloud")
    cloudServer = spawnProcess("node", [path.join(cloudRoot, "apps/api/dist/node-server.js")], {
      cwd: cloudRoot,
      env: cloudEnv,
      name: "cloud-api",
    })
    await waitForHttp(`${apiUrl}/health`)

    stage("start-relay-and-kernel")
    relay = spawnProcess(relayPath, [], {
      cwd: repoRoot,
      env: relayEnv,
      name: "relay",
    })
    daemon = spawnProcess(kernelPath, [], { cwd: repoRoot, env: daemonEnv, name: "kernel" })

    const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}/kernel`
    await waitForLocalDaemon(LocalIpcClient, requests, kernelUrl, workspace, daemonEnv)
    localClient = lifecycle.guardClient(new LocalIpcClient(kernelUrl, { localAuthEnvironment: daemonEnv }))

    terminal = createCloudDrillClient(modules, stateRoot, "owner-terminal")
    terminals.push(terminal)
    clientId = `cli:${terminal.identity.publicKeyThumbprint}`
    const handlers = modules.createCommandActionHandlers(createMinimalCommandDeps({
      apiUrl, runId, workspace, localClient, modules, terminal, profileRef, notices,
    }))

    stage("command-cloud-link")
    await handlers.handleCloudCommand({ kind: "cloud", raw: "/cloud link", args: ["link"] })
    const kernelProfile = await modules.relayApi.getKernelCloudRelayProfile(localClient)
    assert(kernelProfile?.accountSlug === runId && kernelProfile.kernelEnrolled, "cloud link should enroll this kernel")
    assert(kernelProfile.kernelId === daemonId, "enrollment should identify the current kernel")
    assert(!kernelProfile.cloudSessionToken && !kernelProfile.machineCredential && !kernelProfile.kernelCredential, "kernel public profile must not export authority")
    assert(kernelProfile.machineId, "enrollment should record the local machine")
    const linkedMachineId = kernelProfile.machineId

    stage("command-cloud-login")
    await handlers.handleCloudCommand({ kind: "cloud", raw: "/cloud login", args: ["login"] })
    const humanProfile = await terminal.client.profile()
    assert(humanProfile?.accountId === kernelProfile.accountId, "terminal sign-in should use the enrolled account")
    assert(humanProfile?.clientId === clientId && !humanProfile.machineId && !humanProfile.kernelId, "terminal login should use separate CLIENT authority")

    stage("command-cloud-connect")
    await handlers.handleRelayCommand({ kind: "relay", raw: "/relay cloud connect", args: ["cloud", "connect"] })
    const onlineTarget = await waitForCloudRelayTarget(terminal.client, { daemonId, status: "ONLINE" })
    assert(onlineTarget.machineId === linkedMachineId, "cloud presence should associate the target with the enrolled machine")

    // No hand edits to persisted credentials: ordinary enrollment already has
    // only kernel authority. Restart proves its normal product recovery path.
    stage("cloud-kernel-credential-restart")
    await localClient.close()
    localClient = null
    await terminateChild(daemon, "SIGINT")
    daemon = null
    await waitForCloudRelayTarget(terminal.client, { daemonId, status: "OFFLINE" })
    daemon = spawnProcess(kernelPath, [], { cwd: repoRoot, env: daemonEnv, name: "kernel" })
    await waitForLocalDaemon(LocalIpcClient, requests, kernelUrl, workspace, daemonEnv)
    localClient = lifecycle.guardClient(new LocalIpcClient(kernelUrl, { localAuthEnvironment: daemonEnv }))
    const reonlineTarget = await waitForCloudRelayTarget(terminal.client, { daemonId, status: "ONLINE" })
    assert(reonlineTarget.machineId === linkedMachineId, "restarted kernel should preserve enrollment")
    assert((await modules.relayApi.getRelayStatus(localClient)).connected, "restarted kernel should reconnect using its own enrollment")

    stage("cloud-private-client-connect")
    remoteClient = lifecycle.guardClient(await terminal.client.connect(daemonId))

    stage("remote-session-create")
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

    if (kernelCredentialOnly) {
      lifecycle.check()
      succeeded = true
      return
    }

    stage("cloud-shared-session-invite")
    const localInvite = unwrap(
      await remoteClient.send(requests.createSessionInviteRequest(created.session.id, null, 3)),
      "SessionInviteCreated",
    )
    const cloudInvite = await terminal.client.collaboration.createSessionInvite(created.session.id, {
      displayName: "Cloud relay shared session drill", maxUses: 2,
    })
    const localInviteToken = localInvite.invite?.invite_token
    const cloudInviteToken = cloudInvite.invite?.invite_token
    assert(localInviteToken, "local session invite token should be returned", localInvite)
    assert(cloudInviteToken, "cloud session invite token should be returned", cloudInvite)

    const scope = { accountId: kernelProfile.accountId, realmId: kernelProfile.realmId, sessionId: created.session.id, targetDaemonId: daemonId }
    stage("cloud-owner-session-scoped-client")
    const ownerScopedClient = await connectSessionScopedCloudClient(modules, terminal, scope)
    lifecycle.guardClient(ownerScopedClient)
    scopedClients.push(ownerScopedClient)

    stage("cloud-peer-login")
    let peerLogin = await loginCloudDrillUser(apiUrl, {
      modules, stateRoot, name: "peer-terminal", email: `${runId}-peer@example.com`, accountSlug: `${runId}-peer`,
    })
    terminals.push(peerLogin)
    const peerProfile = peerLogin.profile
    const peerClientId = peerProfile.clientId

    stage("cloud-third-login")
    let thirdLogin = await loginCloudDrillUser(apiUrl, {
      modules, stateRoot, name: "third-terminal", email: `${runId}-third@example.com`, accountSlug: `${runId}-third`,
    })
    terminals.push(thirdLogin)
    const thirdProfile = thirdLogin.profile
    const thirdClientId = thirdProfile.clientId

    stage("cloud-peer-accept-invite")
    const peerAcceptance = await peerLogin.client.collaboration.acceptSessionInvite(cloudInviteToken)
    assert(peerAcceptance.acceptance.user_id === peerProfile.userId, "peer should accept the cloud invite as itself")
    stage("cloud-third-accept-invite")
    const thirdAcceptance = await thirdLogin.client.collaboration.acceptSessionInvite(cloudInviteToken)
    assert(thirdAcceptance.acceptance.user_id === thirdProfile.userId, "third user should accept the cloud invite as itself")
    const cloudMembers = await terminal.client.collaboration.sessionMembers(created.session.id)
    assert(cloudMembers.members.some(member => member.user_id === peerProfile.userId)
      && cloudMembers.members.some(member => member.user_id === thirdProfile.userId), "Cloud membership should include accepted collaborators")
    // MP-08 / MP-10 / MP-11: distinct personal accounts must list the shared
    // account's session using their own private CLIENT authority.
    for (const [name, login] of [["peer", peerLogin], ["third", thirdLogin]]) {
      stage(`cloud-${name}-session-members`)
      assert(login.profile.accountId !== kernelProfile.accountId, "collaborator must have a distinct personal account")
      const listed = await login.client.collaboration.sessionMembers(created.session.id)
      assert(listed.members.some(member => member.user_id === kernelProfile.userId)
        && listed.members.some(member => member.user_id === peerProfile.userId)
        && listed.members.some(member => member.user_id === thirdProfile.userId), "collaborator should list all shared session members")
    }

    stage("cloud-peer-relay-join")
    let peerRemoteClient = await connectSessionScopedCloudClient(modules, peerLogin, scope)
    lifecycle.guardClient(peerRemoteClient)
    scopedClients.push(peerRemoteClient)
    let thirdRemoteClient = await connectSessionScopedCloudClient(modules, thirdLogin, scope)
    lifecycle.guardClient(thirdRemoteClient)
    scopedClients.push(thirdRemoteClient)
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

      await peerRemoteClient.close(); await thirdRemoteClient.close()
      // MP-08 / MP-10 / MP-11: the two-use invite is exhausted. Reconstruct
      // both terminals from their saved private profiles without accepting again.
      stage("cloud-collaborator-terminal-restart")
      peerLogin.client.stop(); thirdLogin.client.stop()
      peerLogin = {...createCloudDrillClient(modules, stateRoot, "peer-terminal"), profile: peerProfile}
      thirdLogin = {...createCloudDrillClient(modules, stateRoot, "third-terminal"), profile: thirdProfile}
      terminals.push(peerLogin, thirdLogin)
      await peerLogin.client.resume(); await thirdLogin.client.resume()

      peerRemoteClient = await connectSessionScopedCloudClient(modules, peerLogin, scope)
      thirdRemoteClient = await connectSessionScopedCloudClient(modules, thirdLogin, scope)
      for (const [client, clientId] of [[peerRemoteClient, peerClientId], [thirdRemoteClient, thirdClientId]]) {
        lifecycle.guardClient(client); scopedClients.push(client)
        const reattached = unwrap(await client.send(requests.attachToSessionRequest(created.session.id, `${clientId}-restarted`)), "SessionAttached")
        assert(reattached.attachment?.session_id === created.session.id, "restarted collaborator should reattach without rejoining")
      }
      for (const [name, login] of [["peer", peerLogin], ["third", thirdLogin]]) {
        stage(`cloud-${name}-restored-members-and-contacts`)
        const restored = await login.client.collaboration.sessionMembers(created.session.id)
        assert(restored.members.some(member => member.user_id === login.profile.userId), "restarted collaborator should list shared members without accepting again")
        const contacts = await login.client.collaboration.collaborators()
        assert(contacts.some(contact => contact.user_id === kernelProfile.userId), "restarted collaborator should list the inviter from its shared account")
      }
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

      stage("cloud-session-scoped-workflow-assertions")
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

    lifecycle.check()
    succeeded = true
  }, async () => {
    const cleanupFailures = []
    const settle = async work => {
      try { await work() } catch { cleanupFailures.push("owned resource cleanup failed") }
    }
    for (const client of scopedClients) client.destroy()
    await remoteClient?.close().catch(() => {})
    await localClient?.close().catch(() => {})
    await settle(() => terminateChild(daemon, "SIGINT"))
    if (terminal && profileRef.current) {
      await waitForCloudRelayTarget(terminal.client, { daemonId, status: "OFFLINE" }, 10_000)
        .catch(() => log("cloud-offline-presence-timeout"))
    }
    for (const entry of terminals) entry.client.stop()
    await settle(() => terminateChild(relay))
    await settle(() => terminateChild(cloudServer))
    await settle(() => db?.account.deleteMany({ where: { slug: { in: [runId, `${runId}-peer`, `${runId}-third`] } } }))
    await settle(() => db?.user.deleteMany({ where: { email: { in: [`${runId}@example.com`, `${runId}-peer@example.com`, `${runId}-third@example.com`] } } }))
    await settle(() => db?.$disconnect())
    await settle(() => rm(stateRoot, { recursive: true, force: true }))
    if (cleanupFailures.length) { succeeded = false; failure ??= new Error("Cloud relay drill cleanup incomplete") }
    const metadata = {
      mpItems: ["MP-08", "MP-10", "MP-11"], sourceCommits, runId,
      apiUrl, relayUrl: `ws://127.0.0.1:${ports.relayPort}`, daemonId, daemonAlias,
      clientId, cloudRoot, kernelCredentialOnly, noticeCount: notices.length,
      cleanupComplete: cleanupFailures.length === 0,
    }
    await writeFile(path.join(rootDir, "result.json"), `${JSON.stringify({ passed: succeeded, ...metadata }, null, 2)}\n`)
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: true,
      failure,
      preserveOnSuccess: true,
      metadata,
    })
  }, error => { failure = error; succeeded = false })
  if (failure) throw failure
  console.log(`MP-08 / MP-10 / MP-11: live cloud ${kernelCredentialOnly ? "kernel credential" : "relay"} drill passed`)
}

if (process.argv.includes("--check")) {
  await loadCloudRelayDrillModules()
  console.log("MP-08 / MP-10 / MP-11: live Cloud relay drill runtime imports passed")
} else {
  await main()
}
