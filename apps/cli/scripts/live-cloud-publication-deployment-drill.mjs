#!/usr/bin/env node
import { spawn } from 'node:child_process'
import net from 'node:net'
import path from 'node:path'
import { chmod, cp, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
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
  createWorkflowWatchdogRequest,
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
  exportWorkflowPublicationPackageRequest,
} = requests

const REAL_DASHBOARD_PROMPT = [
  'Generate a vibrant dashboard as a compact self-contained HTML document.',
  'The dashboard must visibly include the title text `Real Provider Workflow Dashboard`.',
  'The main dashboard element must include `data-arroba-real-provider-dashboard="true"`.',
  'Before writing the file, reason through a compact layout plan that balances mobile responsiveness, contrast, KPI cards, one chart-like visual, and a status section.',
  'Use inline CSS only; no scripts, external assets, shell commands, file inspection, or network calls.',
  'Keep the HTML under 1800 characters so it fits comfortably in one runtime tool call.',
  'This is the final workflow node: call the Arroba runtime MCP tool validate_and_submit_workflow_run_output directly with workflow_output_json set to {"kind":"html","html":"<full html document>"}.',
  'Do not use bash, python, node, or other provider tools to construct or submit this payload. After the runtime tool succeeds, emit the final fenced workflow JSON block with the same output.message object.',
  'If the Arroba runtime MCP tool is unavailable, do not search for it: emit the final fenced workflow JSON block directly with output.message set to {"kind":"html","html":"<full html document>"}.',
].join(' ')

const SHOPPING_LIST_PROMPT = '1 kg bananas, 2 bottles of 1l Coca-Cola, and a bag of chips'
const SHOPPING_LIST_PROMPT_B = '2 red apples, 1 bag of coffee beans, and 3 packs of pasta'
const SHOPPING_EXPECTED_SNIPPETS = ['Agent App Grocery Checkout', 'data-arroba-agent-app-checkout', 'bananas', 'Coca-Cola', 'chips']
const SHOPPING_EXPECTED_SNIPPETS_B = ['Agent App Grocery Checkout', 'data-arroba-agent-app-checkout', 'apples', 'coffee', 'pasta']
const AGENT_APP_SHOPPING_PROMPT = [
  'You are the checkout automation agent for a published Arroba Agent App.',
  'The invocation prompt is a grocery shopping list from the browser URL.',
  'First call the Arroba runtime MCP tool arroba.agent_app_action for action_id "cart.add" once per product with input containing sku, name, quantity, and unit.',
  'Then call arroba.agent_app_action with action_id "cart.checkout" and input {"ready":true}.',
  'Use the action responses to build a compact checkout page.',
  'The final workflow output must be JSON stringified through validate_and_submit_workflow_run_output with this exact outer shape: {"kind":"response","response":{"mode":"serve","entry":"/generated/checkout.html"},"effects":{"overlay":[{"path":"/generated/checkout.html","mime_type":"text/html; charset=utf-8","content":"<full html document>"}]}}.',
  'The generated HTML must include the visible title text `Agent App Grocery Checkout`, the attribute data-arroba-agent-app-checkout="true", and the products requested by the invocation prompt.',
  'Use inline CSS only. Do not use shell, bash, python, node, filesystem, or network tools.',
  'If the Arroba runtime MCP tools are unavailable, emit the final fenced workflow JSON block directly with the same output.message object, but still include the checkout HTML.',
].join(' ')

function usage() {
  return [
    'Usage: node apps/cli/scripts/live-cloud-publication-deployment-drill.mjs --mode hosted-container|local-runtime [options]',
    '',
    'Options:',
    '  --transport human_http|api_sse_json|websocket_json|mcp|watchdog',
    '  --slug SLUG',
    '  --provider dev-stub|codex|opencode|claude',
    '  --model MODEL',
    '  --credential-profile NAME',
    '  --real-dashboard',
    '  --agent-app-shopping',
    '  --agent-app-session-isolation',
    '  --artifacts-dir DIR',
    '  --browser-screenshot',
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
    agentAppShopping: false,
    agentAppSessionIsolation: false,
    artifactsDir: path.join(repoRoot, '.artifacts', 'cloud-publication-deployments'),
    browserScreenshot: false,
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
    else if (arg === '--agent-app-shopping') options.agentAppShopping = true
    else if (arg === '--agent-app-session-isolation') options.agentAppSessionIsolation = true
    else if (arg === '--artifacts-dir') options.artifactsDir = path.resolve(argv[++index])
    else if (arg === '--browser-screenshot') options.browserScreenshot = true
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
  if (options.agentAppSessionIsolation) options.agentAppShopping = true
  if (options.agentAppShopping && options.transport !== 'human_http') {
    throw new Error('--agent-app-shopping currently requires --transport human_http')
  }
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
  if (['human_http', 'api_sse_json', 'websocket_json', 'mcp', 'watchdog'].includes(value)) return value
  throw new Error('--transport must be human_http, api_sse_json, websocket_json, mcp, or watchdog')
}

function defaultModel(provider, transport, realDashboard) {
  if (provider === 'dev-stub') {
    if (realDashboard) return 'workflow-html-final-node'
    if (transport === 'mcp' || transport === 'watchdog') return 'workflow-single-turn-node'
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
  const actionPort = options.mode === 'local_runtime' ? await freePort() : 33119
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
  let actionServer = null
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
      agentAppShopping: options.agentAppShopping,
      actionPort,
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
      if (options.agentAppShopping) {
        actionServer = startProcess(process.execPath, [path.join(packageDir, 'app', 'actions.mjs')], {
          ...env,
          PORT: String(actionPort),
        }, 'agent-app-actions')
        await waitForTcpPort('127.0.0.1', actionPort).catch((error) => {
          throw new Error(`${errorMessage(error)}\naction stdout:\n${actionServer?.logs?.stdout ?? ''}\naction stderr:\n${actionServer?.logs?.stderr ?? ''}`)
        })
      }
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
    readyDeployment = await waitForDeploymentReady(profile, deployment.id).catch((error) => {
      throw new Error(`${errorMessage(error)}\nkernel stdout:\n${kernel?.logs?.stdout ?? ''}\nkernel stderr:\n${kernel?.logs?.stderr ?? ''}\nserve stdout:\n${serve?.logs?.stdout ?? ''}\nserve stderr:\n${serve?.logs?.stderr ?? ''}`)
    })
    logStep('deployment_ready', {
      id: readyDeployment.id,
      status: readyDeployment.status,
      publicBaseUrl: readyDeployment.publicBaseUrl,
    })
    const evidence = await validateTransport({
      transport: options.transport,
      publicBaseUrl: readyDeployment.publicBaseUrl,
      prompt: options.agentAppShopping
        ? SHOPPING_LIST_PROMPT
        : options.realDashboard
          ? REAL_DASHBOARD_PROMPT
          : `cloud deployment drill ${options.transport}`,
      artifactsDir: options.artifactsDir,
      slug: options.slug,
      expectHtmlDashboard: options.realDashboard,
      expectAgentAppShopping: options.agentAppShopping,
      agentAppSessionIsolation: options.agentAppSessionIsolation,
      browserScreenshot: options.browserScreenshot,
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
    await stopProcess(actionServer).catch((error) => logStep('cleanup_warning', { process: 'actionServer', error: errorMessage(error) }))
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
    agentAppShopping,
    actionPort,
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
    agentAppShopping
      ? AGENT_APP_SHOPPING_PROMPT
      : realDashboard
      ? REAL_DASHBOARD_PROMPT
      : 'Complete this cloud publication deployment drill with a concise deterministic final output.',
  ))
  await client.send(setWorkflowNodeCanCompleteRunRequest(session.id, workflow.id, node.id, true))
  await client.send(setWorkflowNodeCanEmitIntermediateOutputRequest(session.id, workflow.id, node.id, true))
  const endpointKind = transport === 'watchdog' ? 'watchdog' : transport
  const endpoint = (await client.send(createWorkflowEndpointRequest(session.id, workflow.id, node.id, endpointKind))).WorkflowEndpointCreated.endpoint
  const publication = (await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, publicationOptions(transport, node.id)))).WorkflowPublicationCreated.publication
  let watchdog = null
  if (transport === 'watchdog') {
    watchdog = (await client.send(createWorkflowWatchdogRequest(
      session.id,
      workflow.id,
      endpoint.id,
      60,
      realDashboard ? REAL_DASHBOARD_PROMPT : 'cloud deployment watchdog drill',
      'queue',
      1,
    ))).WorkflowWatchdogCreated.watchdog
  }
  if (agentAppShopping) {
    const assetsDir = await writeShoppingAgentAppAssets(workspace)
    const exported = await client.send(exportWorkflowPublicationPackageRequest(session.id, publication.id, {
      kernelUrl,
      agentApp: shoppingAgentAppConfig(publication.id, actionPort),
      agentAppAssetsDir: assetsDir,
    }))
    const payload = exported.WorkflowPublicationPackageExported
    if (!payload?.package_files) {
      throw new Error(`agent app publication export failed: ${JSON.stringify(exported)}`)
    }
    await writePackageFiles(exportDir, payload.package_files)
  } else {
    const exportResult = await executeShellCommand(
      parseShellCommand(`workflow publication export ${publication.id} ${exportDir} --kernel-url ${kernelUrl}`),
      createDefaultShellContext({ workspace, worktree: workspace, sessionId: session.id, workflowId: workflow.id }),
      { client },
    )
    if (!exportResult.ok) throw new Error(`publication export failed: ${exportResult.message}`)
  }
  if (watchdog) {
    const snapshotPath = path.join(exportDir, 'workflow.snapshot.json')
    const snapshot = JSON.parse(await readFile(snapshotPath, 'utf8'))
    if (snapshot.watchdogs?.[0]?.id !== watchdog.id) {
      throw new Error(`expected exported watchdog ${watchdog.id}, got ${JSON.stringify(snapshot.watchdogs)}`)
    }
    snapshot.watchdogs[0].next_run_at_ms = 0
    await writeFile(snapshotPath, `${JSON.stringify(snapshot, null, 2)}\n`)
  }
  await makePortablePackage(exportDir, packageDir)
  return {
    sessionId: session.id,
    workflowId: workflow.id,
    endpointId: endpoint.id,
    publicationId: publication.id,
    watchdogId: watchdog?.id ?? null,
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
    mode: transport === 'human_http' || transport === 'watchdog' ? 'async' : 'sync',
  }
  if (transport === 'human_http') return { ...base, transport: { kind: 'human_http' } }
  if (transport === 'api_sse_json') return { ...base, transport: { kind: 'api_sse_json' }, parser: { kind: 'json' } }
  if (transport === 'websocket_json') return { ...base, transport: { kind: 'websocket_json' }, parser: { kind: 'json' } }
  if (transport === 'mcp') return { ...base, transport: { kind: 'mcp' }, parser: { kind: 'json' } }
  if (transport === 'watchdog') return { ...base, route: '/watchdog', methods: ['POST'], transport: { kind: 'api_sse_json' }, parser: { kind: 'json' } }
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

async function writePackageFiles(targetDir, packageFiles) {
  await rm(targetDir, { recursive: true, force: true })
  for (const file of packageFiles) {
    const target = path.join(targetDir, file.path)
    if (!target.startsWith(`${path.resolve(targetDir)}${path.sep}`) && path.resolve(target) !== path.resolve(targetDir)) {
      throw new Error(`exported package file escapes target directory: ${file.path}`)
    }
    await mkdir(path.dirname(target), { recursive: true })
    await writeFile(target, Buffer.from(file.content_base64, 'base64'))
    if (file.executable) await chmod(target, 0o755)
  }
}

async function writeShoppingAgentAppAssets(workspace) {
  const assetsDir = path.join(workspace, 'shopping-agent-app')
  await rm(assetsDir, { recursive: true, force: true })
  await mkdir(path.join(assetsDir, 'assets'), { recursive: true })
  await writeFile(path.join(assetsDir, 'index.html'), [
    '<!doctype html>',
    '<html><head><meta charset="utf-8"><title>Agent Grocery</title>',
    '<style>body{font-family:Inter,system-ui,sans-serif;margin:0;background:#f7f7f2;color:#1d2433}.shell{max-width:960px;margin:0 auto;padding:32px}.grid{display:grid;grid-template-columns:repeat(3,1fr);gap:12px}.card{background:white;border:1px solid #ddd;border-radius:8px;padding:16px}</style>',
    '</head><body><main class="shell"><h1>Agent Grocery</h1><p>Use /add/&lt;shopping list&gt; to let the agent prepare checkout.</p><section class="grid"><div class="card">Bananas</div><div class="card">Coca-Cola 1l</div><div class="card">Chips</div></section></main></body></html>',
  ].join(''))
  await writeFile(path.join(assetsDir, 'assets', 'catalog.json'), JSON.stringify({
    products: [
      { sku: 'banana-kg', name: 'Bananas', unit: 'kg' },
      { sku: 'coca-cola-1l', name: 'Coca-Cola 1l bottle', unit: 'bottle' },
      { sku: 'chips-bag', name: 'Chips', unit: 'bag' },
    ],
  }, null, 2))
  await writeFile(path.join(assetsDir, 'actions.mjs'), [
    'import http from "node:http";',
    'const carts = new Map();',
    'function readBody(req){return new Promise((resolve,reject)=>{let data="";req.on("data",c=>data+=c);req.on("end",()=>{try{resolve(data?JSON.parse(data):{})}catch(e){reject(e)}});req.on("error",reject);});}',
    'function send(res,status,body){res.writeHead(status,{"content-type":"application/json"});res.end(JSON.stringify(body));}',
    'function sessionCart(req){const key=String(req.headers["x-arroba-agent-app-session"]||"anonymous");if(!carts.has(key))carts.set(key,[]);return {key,cart:carts.get(key)};}',
    'http.createServer(async (req,res)=>{',
    '  try {',
    '    if (req.url === "/health") return send(res,200,{ok:true});',
    '    if (req.method === "POST" && req.url === "/cart/add") { const body = await readBody(req); const scoped=sessionCart(req); scoped.cart.push(body); return send(res,200,{ok:true,session:scoped.key,item:body,cart:scoped.cart}); }',
    '    if (req.method === "POST" && req.url === "/cart/checkout") { const scoped=sessionCart(req); return send(res,200,{ok:true,session:scoped.key,ready:true,cart:scoped.cart,total_items:scoped.cart.length}); }',
    '    return send(res,404,{error:"not found"});',
    '  } catch (error) { return send(res,500,{error:String(error?.message ?? error)}); }',
    '}).listen(Number(process.env.PORT || 33119), "127.0.0.1");',
  ].join('\n'))
  return assetsDir
}

function shoppingAgentAppConfig(publicationId, actionPort) {
  const actionBase = `http://127.0.0.1:${actionPort}`
  return {
    enabled: true,
    assets: { public_dir: 'app', index: 'index.html' },
    routes: [{
      path: '/add/*',
      hook_id: `${publicationId}-agent-app-hook`,
      prompt_source: 'path_tail',
      response: 'streaming_shell',
      required_role: 'public',
      manipulation: {
        level: 'state_and_overlay',
        scope: 'session',
        allowed_paths: ['/generated/**'],
        protected_paths: ['/auth/**', '/payments/**', '/secrets/**'],
        allowed_actions: ['cart.add', 'cart.checkout'],
      },
    }],
    actions: {
      'cart.add': {
        input_schema: {
          type: 'object',
          required: ['sku', 'name', 'quantity'],
          properties: {
            sku: { type: 'string' },
            name: { type: 'string' },
            quantity: { type: 'number' },
            unit: { type: 'string' },
          },
        },
        transport: { kind: 'http', method: 'POST', url: `${actionBase}/cart/add` },
      },
      'cart.checkout': {
        input_schema: {
          type: 'object',
          properties: { ready: { type: 'boolean' } },
        },
        transport: { kind: 'http', method: 'POST', url: `${actionBase}/cart/checkout` },
      },
    },
    replicas: { count: 1, per_caller_ordering: true, max_queue_depth: 100 },
    persistent_patch: { enabled: false },
  }
}

async function validateTransport(input) {
  const base = input.publicBaseUrl.replace(/\/+$/, '')
  if (input.transport === 'human_http') {
    const promptUrl = input.expectAgentAppShopping
      ? `${base}/add/${encodeURIComponent(input.prompt)}`
      : `${base}/final/${encodeURIComponent(input.prompt)}`
    if (input.browserScreenshot) {
      if (input.expectAgentAppShopping) {
        if (input.agentAppSessionIsolation) {
          return await runAgentAppShoppingSessionIsolation({
            baseUrl: base,
            prompt: input.prompt,
            secondPrompt: SHOPPING_LIST_PROMPT_B,
            artifactsDir: input.artifactsDir,
            slug: input.slug,
          })
        }
        return await runAgentAppShoppingBrowserScreenshot({
          url: promptUrl,
          artifactsDir: input.artifactsDir,
          slug: input.slug,
          expectedSnippets: SHOPPING_EXPECTED_SNIPPETS,
        })
      }
      return await runHumanHttpDashboardBrowserScreenshot({
        url: promptUrl,
        artifactsDir: input.artifactsDir,
        slug: input.slug,
      })
    }
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
  if (input.transport === 'watchdog') {
    const status = await waitForWatchdogPublicationStatus(base, {
      expectHtmlDashboard: input.expectHtmlDashboard,
    })
    const statusPath = path.join(input.artifactsDir, `${input.slug}-watchdog-status.json`)
    await writeFile(statusPath, `${JSON.stringify(status, null, 2)}\n`)
    return { statusPath, latestOutput: status.latest_output?.message ?? null }
  }
  throw new Error(`unsupported transport ${input.transport}`)
}

async function waitForWatchdogPublicationStatus(base, options = {}) {
  const statusUrl = `${base}/.well-known/arroba/publication/status`
  const deadline = Date.now() + 900_000
  let last = null
  while (Date.now() < deadline) {
    const response = await fetch(statusUrl, { headers: { accept: 'application/json' } })
    const body = await response.text()
    if (!response.ok) throw new Error(`watchdog status failed: ${response.status} ${body}`)
    last = JSON.parse(body)
    if (last.watchdog_count !== 1 || !Array.isArray(last.watchdogs) || last.watchdogs.length !== 1) {
      throw new Error(`watchdog status did not expose exactly one watchdog: ${body}`)
    }
    const latest = last.latest_output?.message
    const watchdog = last.watchdogs[0]
    const status = String(watchdog.last_status ?? '').toLowerCase()
    if (latest && ['started', 'completed_budget'].includes(status)) {
      if (options.expectHtmlDashboard) {
        const serialized = JSON.stringify(last)
        for (const snippet of ['Real Provider Workflow Dashboard', 'data-arroba-real-provider-dashboard']) {
          if (!serialized.includes(snippet)) throw new Error(`watchdog latest output missing dashboard snippet ${snippet}:\n${serialized}`)
        }
      }
      return last
    }
    if (watchdog.last_error) {
      throw new Error(`watchdog failed: ${watchdog.last_error}\n${body}`)
    }
    await delay(2_000)
  }
  throw new Error(`watchdog publication did not produce latest output: ${JSON.stringify(last, null, 2)}`)
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
  return await new Promise((resolve, reject) => {
    let invoked = false
    const timeout = setTimeout(() => {
      socket.close()
      reject(new Error(`timed out waiting for websocket event; events=${JSON.stringify(events)}`))
    }, 300_000)
    socket.on('message', (data) => {
      try {
        const event = JSON.parse(data.toString())
        events.push(event)
        if (event.type === 'ready' && !invoked) {
          invoked = true
          socket.send(JSON.stringify({ type: 'invoke', input: payload }))
        }
        if (event.type === 'final') {
          clearTimeout(timeout)
          socket.close()
          resolve(events)
        }
        if (event.type === 'error') {
          clearTimeout(timeout)
          socket.close()
          reject(new Error(`websocket error: ${event.error ?? 'unknown'}; events=${JSON.stringify(events)}`))
        }
      } catch (error) {
        clearTimeout(timeout)
        socket.close()
        reject(error)
      }
    })
    socket.on('error', (error) => {
      clearTimeout(timeout)
      reject(error)
    })
    socket.on('close', () => {
      if (!events.some((event) => event.type === 'final')) {
        clearTimeout(timeout)
        reject(new Error(`websocket closed before final; events=${JSON.stringify(events)}`))
      }
    })
  })
}

async function runHumanHttpDashboardBrowserScreenshot({ url, artifactsDir, slug }) {
  const chromePath = await findChromeExecutable()
  if (!chromePath) throw new Error('Chrome executable was not found for browser screenshot validation')
  const debuggingPort = await freePort()
  const userDataDir = path.join(artifactsDir, `${slug}-chrome-profile`)
  const screenshotPath = path.join(artifactsDir, `${slug}-browser-final-dashboard.png`)
  await rm(userDataDir, { recursive: true, force: true })
  await mkdir(userDataDir, { recursive: true })
  const chrome = startProcess(
    chromePath,
    [
      '--headless=new',
      '--disable-gpu',
      '--disable-background-networking',
      '--disable-extensions',
      '--window-size=1440,1000',
      '--no-first-run',
      '--no-default-browser-check',
      '--no-sandbox',
      '--remote-debugging-address=127.0.0.1',
      `--remote-debugging-port=${debuggingPort}`,
      `--user-data-dir=${userDataDir}`,
      'about:blank',
    ],
    process.env,
    'chrome-cloud-publication-dashboard',
  )
  let cdp = null
  try {
    const target = await waitForChromeTarget(debuggingPort, chrome)
    cdp = await connectChromeTarget(target.webSocketDebuggerUrl)
    await cdp.send('Page.enable')
    await cdp.send('Runtime.enable')
    await withTimeout(cdp.send('Page.navigate', { url }), 10_000, `browser navigate ${url}`)
    const finalState = await waitForBrowserDashboardFinal(cdp, 420_000)
    const screenshot = await withTimeout(
      cdp.send('Page.captureScreenshot', { format: 'png', fromSurface: true }),
      10_000,
      'browser dashboard screenshot',
    )
    if (typeof screenshot.data !== 'string' || screenshot.data.length < 1000) {
      throw new Error('browser dashboard screenshot was empty')
    }
    await writeFile(screenshotPath, Buffer.from(screenshot.data, 'base64'))
    return { promptUrl: url, screenshotPath, finalState }
  } catch (error) {
    throw new Error(`${errorMessage(error)}\nchrome stdout:\n${chrome.logs.stdout}\nchrome stderr:\n${chrome.logs.stderr}`)
  } finally {
    await cdp?.close?.().catch(() => {})
    await stopProcess(chrome)
    await rm(userDataDir, { recursive: true, force: true }).catch(() => {})
  }
}

async function runAgentAppShoppingSessionIsolation({ baseUrl, prompt, secondPrompt, artifactsDir, slug }) {
  const firstUrl = `${baseUrl}/add/${encodeURIComponent(prompt)}`
  const secondUrl = `${baseUrl}/add/${encodeURIComponent(secondPrompt)}`
  const first = await runAgentAppShoppingBrowserScreenshot({
    url: firstUrl,
    artifactsDir,
    slug,
    expectedSnippets: SHOPPING_EXPECTED_SNIPPETS,
    screenshotName: 'agent-app-shopping-checkout-session-a',
  })
  const second = await runAgentAppShoppingBrowserScreenshot({
    url: secondUrl,
    artifactsDir,
    slug,
    expectedSnippets: SHOPPING_EXPECTED_SNIPPETS_B,
    screenshotName: 'agent-app-shopping-checkout-session-b',
  })
  if (!first.cookieHeader) throw new Error('first Agent App session did not expose a browser session cookie')
  if (!first.finalState?.iframeSrc) throw new Error('first Agent App session did not expose a generated checkout iframe URL')
  const firstCheckoutUrl = new URL(first.finalState.iframeSrc, firstUrl)
  const revisited = await fetch(firstCheckoutUrl, {
    headers: { cookie: first.cookieHeader },
  })
  const revisitedHtml = await revisited.text()
  const lowerRevisitedHtml = revisitedHtml.toLowerCase()
  const firstMissing = SHOPPING_EXPECTED_SNIPPETS.filter((snippet) => !lowerRevisitedHtml.includes(snippet.toLowerCase()))
  const leakedSecond = SHOPPING_EXPECTED_SNIPPETS_B.filter((snippet) => (
    !SHOPPING_EXPECTED_SNIPPETS.some((firstSnippet) => firstSnippet.toLowerCase() === snippet.toLowerCase())
    && lowerRevisitedHtml.includes(snippet.toLowerCase())
  ))
  if (!revisited.ok || firstMissing.length || leakedSecond.length) {
    throw new Error(`Agent App session isolation failed: ${JSON.stringify({
      status: revisited.status,
      firstMissing,
      leakedSecond,
      firstCheckoutUrl: firstCheckoutUrl.toString(),
    })}`)
  }
  return {
    promptUrl: firstUrl,
    secondPromptUrl: secondUrl,
    screenshotPath: first.screenshotPath,
    secondScreenshotPath: second.screenshotPath,
    finalState: first.finalState,
    secondFinalState: second.finalState,
    isolation: { firstCheckoutUrl: firstCheckoutUrl.toString(), status: revisited.status },
  }
}

async function runAgentAppShoppingBrowserScreenshot({
  url,
  artifactsDir,
  slug,
  expectedSnippets = SHOPPING_EXPECTED_SNIPPETS,
  screenshotName = 'agent-app-shopping-checkout',
}) {
  const chromePath = await findChromeExecutable()
  if (!chromePath) throw new Error('Chrome executable was not found for Agent App browser screenshot validation')
  const debuggingPort = await freePort()
  const userDataDir = path.join(artifactsDir, `${slug}-agent-app-chrome-profile`)
  const screenshotPath = path.join(artifactsDir, `${slug}-${screenshotName}.png`)
  await rm(userDataDir, { recursive: true, force: true })
  await mkdir(userDataDir, { recursive: true })
  const chrome = startProcess(
    chromePath,
    [
      '--headless=new',
      '--disable-gpu',
      '--disable-background-networking',
      '--disable-extensions',
      '--window-size=1440,1000',
      '--no-first-run',
      '--no-default-browser-check',
      '--no-sandbox',
      '--remote-debugging-address=127.0.0.1',
      `--remote-debugging-port=${debuggingPort}`,
      `--user-data-dir=${userDataDir}`,
      'about:blank',
    ],
    process.env,
    'chrome-cloud-agent-app-shopping',
  )
  let cdp = null
  try {
    const target = await waitForChromeTarget(debuggingPort, chrome)
    cdp = await connectChromeTarget(target.webSocketDebuggerUrl)
    await cdp.send('Page.enable')
    await cdp.send('Runtime.enable')
    await cdp.send('Network.enable')
    await withTimeout(cdp.send('Page.navigate', { url }), 10_000, `browser navigate ${url}`)
    const finalState = await waitForAgentAppShoppingFinal(cdp, 600_000, expectedSnippets)
    const screenshot = await withTimeout(
      cdp.send('Page.captureScreenshot', { format: 'png', fromSurface: true }),
      10_000,
      'Agent App shopping screenshot',
    )
    if (typeof screenshot.data !== 'string' || screenshot.data.length < 1000) {
      throw new Error('Agent App shopping screenshot was empty')
    }
    await writeFile(screenshotPath, Buffer.from(screenshot.data, 'base64'))
    const cookieHeader = await agentAppCookieHeader(cdp)
    return { promptUrl: url, screenshotPath, finalState, cookieHeader }
  } catch (error) {
    throw new Error(`${errorMessage(error)}\nchrome stdout:\n${chrome.logs.stdout}\nchrome stderr:\n${chrome.logs.stderr}`)
  } finally {
    await cdp?.close?.().catch(() => {})
    await stopProcess(chrome)
    await rm(userDataDir, { recursive: true, force: true }).catch(() => {})
  }
}

async function waitForAgentAppShoppingFinal(cdp, timeoutMs, expectedSnippets) {
  const deadline = Date.now() + timeoutMs
  let lastState = null
  const requiredSnippetsJson = JSON.stringify(expectedSnippets)
  while (Date.now() < deadline) {
    const evaluated = await withTimeout(cdp.send('Runtime.evaluate', {
      returnByValue: true,
      awaitPromise: true,
      expression: `(async () => {
        const required = ${requiredSnippetsJson};
        const status = document.querySelector('#status')?.textContent?.trim() || '';
        const iframe = document.querySelector('#html-output iframe');
        const iframeSrcdoc = iframe?.getAttribute('srcdoc') || '';
        const iframeSrc = iframe?.getAttribute('src') || '';
        let fetchedHtml = '';
        let fetchStatus = null;
        if (!iframeSrcdoc && iframeSrc) {
          try {
            const response = await fetch(iframeSrc, { cache: 'no-store' });
            fetchStatus = response.status;
            fetchedHtml = await response.text();
          } catch (error) {
            fetchedHtml = String(error?.message || error);
          }
        }
        const iframeDocumentHtml = iframe?.contentDocument?.documentElement?.outerHTML || '';
        const renderedHtml = iframeSrcdoc || fetchedHtml || iframeDocumentHtml;
        const traceText = Array.from(document.querySelectorAll('#trace-feed .trace-item')).map((item) => item.textContent || '').join('\\n');
        const lowerRenderedHtml = renderedHtml.toLowerCase();
        const missing = required.filter((snippet) => !lowerRenderedHtml.includes(String(snippet).toLowerCase()));
        return {
          status,
          missing,
          iframeSrc,
          fetchStatus,
          renderedLength: renderedHtml.length,
          traceText,
          traceCount: document.querySelectorAll('#trace-feed .trace-item').length,
          actionTraceOk: traceText.includes('agent_app_action') || traceText.includes('arroba.agent_app_action') || traceText.includes('cart.add'),
          ok: status === 'Completed' && missing.length === 0 && (traceText.includes('cart.add') || traceText.includes('agent_app_action')),
        };
      })()`,
    }), 15_000, 'Agent App shopping Runtime.evaluate')
    lastState = evaluated.result?.value ?? null
    if (lastState?.ok) return lastState
    if (
      lastState?.status === 'Completed'
      && Number(lastState?.renderedLength ?? 0) > 0
      && Array.isArray(lastState?.missing)
      && lastState.missing.length > 0
    ) {
      throw new Error(`Agent App shopping completed without checkout snippets: ${JSON.stringify(lastState)}`)
    }
    await delay(750)
  }
  throw new Error(`Agent App shopping browser did not render final checkout: ${JSON.stringify(lastState)}`)
}

async function agentAppCookieHeader(cdp) {
  const response = await cdp.send('Network.getAllCookies')
  const cookies = Array.isArray(response?.cookies) ? response.cookies : []
  const sessionCookie = cookies.find((cookie) => cookie?.name === 'arroba_agent_app_session')
  if (!sessionCookie?.value) return null
  return `${sessionCookie.name}=${encodeURIComponent(sessionCookie.value)}`
}

async function findChromeExecutable() {
  const candidates = [
    process.env.ARROBA_CHROME_PATH,
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    'google-chrome',
    'chromium',
    'chromium-browser',
  ].filter(Boolean)
  for (const candidate of candidates) {
    const result = await run(candidate, ['--version'])
    if (result.code === 0) return candidate
  }
  return null
}

async function waitForChromeTarget(debuggingPort, chrome) {
  const endpoint = `http://127.0.0.1:${debuggingPort}/json/list`
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const response = await fetch(endpoint)
      const targets = await response.json()
      const target = targets.find((candidate) => candidate.type === 'page' && candidate.webSocketDebuggerUrl)
      if (target?.webSocketDebuggerUrl) return target
    } catch (error) {
      lastError = error
    }
    await delay(250)
  }
  throw new Error(`Chrome DevTools target did not become ready: ${lastError?.message ?? 'no page target'}\n${chrome.logs.stderr.slice(-2_000)}`)
}

async function connectChromeTarget(webSocketUrl) {
  const socket = new WebSocket(webSocketUrl)
  let nextId = 1
  const pending = new Map()
  await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('timed out opening Chrome DevTools socket')), 10_000)
    socket.once('open', () => {
      clearTimeout(timeout)
      resolve()
    })
    socket.once('error', reject)
  })
  socket.on('message', (data) => {
    const message = JSON.parse(data.toString())
    if (typeof message.id !== 'number') return
    const waiter = pending.get(message.id)
    if (!waiter) return
    pending.delete(message.id)
    if (message.error) waiter.reject(new Error(`${message.error.message}: ${message.error.data ?? ''}`))
    else waiter.resolve(message.result ?? {})
  })
  socket.on('error', (error) => {
    for (const waiter of pending.values()) waiter.reject(error)
    pending.clear()
  })
  return {
    send(method, params = {}) {
      const id = nextId++
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject })
        socket.send(JSON.stringify({ id, method, params }))
      })
    },
    close() {
      return new Promise((resolve) => {
        if (socket.readyState === WebSocket.CLOSED) return resolve()
        socket.once('close', resolve)
        socket.close()
      })
    },
  }
}

async function waitForBrowserDashboardFinal(cdp, timeoutMs) {
  const requiredTraceLevels = ['output_summary', 'assistant_messages', 'thinking', 'tool_use']
  const requiredHtmlSnippets = ['Real Provider Workflow Dashboard', 'data-arroba-real-provider-dashboard']
  const deadline = Date.now() + timeoutMs
  let lastState = null
  while (Date.now() < deadline) {
    const evaluated = await withTimeout(cdp.send('Runtime.evaluate', {
      returnByValue: true,
      expression: `(() => {
        const status = document.querySelector('#status')?.textContent?.trim() || '';
        const iframe = document.querySelector('#html-output iframe');
        const iframeSrcdoc = iframe?.getAttribute('srcdoc') || '';
        const traces = Array.from(document.querySelectorAll('#trace-feed .trace-item')).map((item) => ({
          text: item.textContent || '',
          meta: Array.from(item.querySelectorAll('.trace-meta span')).map((span) => (span.textContent || '').trim()),
        }));
        const requiredTraceLevels = ${JSON.stringify(requiredTraceLevels)};
        const traceLevels = Array.from(new Set(traces.flatMap((trace) => trace.meta).filter((value) => requiredTraceLevels.includes(value)))).sort();
        const traceAliases = Array.from(new Set(traces.map((trace) => trace.meta[0]).filter(Boolean))).sort();
        const missingTraceLevels = requiredTraceLevels.filter((level) => !traceLevels.includes(level));
        const htmlOk = ${JSON.stringify(requiredHtmlSnippets)}.every((snippet) => iframeSrcdoc.includes(snippet));
        return {
          status,
          htmlOk,
          traceCount: traces.length,
          traceLevels,
          traceAliases,
          missingTraceLevels,
          ok: status === 'Completed' && htmlOk && traces.length > 0 && missingTraceLevels.length === 0,
        };
      })()`,
    }), 15_000, 'browser dashboard Runtime.evaluate')
    lastState = evaluated.result?.value ?? null
    if (lastState?.ok) return lastState
    if (
      lastState?.status === 'Completed'
      && lastState?.htmlOk
      && Array.isArray(lastState?.missingTraceLevels)
      && lastState.missingTraceLevels.length > 0
    ) {
      throw new Error(`browser completed without required trace levels: ${JSON.stringify(lastState)}`)
    }
    await delay(500)
  }
  throw new Error(`browser did not render final dashboard: ${JSON.stringify(lastState)}`)
}

async function withTimeout(promise, timeoutMs, label) {
  let timer = null
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} timed out after ${timeoutMs}ms`)), timeoutMs)
      }),
    ])
  } finally {
    if (timer) clearTimeout(timer)
  }
}

async function waitForDeploymentReady(profile, deploymentId) {
  const deadline = Date.now() + 420_000
  let last = null
  while (Date.now() < deadline) {
    last = await getPublicationDeployment(profile, deploymentId)
    if (last.status === 'ready') return last
    if (last.status === 'failed' || last.status === 'unavailable') {
      throw new Error(`deployment ${last.status}: ${last.lastError ?? 'unknown error'}`)
    }
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
