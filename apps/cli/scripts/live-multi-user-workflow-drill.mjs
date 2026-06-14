#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { createHmac } from 'node:crypto'
import { access, mkdir, rm } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const RELAY_ISSUER = 'arroba-multi-user-workflow-drill'
const RELAY_SECRET = 'arroba-multi-user-workflow-drill-secret'
const RELAY_REALM = 'multi-user-workflow-drill'
const DEFAULT_WORKSPACE = repoRoot
const DEFAULT_WORKTREE = repoRoot
const DEFAULT_PROVIDER = 'codex'
const DEFAULT_MODEL = 'gpt-5.4-mini'

function nowStamp() {
  return new Date().toISOString().replace(/[:.]/g, '-')
}

async function loadCliModules(runtimeDir) {
  const [{ transformAsync }, tsPreset] = await Promise.all([
    import('@babel/core'),
    import('@babel/preset-typescript'),
  ])
  for (const rel of ['src/ipc.ts', 'src/ipc-requests.ts']) {
    const sourcePath = path.join(cliRoot, rel)
    const outPath = path.join(runtimeDir, path.basename(rel).replace(/\.tsx?$/, '.js'))
    const code = await (await import('node:fs/promises')).readFile(sourcePath, 'utf8')
    const transformed = await transformAsync(code, {
      filename: sourcePath,
      presets: [[tsPreset.default ?? tsPreset]],
      sourceMaps: false,
    })
    await (await import('node:fs/promises')).writeFile(outPath, transformed?.code ?? '', 'utf8')
  }
  const ipcUrl = pathToFileURL(path.join(runtimeDir, 'ipc.js')).href
  const requestsUrl = pathToFileURL(path.join(runtimeDir, 'ipc-requests.js')).href
  const { LocalIpcClient } = await import(ipcUrl)
  const requests = await import(requestsUrl)
  return { LocalIpcClient, requests }
}

function parseArgs(argv) {
  const options = {
    workspace: DEFAULT_WORKSPACE,
    worktree: DEFAULT_WORKTREE,
    provider: DEFAULT_PROVIDER,
    model: DEFAULT_MODEL,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--workspace') options.workspace = argv[++index]
    else if (arg === '--worktree') options.worktree = argv[++index]
    else if (arg === '--provider') options.provider = argv[++index]
    else if (arg === '--model') options.model = argv[++index]
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-multi-user-workflow-drill.mjs [options]',
    '',
    'Options:',
    `  --workspace ${DEFAULT_WORKSPACE}`,
    `  --worktree ${DEFAULT_WORKTREE}`,
    `  --provider ${DEFAULT_PROVIDER}`,
    `  --model ${DEFAULT_MODEL}`,
  ].join('\n'))
}

function modelForProvider(provider, model) {
  if (provider === 'opencode' && !model.includes('/')) return `opencode/${model}`
  if (provider === 'codex' && !model.includes('/')) return opencodeCodexModel(model)
  return model
}

function opencodeCodexModel(model) {
  if (model.endsWith('-codex')) return model
  if (/^gpt-5\.[23]$/.test(model)) return `${model}-codex`
  return model
}

function base64url(input) {
  return Buffer.from(input).toString('base64url')
}

function signRelayToken(claims) {
  const claimsPayload = base64url(JSON.stringify(claims))
  const signature = createHmac('sha256', RELAY_SECRET).update(claimsPayload).digest('base64url')
  return `arroba-scoped-v1.${claimsPayload}.${signature}`
}

function relayClaims({ subject, subjectKind, actions, userId = null }) {
  return {
    issuer: RELAY_ISSUER,
    subject,
    subject_kind: subjectKind,
    realm_id: RELAY_REALM,
    allowed_actions: actions,
    allowed_targets: null,
    issued_at_ms: Date.now(),
    expires_at_ms: Date.now() + 10 * 60_000,
    token_id: `${subject}-${Date.now()}`,
    account_id: 'multi-user-workflow-drill-account',
    organization_id: null,
    user_id: userId,
    device_id: subject,
    machine_id: subjectKind === 'kernel' || subjectKind === 'machine' ? subject : null,
    client_id: subjectKind === 'client' ? subject : null,
    public_key_thumbprint: `${subject}-thumbprint`,
    entitlements_version: 'drill',
  }
}

function makePorts() {
  const base = 47000 + Math.floor(Math.random() * 1000)
  return {
    relayPort: base,
    kernelPort: base + 1000,
    mcpPort: base + 2000,
    opencodePort: base + 3000,
    codexPort: base + 3001,
  }
}

function makeChildrenEnv(ports, rootDir) {
  const daemonId = `multi-user-workflow-daemon-${process.pid}-${Date.now()}`
  const daemonAlias = `multi-user-workflow-${process.pid}`
  const daemonRelayToken = signRelayToken(relayClaims({
    subject: daemonId,
    subjectKind: 'kernel',
    actions: ['daemon_register', 'daemon_heartbeat', 'peer_request', 'peer_event'],
    userId: 'kernel-owner',
  }))
  return {
    daemonAlias,
    relayUrl: `ws://127.0.0.1:${ports.relayPort}`,
    relayEnv: {
      ...process.env,
      ARROBA_RELAY_HOST: '127.0.0.1',
      ARROBA_RELAY_PORT: String(ports.relayPort),
      ARROBA_RELAY_SCOPED_ISSUER: RELAY_ISSUER,
      ARROBA_RELAY_SCOPED_HMAC_SECRET: RELAY_SECRET,
    },
    daemonEnv: {
      ...process.env,
      ARROBA_KERNEL_PORT: String(ports.kernelPort),
      ARROBA_MCP_PORT: String(ports.mcpPort),
      ARROBA_OPENCODE_PORT: String(ports.opencodePort),
      ARROBA_CODEX_PORT: String(ports.codexPort),
      ARROBA_RELAY_URL: `ws://127.0.0.1:${ports.relayPort}`,
      ARROBA_RELAY_TOKEN: daemonRelayToken,
      ARROBA_DAEMON_ID: daemonId,
      ARROBA_DAEMON_ALIAS: daemonAlias,
      ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
      ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'session-history'),
    },
  }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

async function resolveBinary(binaryPath, manifestPath, binName) {
  try {
    await access(binaryPath)
    return binaryPath
  } catch {
    throw new Error(`missing built binary ${binaryPath}; run cargo build --manifest-path ${manifestPath} --bin ${binName} first`)
  }
}

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    sleep(5_000),
  ])
  if (child.exitCode == null) {
    child.kill('SIGKILL')
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      sleep(2_000),
    ])
  }
}

function spawnObserved(label, command, args, options) {
  const child = spawn(command, args, options)
  const startupError = new Promise((_, reject) => {
    child.once('error', (error) => {
      reject(new Error(`${label} failed to start: ${error.message}`))
    })
  })
  startupError.catch(() => {})
  return { child, startupError }
}

function unwrapVariant(resp, ...keys) {
  for (const key of keys) {
    if (resp?.[key] != null) return resp[key]
  }
  return resp
}

function requireCondition(condition, message, detail = null) {
  if (!condition) {
    const suffix = detail == null ? '' : `\n${JSON.stringify(detail, null, 2)}`
    throw new Error(`${message}${suffix}`)
  }
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

function logStep(name, details = null) {
  if (details == null) console.log(`[multi-user-workflow-drill] ${name}`)
  else console.log(`[multi-user-workflow-drill] ${name}`, JSON.stringify(details))
}

function clientToken(userId) {
  return signRelayToken(relayClaims({
    subject: `client-${userId}-${process.pid}`,
    subjectKind: 'client',
    actions: ['client_connect', 'client_metadata_read', 'packet_route'],
    userId,
  }))
}

function clientFor(LocalIpcClient, relayUrl, daemonAlias, userId) {
  return new LocalIpcClient(relayUrl, {
    relayAuthToken: clientToken(userId),
    targetDaemonAlias: daemonAlias,
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
}

async function waitForRelayTarget(LocalIpcClient, relayUrl, daemonAlias) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = clientFor(LocalIpcClient, relayUrl, daemonAlias, 'user-1')
    try {
      await Promise.race([
        client.send({ ListSessions: null }),
        sleep(2_000).then(() => { throw new Error('probe timeout') }),
      ])
      await client.close()
      return
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error)
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`relay target did not become reachable: ${lastError}`)
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

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }

  const runtimeDir = path.join(cliRoot, '.tmp-live-multi-user-workflow-drill')
  const rootDir = path.join(repoRoot, '.artifacts', 'live-multi-user-workflow-drill', nowStamp())
  await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
  await prepareDrillArtifacts(rootDir)
  await mkdir(runtimeDir, { recursive: true })

  const ports = makePorts()
  const envs = makeChildrenEnv(ports, rootDir)
  let relayChild = null
  let daemonChild = null
  const clients = []
  let sessionId = null
  let endSessionRequest = null
  let passed = false
  let failure = null
  let providerModel = null
  let workflow = null
  let user1Agent = null
  let user2Agent = null
  let user2PlacedUser1Node = null
  let user2Node = null
  let edge = null
  let removedWorkflow = null
  let user2State = null

  try {
    const { LocalIpcClient, requests } = await loadCliModules(runtimeDir)
    const {
      createSessionRequest,
      createSessionInviteRequest,
      joinSessionInviteRequest,
      listSessionMembersRequest,
      listSessionsRequest,
      getSessionStateRequest,
      spawnAgentRequest,
      listAgentsRequest,
      createWorkflowRequest,
      resolveWorkflowRequest,
      invokeWorkflowEndpointRequest,
    } = requests
    ;({ endSessionRequest } = requests)

    logStep('start_relay', { relayUrl: envs.relayUrl })
    const relayProcess = spawnObserved('relay', 'cargo', ['run', '--manifest-path', path.join(repoRoot, 'apps/relay/Cargo.toml'), '--bin', 'arroba-relay'], {
      cwd: repoRoot,
      env: envs.relayEnv,
      stdio: ['ignore', 'ignore', 'inherit'],
    })
    relayChild = relayProcess.child
    const daemonBinary = await resolveBinary(
      path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel'),
      path.join(repoRoot, 'apps/kernel/Cargo.toml'),
      'arroba-kernel',
    )
    logStep('start_kernel', { daemonAlias: envs.daemonAlias })
    const daemonProcess = spawnObserved('kernel', daemonBinary, [], {
      cwd: repoRoot,
      env: envs.daemonEnv,
      stdio: ['ignore', 'ignore', 'inherit'],
    })
    daemonChild = daemonProcess.child
    await Promise.race([
      waitForRelayTarget(LocalIpcClient, envs.relayUrl, envs.daemonAlias),
      relayProcess.startupError,
      daemonProcess.startupError,
    ])

    const user1 = clientFor(LocalIpcClient, envs.relayUrl, envs.daemonAlias, 'user-1')
    const user2 = clientFor(LocalIpcClient, envs.relayUrl, envs.daemonAlias, 'user-2')
    const user3 = clientFor(LocalIpcClient, envs.relayUrl, envs.daemonAlias, 'user-3')
    clients.push(user1, user2, user3)

    logStep('create_session_as_user_1')
    const sessionCreated = unwrapVariant(
      await user1.send(createSessionRequest(options.workspace, options.worktree)),
      'SessionCreated',
    )
    const session = sessionCreated.session
    sessionId = session.id
    requireCondition(session.owner_user_id === 'user-1', 'session owner should be relay caller user-1', session)

    const inviteCreated = unwrapVariant(
      await user1.send(createSessionInviteRequest(session.id, null, 2)),
      'SessionInviteCreated',
    )
    const inviteToken = inviteCreated.invite.invite_token
    logStep('join_session_as_user_2_and_user_3')
    await user2.send(joinSessionInviteRequest(inviteToken, 'user-2'))
    await user3.send(joinSessionInviteRequest(inviteToken, 'user-3'))
    const members = unwrapVariant(await user1.send(listSessionMembersRequest(session.id)), 'SessionMembersListed').members
    requireCondition(
      ['user-1', 'user-2', 'user-3'].every((userId) => members.some((member) => member.user_id === userId)),
      'session members should include all three drill users',
      members,
    )
    const user2Sessions = unwrapVariant(await user2.send(listSessionsRequest()), 'SessionsListed').sessions
    requireCondition(user2Sessions.some((entry) => entry.id === session.id), 'user-2 should see joined session', user2Sessions)

    logStep('spawn_user_owned_agents')
    providerModel = modelForProvider(options.provider, options.model)
    user1Agent = unwrapVariant(
      await user1.send(spawnAgentRequest(session.id, options.provider, 'user-one-agent', providerModel, options.worktree, 'low')),
      'AgentSpawned',
    ).agent
    user2Agent = unwrapVariant(
      await user2.send(spawnAgentRequest(session.id, options.provider, 'user-two-agent', providerModel, options.worktree, 'low')),
      'AgentSpawned',
    ).agent
    requireCondition(user1Agent.owner_user_id === 'user-1', 'user-1 agent owner mismatch', user1Agent)
    requireCondition(user2Agent.owner_user_id === 'user-2', 'user-2 agent owner mismatch', user2Agent)

    const user2Agents = unwrapVariant(await user2.send(listAgentsRequest(session.id)), 'AgentsListed').agents
    requireCondition(
      user2Agents.some((agent) => agent.id === user2Agent.id)
        && user2Agents.some((agent) => agent.id === user1Agent.id && agent.provider === 'redacted'),
      'private user-2 should list its own agent plus redacted workflow-selectable owner handles',
      user2Agents,
    )

    logStep('mutate_shared_workflow')
    workflow = unwrapVariant(
      await user1.send(createWorkflowRequest(session.id, 'multi-user-live-flow')),
      'WorkflowCreated',
    ).workflow
    const resolvedBeforeUser2Node = unwrapVariant(
      await user2.send(resolveWorkflowRequest(session.id, workflow.id)),
      'WorkflowResolved',
    ).workflow
    user2PlacedUser1Node = unwrapVariant(
      await user2.send(addWorkflowNodeRequest(session.id, workflow.id, user1Agent.id, resolvedBeforeUser2Node.revision)),
      'WorkflowNodeAdded',
    ).node
    requireCondition(
      user2PlacedUser1Node.agent_id === user1Agent.id && user2PlacedUser1Node.created_by_user_id === 'user-2',
      'private user-2 should be able to add user-1 agent as a workflow node',
      user2PlacedUser1Node,
    )
    const user1Node = user2PlacedUser1Node
    const resolvedAfterSharedNode = unwrapVariant(
      await user2.send(resolveWorkflowRequest(session.id, workflow.id)),
      'WorkflowResolved',
    ).workflow
    user2Node = unwrapVariant(
      await user2.send(addWorkflowNodeRequest(session.id, workflow.id, user2Agent.id, resolvedAfterSharedNode.revision)),
      'WorkflowNodeAdded',
    ).node
    await user2.send(updateWorkflowNodeInstructionsRequest(
      session.id,
      workflow.id,
      user2Node.id,
      'private prompt from user-2',
    ))

    const endpoint = unwrapVariant(
      await user1.send(createWorkflowEndpointRequest(session.id, workflow.id, user1Node.id, 'user-one-entry')),
      'WorkflowEndpointCreated',
    ).endpoint
    await expectReject(
      user2.send(invokeWorkflowEndpointRequest(session.id, workflow.id, endpoint.id, 'should be denied')),
      'user-2 invoking user-1 endpoint',
      'owned by `user-1`',
    )

    const beforeEdge = unwrapVariant(
      await user2.send(resolveWorkflowRequest(session.id, workflow.id)),
      'WorkflowResolved',
    ).workflow
    edge = unwrapVariant(
      await user2.send(addWorkflowEdgeRequest(session.id, workflow.id, user1Node.id, user2Node.id, beforeEdge.revision)),
      'WorkflowEdgeAdded',
    ).edge
    requireCondition(edge.created_by_user_id === 'user-2', 'cross-owner edge should record creating user', edge)

    await expectReject(
      user3.send(removeWorkflowEdgeRequest(session.id, workflow.id, edge.id)),
      'user-3 removing edge unrelated to its nodes',
      'cannot perform',
    )

    const beforeStaleMutation = unwrapVariant(
      await user1.send(resolveWorkflowRequest(session.id, workflow.id)),
      'WorkflowResolved',
    ).workflow
    await user2.send(updateWorkflowNodeInstructionsRequest(
      session.id,
      workflow.id,
      user2Node.id,
      'private prompt from user-2 after revision bump',
    ))
    await expectReject(
      user2.send(updateWorkflowNodeInstructionsRequest(
        session.id,
        workflow.id,
        user2Node.id,
        'stale private prompt from user-2',
        beforeStaleMutation.revision,
      )),
      'stale workflow revision mutation',
      'expected',
    )

    const freshWorkflow = unwrapVariant(
      await user1.send(resolveWorkflowRequest(session.id, workflow.id)),
      'WorkflowResolved',
    ).workflow
    removedWorkflow = unwrapVariant(
      await user1.send(removeWorkflowEdgeRequest(session.id, workflow.id, edge.id, freshWorkflow.revision)),
      'WorkflowEdgeRemoved',
    ).workflow
    requireCondition(removedWorkflow.edges.length === 0, 'user-1 should remove edge incident to its own node', removedWorkflow)

    logStep('verify_redacted_shared_projection')
    user2State = unwrapVariant(await user2.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState').session
    requireCondition(
      user2State.agents.some((agent) => agent.id === user2Agent.id && agent.provider !== 'redacted')
        && user2State.agents.some((agent) => agent.id === user1Agent.id && agent.provider === 'redacted'),
      'user-2 state should retain own agent and redact user-1 agent details',
      user2State.agents,
    )
    const redactedWorkflow = user2State.workflows.find((entry) => entry.id === workflow.id)
    requireCondition(redactedWorkflow != null, 'user-2 should see shared workflow graph', user2State.workflows)
    const placedUser1Node = redactedWorkflow.nodes.find((node) => node.id === user2PlacedUser1Node.id)
    const visibleUser2Node = redactedWorkflow.nodes.find((node) => node.id === user2Node.id)
    requireCondition(placedUser1Node != null, 'user-2 should see the node it created for user-1 agent', redactedWorkflow)
    requireCondition(visibleUser2Node != null, 'user-2 should see own node', redactedWorkflow)
    requireCondition(placedUser1Node.agent_id === user1Agent.id, 'user-2 placed node should preserve backing user-1 agent id', placedUser1Node)
    requireCondition(placedUser1Node.instructions == null, 'user-1 backing agent node instructions should stay unset/redacted from user-2', placedUser1Node)
    requireCondition(visibleUser2Node.instructions === 'private prompt from user-2 after revision bump', 'user-2 node instructions should remain visible to owner', visibleUser2Node)

    console.log(JSON.stringify({
      status: 'ok',
      mode: 'multi-user-workflow-relay',
      relayUrl: envs.relayUrl,
      daemonAlias: envs.daemonAlias,
      sessionId: session.id,
      provider: options.provider,
      model: providerModel,
      users: ['user-1', 'user-2', 'user-3'],
      workflowId: workflow.id,
      nodes: [
        { id: user2PlacedUser1Node.id, ownerUserId: user2PlacedUser1Node.owner_user_id, createdByUserId: user2PlacedUser1Node.created_by_user_id, publicLabel: user2PlacedUser1Node.public_label },
        { id: user2Node.id, ownerUserId: user2Node.owner_user_id, publicLabel: user2Node.public_label },
      ],
      assertions: [
        'session membership over scoped relay',
        'private collaborator receives redacted workflow-selectable agent handles',
        'private collaborator can add another user agent as node',
        'cannot invoke another user endpoint',
        'cross-owner edge allowed when touching caller node',
        'unrelated user cannot remove edge',
        'stale workflow revision rejected',
        'edge incident to caller node removable by caller',
        'other-user node instructions redacted',
      ],
    }, null, 2))
    passed = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (sessionId && clients[0] && endSessionRequest) {
      await clients[0].send(endSessionRequest(sessionId)).catch(() => {})
    }
    await Promise.all(clients.map((client) => client.close().catch(() => {})))
    await terminateChild(daemonChild)
    await terminateChild(relayChild)
    await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
    await finalizeDrillArtifacts({
      rootDir,
      passed,
      preserveOnFailure: true,
      failure,
      metadata: {
        drill: 'live-multi-user-workflow',
        mode: 'multi-user-workflow-relay',
        relayUrl: envs.relayUrl,
        daemonAlias: envs.daemonAlias,
        sessionId,
        provider: options.provider,
        model: providerModel ?? modelForProvider(options.provider, options.model),
        workflowId: workflow?.id ?? null,
        agents: [
          user1Agent ? { id: user1Agent.id, ownerUserId: user1Agent.owner_user_id } : null,
          user2Agent ? { id: user2Agent.id, ownerUserId: user2Agent.owner_user_id } : null,
        ].filter(Boolean),
        nodes: [
          user2PlacedUser1Node ? { id: user2PlacedUser1Node.id, ownerUserId: user2PlacedUser1Node.owner_user_id, createdByUserId: user2PlacedUser1Node.created_by_user_id } : null,
          user2Node ? { id: user2Node.id, ownerUserId: user2Node.owner_user_id, createdByUserId: user2Node.created_by_user_id } : null,
        ].filter(Boolean),
        edgeId: edge?.id ?? null,
        remainingEdgeCount: removedWorkflow?.edges?.length ?? null,
        user2Projection: user2State ? {
          agentCount: user2State.agents?.length ?? 0,
          workflowCount: user2State.workflows?.length ?? 0,
        } : null,
      },
    })
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
