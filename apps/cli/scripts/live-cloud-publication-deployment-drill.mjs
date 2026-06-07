#!/usr/bin/env node
import { spawn } from 'node:child_process'
import net from 'node:net'
import path from 'node:path'
import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { WebSocket } from 'ws'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')
const { createDefaultShellContext, parseShellCommand } = await import('../../../packages/kernel-client/dist/shell-core.js')
const { executeShellCommand } = await import('../../../packages/kernel-client/dist/shell-executor.js')
const { loadPreferences, relayCloudProfile } = await import('../dist/preferences.js')
const { connectKernelCloudRelay } = await import('../dist/relay-api.js')
const {
  changePublicationDeployment,
  createPublicationDeploymentFromPackage,
  getPublicationDeployment,
  listPublicationDeploymentLogs,
} = await import('../dist/publication-deployment-api.js')

const {
  addWorkflowNodeRequest,
  attachToSessionRequest,
  createSessionRequest,
  createWorkflowEndpointRequest,
  createWorkflowPublicationRequest,
  createWorkflowRequest,
  endSessionRequest,
  getProviderRunRequest,
  getDaemonHealthRequest,
  launchProviderRunRequest,
  setWorkflowNodeCanCompleteRunRequest,
  setWorkflowNodeCanEmitIntermediateOutputRequest,
  spawnAgentRequest,
  updateWorkflowNodeInstructionsRequest,
} = requests

const REAL_DASHBOARD_PROMPT = [
  'Generate a vibrant dashboard as a compact self-contained HTML document.',
  'The dashboard must visibly include the title text `Real Provider Workflow Dashboard`.',
  'The main dashboard element must include `data-arroba-real-provider-dashboard="true"`.',
  'Before writing the file, reason through a compact layout plan that balances mobile responsiveness, contrast, KPI cards, one chart-like visual, and a status section.',
  'Use inline CSS only; no scripts, external assets, network calls, or unrelated file inspection.',
  'This is the final workflow node: submit final workflow output as {"kind":"html","html":"<full html document>"} by calling validate_and_submit_workflow_run_output, then emit the final fenced workflow JSON block with the same output.message object.',
].join(' ')

function usage() {
  return [
    'Usage: node apps/cli/scripts/live-cloud-publication-deployment-drill.mjs --mode hosted-container|local-runtime [options]',
    '',
    'Options:',
    '  --transport human_http|api_sse_json|websocket_json|mcp',
    '  --slug SLUG',
    '  --provider dev-stub|codex|opencode|claude',
    '  --model MODEL',
    '  --credential-profile NAME',
    '  --real-dashboard',
    '  --artifacts-dir DIR',
    '  --keep-tmp',
    '  --debug-hold-ms MS',
  ].join('\n')
}

function parseArgs(argv) {
  const options = {
    mode: null,
    transport: 'human_http',
    provider: 'dev-stub',
    model: null,
    slug: null,
    credentialProfile: null,
    realDashboard: false,
    artifactsDir: path.join(repoRoot, '.artifacts', 'cloud-publication-deployments'),
    keepTmp: false,
    debugHoldMs: 0,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--mode') options.mode = normalizeMode(argv[++index])
    else if (arg === '--transport') options.transport = normalizeTransport(argv[++index])
    else if (arg === '--provider') options.provider = argv[++index]
    else if (arg === '--model') options.model = argv[++index]
    else if (arg === '--slug') options.slug = argv[++index]
    else if (arg === '--credential-profile') options.credentialProfile = argv[++index]
    else if (arg === '--real-dashboard') options.realDashboard = true
    else if (arg === '--artifacts-dir') options.artifactsDir = path.resolve(argv[++index])
    else if (arg === '--keep-tmp') options.keepTmp = true
    else if (arg === '--debug-hold-ms') options.debugHoldMs = Number.parseInt(argv[++index] ?? '', 10)
    else if (arg === '--help' || arg === '-h') {
      console.log(usage())
      process.exit(0)
    } else {
      throw new Error(`unknown option ${arg}\n${usage()}`)
    }
  }
  if (!options.mode) throw new Error(`--mode is required\n${usage()}`)
  if (!options.model) options.model = defaultModel(options.provider, options.transport, options.realDashboard)
  if (!options.slug) options.slug = `cloud-${options.mode.replace('_', '-')}-${options.transport.replace(/_/g, '-')}-${Date.now()}`
  if (!Number.isFinite(options.debugHoldMs) || options.debugHoldMs < 0) throw new Error('--debug-hold-ms must be a non-negative integer')
  return options
}

function normalizeMode(value) {
  if (value === 'hosted-container' || value === 'hosted_container') return 'hosted_container'
  if (value === 'local-runtime' || value === 'local_runtime') return 'local_runtime'
  throw new Error('--mode must be hosted-container or local-runtime')
}

function normalizeTransport(value) {
  if (['human_http', 'api_sse_json', 'websocket_json', 'mcp'].includes(value)) return value
  throw new Error('--transport must be human_http, api_sse_json, websocket_json, or mcp')
}

function defaultModel(provider, transport, realDashboard) {
  if (provider === 'dev-stub') {
    if (realDashboard) return 'workflow-html-final-node'
    if (transport === 'mcp') return 'workflow-single-turn-node'
    return 'workflow-intermediate-node'
  }
  if (provider === 'codex') return 'gpt-5.4'
  if (provider === 'opencode') return 'opencode/gpt-5.4'
  if (provider === 'claude') return 'claude-sonnet-4-6'
  return 'gpt-5.5'
}

function logStep(name, details = null) {
  if (details == null) console.log(`[cloud-publication-drill] ${name}`)
  else console.log(`[cloud-publication-drill] ${name}`, JSON.stringify(details))
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  await mkdir(options.artifactsDir, { recursive: true })
  await mkdir(path.join(repoRoot, '.tmp'), { recursive: true })
  const root = await mkdtemp(path.join(repoRoot, '.tmp', 'arroba-cloud-publication-drill-'))
  const workspace = path.join(root, 'workspace')
  const exportDir = path.join(root, 'exported')
  const packageDir = path.join(root, 'package')
  const configHome = path.join(root, 'config')
  const serverConfigHome = path.join(root, 'server-config')
  const stateHome = path.join(root, 'state')
  const home = path.join(root, 'home')
  const kernelPort = await freePort()
  const relayPort = await freePort()
  const mcpPort = await freePort()
  const opencodePort = await freePort()
  const codexPort = await freePort()
  const servePort = await freePort()
  const daemonAlias = `cloud-publication-drill-${process.pid}-${Date.now()}`
  const relayToken = `cloud-publication-drill-${process.pid}-${Date.now()}`
  const useLocalRelay = options.mode !== 'local_runtime'
  const env = {
    ...process.env,
    HOME: options.provider === 'dev-stub' ? home : process.env.HOME,
    XDG_CONFIG_HOME: configHome,
    XDG_STATE_HOME: stateHome,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(mcpPort),
    ARROBA_OPENCODE_PORT: String(opencodePort),
    ARROBA_CODEX_PORT: String(codexPort),
    ARROBA_DAEMON_ID: daemonAlias,
    ARROBA_DAEMON_ALIAS: daemonAlias,
    ARROBA_DAEMON_SOCKET: path.join(root, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(root, 'history'),
    ...(useLocalRelay ? {
      ARROBA_RELAY_URL: `ws://127.0.0.1:${relayPort}`,
      ARROBA_RELAY_TOKEN: relayToken,
    } : {}),
  }
  if (process.env.HOME) {
    if (!env.CODEX_HOME) env.CODEX_HOME = path.join(process.env.HOME, '.codex')
    if (!env.ARROBA_CLAUDE_CONFIG) env.ARROBA_CLAUDE_CONFIG = path.join(process.env.HOME, '.claude.json')
  }
  let kernel = null
  let relay = null
  let serve = null
  let client = null
  let deploymentId = null
  let succeeded = false
  const sessionIds = []
  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(path.join(configHome, 'arroba'), { recursive: true })
    await mkdir(path.join(serverConfigHome, 'arroba'), { recursive: true })
    await writeFile(path.join(configHome, 'arroba', 'config.toml'), 'version = 1\n', 'utf8')
    await writeFile(path.join(serverConfigHome, 'arroba', 'config.toml'), 'version = 1\n', 'utf8')
    await copyCloudProfile(configHome).catch(() => {})

    const kernelBinary = await buildRustBinary('arroba-kernel')
    const relayBinary = await buildRustBinary('arroba-relay')
    if (useLocalRelay) {
      relay = startProcess(relayBinary, [], {
        ...env,
        ARROBA_RELAY_HOST: '127.0.0.1',
        ARROBA_RELAY_PORT: String(relayPort),
        ARROBA_RELAY_TOKEN: relayToken,
      }, 'relay')
      await waitForTcpPort('127.0.0.1', relayPort)
    }
    kernel = startProcess(kernelBinary, [], env, 'kernel')
    const kernelUrl = `ws://127.0.0.1:${kernelPort}`
    await waitForKernel(kernelUrl)
    client = new LocalIpcClient(kernelUrl, { kernelPingIntervalMs: 60_000, kernelMaxMissedPongs: 10 })
    if (options.mode === 'local_runtime') {
      const connected = await connectKernelCloudRelay(client)
      logStep('cloud_relay_connected', { relayUrl: connected.relayUrl, tokenExpiresAtMs: connected.tokenExpiresAtMs })
      await delay(1_000)
    }

    const publicationContext = await createPublicationPackage({
      client,
      sessionIds,
      workspace,
      exportDir,
      packageDir,
      kernelUrl,
      transport: options.transport,
      provider: options.provider,
      model: options.model,
      realDashboard: options.realDashboard,
    })
    const profile = relayCloudProfile(await loadPreferences())
    if (!profile) throw new Error('cloud is not linked. Run /cloud login from the TUI before this drill.')
    logStep('deploy', { mode: options.mode, transport: options.transport, slug: options.slug })
    const deployment = await createPublicationDeploymentFromPackage({
      profile,
      packagePath: packageDir,
      mode: options.mode,
      slug: options.slug,
      ...(options.credentialProfile ? { credentialProfile: options.credentialProfile } : {}),
      start: options.mode === 'hosted_container',
    })
    deploymentId = deployment.id
    let readyDeployment = deployment
    if (options.mode === 'local_runtime') {
      serve = startProcess(process.execPath, [path.join(repoRoot, 'apps/server/dist/index.js')], {
        ...env,
        XDG_CONFIG_HOME: serverConfigHome,
        HOST: '127.0.0.1',
        PORT: String(servePort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_PACKAGE: packageDir,
        ARROBA_PUBLICATION_CLOUD_DEPLOYMENT_ID: deployment.id,
        ARROBA_PUBLICATION_CLOUD_API_URL: profile.apiUrl,
        ARROBA_PUBLICATION_CLOUD_ACCOUNT_ID: profile.accountId,
        ...(profile.cloudSessionToken ? { ARROBA_PUBLICATION_CLOUD_SESSION_TOKEN: profile.cloudSessionToken } : {}),
      }, 'arroba-serve-local-runtime')
      await waitForGateway(`http://127.0.0.1:${servePort}`).catch((error) => {
        throw new Error(`${errorMessage(error)}\nserve stdout:\n${serve?.logs?.stdout ?? ''}\nserve stderr:\n${serve?.logs?.stderr ?? ''}`)
      })
    }
    readyDeployment = await waitForDeploymentReady(profile, deployment.id)
    logStep('deployment_ready', {
      id: readyDeployment.id,
      status: readyDeployment.status,
      publicBaseUrl: readyDeployment.publicBaseUrl,
    })
    const evidence = await validateTransport({
      transport: options.transport,
      publicBaseUrl: readyDeployment.publicBaseUrl,
      prompt: options.realDashboard ? REAL_DASHBOARD_PROMPT : `cloud deployment drill ${options.transport}`,
      artifactsDir: options.artifactsDir,
      slug: options.slug,
      expectHtmlDashboard: options.realDashboard,
    }).catch((error) => {
      if (options.debugHoldMs > 0) {
        logStep('debug_hold', {
          ms: options.debugHoldMs,
          publicBaseUrl: readyDeployment.publicBaseUrl,
          deploymentId: readyDeployment.id,
        })
        return delay(options.debugHoldMs).then(() => {
          throw error
        })
      }
      throw new Error(`${errorMessage(error)}\nkernel stdout:\n${kernel?.logs?.stdout ?? ''}\nkernel stderr:\n${kernel?.logs?.stderr ?? ''}\nserve stdout:\n${serve?.logs?.stdout ?? ''}\nserve stderr:\n${serve?.logs?.stderr ?? ''}`)
    })
    const logs = await listPublicationDeploymentLogs(profile, deployment.id).catch((error) => [{ level: 'error', occurredAt: new Date().toISOString(), message: String(error) }])
    const statePath = path.join(options.artifactsDir, `${options.slug}-deployment.json`)
    await writeFile(statePath, `${JSON.stringify({ deployment: readyDeployment, publicationContext, evidence, logs }, null, 2)}\n`)
    logStep('evidence_written', { statePath, ...evidence })
    succeeded = true
  } finally {
    if (!succeeded && deploymentId) {
      const profile = relayCloudProfile(await loadPreferences().catch(() => ({})))
      if (profile) await changePublicationDeployment(profile, deploymentId, 'stop').catch(() => {})
    }
    await stopProcess(serve).catch((error) => logStep('cleanup_warning', { process: 'serve', error: errorMessage(error) }))
    for (const id of sessionIds.reverse()) await client?.send(endSessionRequest(id)).catch(() => {})
    await client?.close?.().catch(() => {})
    await stopProcess(kernel).catch((error) => logStep('cleanup_warning', { process: 'kernel', error: errorMessage(error) }))
    await stopProcess(relay).catch((error) => logStep('cleanup_warning', { process: 'relay', error: errorMessage(error) }))
    if (!options.keepTmp) await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 })
    else logStep('kept_tmp', { root })
  }
}

async function createPublicationPackage(input) {
  const {
    client,
    sessionIds,
    workspace,
    exportDir,
    packageDir,
    kernelUrl,
    transport,
    provider,
    model,
    realDashboard,
  } = input
  const session = (await client.send(createSessionRequest(workspace, workspace, `cloud-publication-${transport}`))).SessionCreated.session
  sessionIds.push(session.id)
  await client.send(attachToSessionRequest(session.id, `cloud-publication-drill-${process.pid}`))
  const agent = (await client.send(spawnAgentRequest(session.id, provider, `${provider}-${transport}`, model, workspace, 'high'))).AgentSpawned.agent
  const providerRun = (await client.send(launchProviderRunRequest(session.id, provider, 'default', model, 'high', agent.id))).ProviderRunLaunchAccepted.provider_run
  await waitForProviderRunReady(client, providerRun.id)
  const workflow = (await client.send(createWorkflowRequest(session.id, `cloud-publication-${transport}`))).WorkflowCreated.workflow
  const node = (await client.send(addWorkflowNodeRequest(session.id, workflow.id, agent.id))).WorkflowNodeAdded.node
  await client.send(updateWorkflowNodeInstructionsRequest(
    session.id,
    workflow.id,
    node.id,
    realDashboard
      ? REAL_DASHBOARD_PROMPT
      : 'Complete this cloud publication deployment drill with a concise deterministic final output.',
  ))
  await client.send(setWorkflowNodeCanCompleteRunRequest(session.id, workflow.id, node.id, true))
  await client.send(setWorkflowNodeCanEmitIntermediateOutputRequest(session.id, workflow.id, node.id, true))
  const endpoint = (await client.send(createWorkflowEndpointRequest(session.id, workflow.id, node.id, transport))).WorkflowEndpointCreated.endpoint
  const publication = (await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, publicationOptions(transport, node.id)))).WorkflowPublicationCreated.publication
  const exportResult = await executeShellCommand(
    parseShellCommand(`workflow publication export ${publication.id} ${exportDir} --kernel-url ${kernelUrl}`),
    createDefaultShellContext({ workspace, worktree: workspace, sessionId: session.id, workflowId: workflow.id }),
    { client },
  )
  if (!exportResult.ok) throw new Error(`publication export failed: ${exportResult.message}`)
  await makePortablePackage(exportDir, packageDir)
  return {
    sessionId: session.id,
    workflowId: workflow.id,
    endpointId: endpoint.id,
    publicationId: publication.id,
    nodeId: node.id,
    agentId: agent.id,
    agentAlias: agent.alias ?? null,
  }
}

function publicationOptions(transport, nodeId) {
  const base = {
    alias: `cloud_${transport}`,
    route: routeForTransport(transport),
    methods: methodsForTransport(transport),
    parser: transport === 'human_http'
      ? { kind: 'path_template', template: '/final/:prompt' }
      : { kind: 'json' },
    traceExposure: { nodes: { [nodeId]: ['output_summary', 'assistant_messages', 'thinking', 'tool_use'] } },
    mode: transport === 'human_http' ? 'async' : 'sync',
  }
  if (transport === 'human_http') return { ...base, transport: { kind: 'human_http' } }
  if (transport === 'api_sse_json') return { ...base, transport: { kind: 'api_sse_json' }, parser: { kind: 'json' } }
  if (transport === 'websocket_json') return { ...base, transport: { kind: 'websocket_json' }, parser: { kind: 'json' } }
  if (transport === 'mcp') return { ...base, transport: { kind: 'mcp' }, parser: { kind: 'json' } }
  throw new Error(`unsupported transport ${transport}`)
}

function routeForTransport(transport) {
  if (transport === 'mcp') return '/mcp'
  if (transport === 'websocket_json') return '/.well-known/arroba/publication/ws'
  if (transport === 'api_sse_json') return '/invoke'
  return '/final/*'
}

function methodsForTransport(transport) {
  if (transport === 'human_http') return ['GET']
  return ['POST']
}

async function makePortablePackage(sourceDir, targetDir) {
  await rm(targetDir, { recursive: true, force: true })
  await cp(sourceDir, targetDir, { recursive: true })
  const snapshotPath = path.join(targetDir, 'workflow.snapshot.json')
  const snapshot = JSON.parse(await readFile(snapshotPath, 'utf8'))
  if (snapshot.source_session) {
    snapshot.source_session.workspace_id = '/workspace'
    snapshot.source_session.worktree_id = '/workspace'
  }
  for (const agent of snapshot.agents ?? []) {
    if (agent.workspace_id != null) agent.workspace_id = '/workspace'
    if (agent.worktree_id != null) agent.worktree_id = '/workspace'
  }
  await writeFile(snapshotPath, `${JSON.stringify(snapshot, null, 2)}\n`)
}

async function validateTransport(input) {
  const base = input.publicBaseUrl.replace(/\/+$/, '')
  if (input.transport === 'human_http') {
    const promptUrl = `${base}/final/${encodeURIComponent(input.prompt)}`
    const response = await fetch(promptUrl, { headers: { accept: 'text/html' } })
    const body = await response.text()
    if (!response.ok || !body.includes('EventSource')) throw new Error(`human HTTP viewer failed: ${response.status} ${body.slice(0, 200)}`)
    const viewerConfig = parseHumanHttpViewerConfig(body)
    if (!viewerConfig.eventsUrl) throw new Error(`human HTTP viewer did not expose an events URL:\n${body.slice(0, 1000)}`)
    const eventTranscript = await readSse(`${base}${viewerConfig.eventsUrl}`, null, { method: 'GET' })
    if (!eventTranscript.includes('event: final')) throw new Error(`human HTTP event transcript missing final:\n${eventTranscript}`)
    if (!eventTranscript.includes('event: trace')) throw new Error(`human HTTP event transcript missing trace:\n${eventTranscript}`)
    assertSuccessfulSseTranscript(eventTranscript, 'human HTTP')
    if (input.expectHtmlDashboard) {
      for (const snippet of ['Real Provider Workflow Dashboard', 'data-arroba-real-provider-dashboard']) {
        if (!eventTranscript.includes(snippet)) {
          throw new Error(`human HTTP final transcript missing dashboard snippet ${snippet}:\n${eventTranscript}`)
        }
      }
    }
    const htmlPath = path.join(input.artifactsDir, `${input.slug}-human-http-viewer.html`)
    const transcriptPath = path.join(input.artifactsDir, `${input.slug}-human-http-events.txt`)
    await writeFile(htmlPath, body)
    await writeFile(transcriptPath, eventTranscript)
    return { promptUrl, htmlPath, transcriptPath }
  }
  if (input.transport === 'api_sse_json') {
    const body = await readSse(`${base}/invoke`, { prompt: input.prompt })
    const transcriptPath = path.join(input.artifactsDir, `${input.slug}-api-sse.txt`)
    await writeFile(transcriptPath, body)
    for (const event of ['queued', 'started', 'trace', 'final']) {
      if (!body.includes(`event: ${event}`)) throw new Error(`API SSE transcript missing ${event}:\n${body}`)
    }
    assertSuccessfulSseTranscript(body, 'API SSE')
    if (input.expectHtmlDashboard) {
      for (const snippet of ['Real Provider Workflow Dashboard', 'data-arroba-real-provider-dashboard']) {
        if (!body.includes(snippet)) throw new Error(`API SSE final transcript missing dashboard snippet ${snippet}:\n${body}`)
      }
    }
    return { transcriptPath }
  }
  if (input.transport === 'websocket_json') {
    const events = await invokeWebSocket(`${base}/.well-known/arroba/publication/ws`, { prompt: input.prompt })
    const transcriptPath = path.join(input.artifactsDir, `${input.slug}-websocket.json`)
    await writeFile(transcriptPath, `${JSON.stringify(events, null, 2)}\n`)
    for (const type of ['ready', 'accepted', 'trace', 'final']) {
      if (!events.some((event) => event.type === type)) throw new Error(`WebSocket transcript missing ${type}: ${JSON.stringify(events)}`)
    }
    assertSuccessfulWebSocketEvents(events)
    if (!events.some((event) => event.type === 'queued' || event.type === 'started' || event.type === 'status')) {
      throw new Error(`WebSocket transcript missing queued/started/status progress event: ${JSON.stringify(events)}`)
    }
    if (input.expectHtmlDashboard) {
      const body = JSON.stringify(events)
      for (const snippet of ['Real Provider Workflow Dashboard', 'data-arroba-real-provider-dashboard']) {
        if (!body.includes(snippet)) throw new Error(`WebSocket final transcript missing dashboard snippet ${snippet}: ${body}`)
      }
    }
    return { transcriptPath }
  }
  if (input.transport === 'mcp') {
    const listResponse = await fetch(`${base}/mcp`, {
      method: 'POST',
      headers: { accept: 'application/json', 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list', params: {} }),
    })
    const transcriptPath = path.join(input.artifactsDir, `${input.slug}-mcp-tools-list.json`)
    const listBody = await listResponse.text()
    await writeFile(transcriptPath, listBody)
    if (!listResponse.ok || !listBody.includes('tools')) throw new Error(`MCP tools/list failed: ${listResponse.status} ${listBody}`)
    const toolName = JSON.parse(listBody)?.result?.tools?.[0]?.name
    if (!toolName) throw new Error(`MCP tools/list did not return a tool name: ${listBody}`)
    const callResponse = await fetch(`${base}/mcp`, {
      method: 'POST',
      headers: { accept: 'application/json', 'content-type': 'application/json' },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: 2,
        method: 'tools/call',
        params: { name: toolName, arguments: { prompt: input.prompt } },
      }),
    })
    const callBody = await callResponse.text()
    const callTranscriptPath = path.join(input.artifactsDir, `${input.slug}-mcp-tools-call.json`)
    await writeFile(callTranscriptPath, callBody)
    if (!callResponse.ok || !callBody.includes('content')) throw new Error(`MCP tools/call failed: ${callResponse.status} ${callBody}`)
    assertSuccessfulMcpToolCall(callBody)
    if (input.expectHtmlDashboard) {
      for (const snippet of ['Real Provider Workflow Dashboard', 'data-arroba-real-provider-dashboard']) {
        if (!callBody.includes(snippet)) throw new Error(`MCP tools/call final missing dashboard snippet ${snippet}:\n${callBody}`)
      }
    }
    return { transcriptPath, callTranscriptPath }
  }
  throw new Error(`unsupported transport ${input.transport}`)
}

function assertSuccessfulSseTranscript(transcript, label) {
  const frames = parseSseTranscript(transcript)
  const finalFrame = [...frames].reverse().find((frame) => frame.event === 'final')
  const workflowRun = finalFrame?.data?.workflow_run ?? null
  if (!workflowRun) throw new Error(`${label} transcript final event did not include workflow_run:\n${transcript}`)
  assertWorkflowRunCompleted(workflowRun, `${label} transcript`)
}

function parseSseTranscript(transcript) {
  const frames = []
  for (const frame of transcript.split(/\r?\n\r?\n/)) {
    if (!frame.trim()) continue
    let event = 'message'
    const data = []
    for (const line of frame.split(/\r?\n/)) {
      if (line.startsWith('event:')) event = line.slice(6).trim()
      if (line.startsWith('data:')) data.push(line.slice(5).trimStart())
    }
    if (!data.length) continue
    try {
      frames.push({ event, data: JSON.parse(data.join('\n')) })
    } catch (error) {
      throw new Error(`could not parse ${event} SSE frame as JSON: ${errorMessage(error)}\n${frame}`)
    }
  }
  return frames
}

function assertSuccessfulWebSocketEvents(events) {
  const finalEvent = [...events].reverse().find((event) => event.type === 'final')
  if (!finalEvent?.workflow_run) throw new Error(`WebSocket final event did not include workflow_run: ${JSON.stringify(events)}`)
  assertWorkflowRunCompleted(finalEvent.workflow_run, 'WebSocket transcript')
}

function assertSuccessfulMcpToolCall(callBody) {
  let payload
  try {
    payload = JSON.parse(callBody)
  } catch (error) {
    throw new Error(`MCP tools/call response was not JSON: ${errorMessage(error)}\n${callBody}`)
  }
  const result = payload?.result
  const structured = result?.structuredContent
  if (!structured) throw new Error(`MCP tools/call response missing structuredContent:\n${callBody}`)
  if (structured.status !== 'Completed' || result.isError) {
    throw new Error(`MCP tools/call workflow did not complete successfully:\n${callBody}`)
  }
  if (JSON.stringify(structured).includes('provider_failure')) {
    throw new Error(`MCP tools/call exposed provider failure:\n${callBody}`)
  }
}

function assertWorkflowRunCompleted(workflowRun, label) {
  if (workflowRun.status !== 'Completed') {
    throw new Error(`${label} workflow status was ${workflowRun.status}, expected Completed:\n${JSON.stringify(workflowRun, null, 2)}`)
  }
  const failures = workflowRun.failure_events ?? []
  if (failures.length > 0) {
    throw new Error(`${label} workflow had failure events:\n${JSON.stringify(failures, null, 2)}`)
  }
}

function parseHumanHttpViewerConfig(body) {
  const match = body.match(/window\.__arrobaPublicationViewerConfig\s*=\s*(\{.*?\});/s)
  if (!match) return {}
  return JSON.parse(match[1])
}

async function readSse(url, payload, options = {}) {
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(), options.timeoutMs ?? 300_000)
  const response = await fetch(url, {
    method: options.method ?? 'POST',
    headers: payload == null
      ? { accept: 'text/event-stream' }
      : { accept: 'text/event-stream', 'content-type': 'application/json' },
    ...(payload == null ? {} : { body: JSON.stringify(payload) }),
    signal: controller.signal,
  }).finally(() => clearTimeout(timeout))
  const body = await response.text()
  if (!response.ok) throw new Error(`SSE failed: ${response.status} ${body}`)
  return body
}

async function invokeWebSocket(url, payload) {
  const socket = new WebSocket(url)
  const events = []
  try {
    while (true) {
      const event = await readWebSocketEvent(socket).catch((error) => {
        throw new Error(`${error instanceof Error ? error.message : String(error)}; events=${JSON.stringify(events)}`)
      })
      events.push(event)
      if (event.type === 'ready') socket.send(JSON.stringify({ type: 'invoke', input: payload }))
      if (event.type === 'final' || event.type === 'error') break
    }
    return events
  } finally {
    socket.close()
  }
}

function readWebSocketEvent(socket) {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('timed out waiting for websocket event')), 120_000)
    socket.once('message', (data) => {
      clearTimeout(timeout)
      try {
        resolve(JSON.parse(data.toString()))
      } catch (error) {
        reject(error)
      }
    })
    socket.once('error', (error) => {
      clearTimeout(timeout)
      reject(error)
    })
  })
}

async function waitForDeploymentReady(profile, deploymentId) {
  const deadline = Date.now() + 180_000
  let last = null
  while (Date.now() < deadline) {
    last = await getPublicationDeployment(profile, deploymentId)
    if (last.status === 'ready') return last
    if (last.status === 'failed') throw new Error(`deployment failed: ${last.lastError ?? 'unknown error'}`)
    await delay(2_000)
  }
  throw new Error(`deployment did not become ready: ${JSON.stringify(last)}`)
}

async function waitForProviderRunReady(client, providerRunId) {
  const deadline = Date.now() + 120_000
  let last = null
  while (Date.now() < deadline) {
    const response = await client.send(getProviderRunRequest(providerRunId)).catch(() => null)
    last = response?.ProviderRun?.provider_run ?? response?.provider_run ?? response
    const state = String(last?.status ?? last?.state ?? '').toLowerCase()
    if (state === 'running' || state === 'ready') return
    if (state === 'failed' || state === 'exited' || state === 'stopped') {
      throw new Error(`provider run ${providerRunId} ended before ready: ${JSON.stringify(last)}`)
    }
    await delay(500)
  }
  throw new Error(`provider run ${providerRunId} did not become ready: ${JSON.stringify(last)}`)
}

async function buildRustBinary(binaryName) {
  const binaryPath = path.join(repoRoot, 'apps', binaryName.replace(/^arroba-/, ''), 'target', 'debug', binaryName)
  const exists = await readFile(binaryPath).then(() => true).catch(() => false)
  if (exists) return binaryPath
  const manifestPath = path.join(repoRoot, 'apps', binaryName.replace(/^arroba-/, ''), 'Cargo.toml')
  const result = await run('cargo', ['build', '--manifest-path', manifestPath, '--bin', binaryName])
  if (result.code !== 0) throw new Error(`cargo build ${binaryName} failed\n${result.stdout}\n${result.stderr}`)
  return binaryPath
}

function startProcess(command, args, env, name) {
  const logs = { stdout: '', stderr: '' }
  const child = spawn(command, args, { cwd: repoRoot, env, stdio: ['ignore', 'pipe', 'pipe'] })
  child.stdout.on('data', (chunk) => { logs.stdout += chunk.toString() })
  child.stderr.on('data', (chunk) => { logs.stderr += chunk.toString() })
  child.logs = logs
  child.name = name
  return child
}

async function stopProcess(child) {
  if (!child || child.killed) return
  if (child.exitCode !== null || child.signalCode !== null) return
  child.kill('SIGTERM')
  const result = await waitForProcessExit(child, 5_000).catch(async () => {
    child.kill('SIGKILL')
    return waitForProcessExit(child, 5_000)
  })
  return result
}

function waitForProcessExit(child, timeoutMs) {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error(`timed out waiting for ${child.name ?? child.pid}`)), timeoutMs)
    child.once('exit', (code, signal) => {
      clearTimeout(timeout)
      resolve({ code, signal })
    })
  })
}

async function waitForKernel(kernelUrl) {
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(getDaemonHealthRequest())
      await client.close?.()
      return
    } catch {
      await client.close?.().catch(() => {})
      await delay(250)
    }
  }
  throw new Error(`kernel did not become ready at ${kernelUrl}`)
}

async function waitForGateway(baseUrl) {
  const deadline = Date.now() + 60_000
  const statusUrl = `${baseUrl.replace(/\/+$/, '')}/.well-known/arroba/publication/status`
  while (Date.now() < deadline) {
    try {
      const response = await fetch(statusUrl)
      if (response.ok) return
    } catch {}
    await delay(250)
  }
  throw new Error(`gateway did not become ready at ${statusUrl}`)
}

async function waitForTcpPort(host, port) {
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    if (await canConnect(host, port)) return
    await delay(250)
  }
  throw new Error(`timed out waiting for ${host}:${port}`)
}

function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer()
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      const port = typeof address === 'object' && address ? address.port : null
      server.close(() => port ? resolve(port) : reject(new Error('could not allocate free port')))
    })
    server.once('error', reject)
  })
}

function canConnect(host, port) {
  return new Promise((resolve) => {
    const socket = net.connect({ host, port })
    socket.once('connect', () => {
      socket.end()
      resolve(true)
    })
    socket.once('error', () => resolve(false))
  })
}

function run(command, args, options = {}) {
  return new Promise((resolve) => {
    const child = spawn(command, args, { cwd: options.cwd ?? repoRoot, env: options.env ?? process.env, stdio: ['ignore', 'pipe', 'pipe'] })
    const stdout = []
    const stderr = []
    child.stdout.on('data', (chunk) => stdout.push(Buffer.from(chunk)))
    child.stderr.on('data', (chunk) => stderr.push(Buffer.from(chunk)))
    child.on('close', (code) => resolve({
      code,
      stdout: Buffer.concat(stdout).toString('utf8'),
      stderr: Buffer.concat(stderr).toString('utf8'),
    }))
  })
}

async function copyCloudProfile(configHome) {
  const source = path.join(process.env.HOME, '.arroba', 'config.json')
  const target = path.join(configHome, 'arroba', 'config.json')
  await cp(source, target)
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error)
}

main().catch((error) => {
  console.error(`[cloud-publication-drill] failed: ${error.stack ?? error.message}`)
  process.exit(1)
})
