#!/usr/bin/env node
import path from 'node:path'
import { chmod, cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'
import { withDevStubProviderInventory } from './lib/drill-runtime-helpers.mjs'
import {
  assertSuccessfulSseTranscript,
  assertWorkflowRunCompleted,
  buildRustBinary,
  copyCloudProfile,
  delay,
  errorMessage,
  freePort,
  parseHumanHttpViewerConfig,
  readSse,
  run,
  runAgentAppShoppingBrowserScreenshot,
  runAgentAppShoppingSessionIsolation,
  runHumanHttpDashboardBrowserScreenshot,
  startProcess,
  stopProcess,
  validateTransport,
  waitForDeploymentActionAuditLogs,
  waitForDeploymentReady,
  waitForGateway,
  waitForKernel,
  waitForProviderRunReady,
  waitForSchedulePublicationStatus,
  waitForTcpPort,
} from './lib/live-cloud-publication-deployment-drill-transport.mjs'

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
  listPublicationDeploymentLogs,
} = await import('../dist/publication-deployment-api.js')

const {
  addWorkflowNodeRequest,
  attachToSessionRequest,
  createSessionRequest,
  createWorkflowScheduleRequest,
  createWorkflowEndpointRequest,
  createWorkflowPublicationRequest,
  createWorkflowRequest,
  endSessionRequest,
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
  'The main dashboard element must include `data-chariox-real-provider-dashboard="true"`.',
  'Before writing the file, reason through a compact layout plan that balances mobile responsiveness, contrast, KPI cards, one chart-like visual, and a status section.',
  'Use inline CSS only; no scripts, external assets, shell commands, file inspection, or network calls.',
  'Keep the HTML under 1800 characters so it fits comfortably in one runtime tool call.',
  'This is the final workflow node: call the Chariox runtime MCP tool validate_and_submit_workflow_run_output directly with workflow_output_json set to {"kind":"html","html":"<full html document>"}.',
  'Do not use bash, python, node, or other provider tools to construct or submit this payload. After the runtime tool succeeds, emit the final fenced workflow JSON block with the same output.message object.',
  'If the Chariox runtime MCP tool is unavailable, do not search for it: emit the final fenced workflow JSON block directly with output.message set to {"kind":"html","html":"<full html document>"}.',
].join(' ')

const SHOPPING_LIST_PROMPT = '1 kg bananas, 2 bottles of 1l Coca-Cola, and a bag of chips'
const SHOPPING_LIST_PROMPT_B = '2 red apples, 1 bag of coffee beans, and 3 packs of pasta'
const SHOPPING_EXPECTED_SNIPPETS = ['Agent App Grocery Checkout', 'data-chariox-agent-app-checkout', 'bananas', 'Coca-Cola', 'chips']
const SHOPPING_EXPECTED_SNIPPETS_B = ['Agent App Grocery Checkout', 'data-chariox-agent-app-checkout', 'apples', 'coffee', 'pasta']
const AGENT_APP_SHOPPING_PROMPT = [
  'You are the checkout automation agent for a published Chariox Agent App.',
  'The invocation prompt is a grocery shopping list from the browser URL.',
  'First call the Chariox runtime MCP tool chariox.agent_app_action for action_id "cart.add" once per product with input containing sku, name, quantity, and unit.',
  'Then call chariox.agent_app_action with action_id "cart.checkout" and input {"ready":true}.',
  'Use the action responses to build a compact checkout page.',
  'The final workflow output must be JSON stringified through validate_and_submit_workflow_run_output with this exact outer shape: {"kind":"response","response":{"mode":"serve","entry":"/generated/checkout.html"},"effects":{"overlay":[{"path":"/generated/checkout.html","mime_type":"text/html; charset=utf-8","content":"<full html document>"}]}}.',
  'The generated HTML must include the visible title text `Agent App Grocery Checkout`, the attribute data-chariox-agent-app-checkout="true", and the products requested by the invocation prompt.',
  'Use inline CSS only. Do not use shell, bash, python, node, filesystem, or network tools.',
  'If the Chariox runtime MCP tools are unavailable, emit the final fenced workflow JSON block directly with the same output.message object, but still include the checkout HTML.',
].join(' ')

function usage() {
  return [
    'Usage: node apps/cli/scripts/live-cloud-publication-deployment-drill.mjs --mode hosted-container|local-runtime [options]',
    '',
    'Options:',
    '  --transport human_http|schedule',
    '  --slug SLUG',
    '  --provider dev-stub|codex|opencode|claude',
    '  --model MODEL',
    '  --credential-profile NAME',
    '  --cloud-api-url URL',
    '  --cloud-account-id ID',
    '  --cloud-session-token TOKEN',
    '  --relay-mode auto|local|cloud',
    '  --kernel-port PORT',
    '  --relay-port PORT',
    '  --serve-port PORT',
    '  --daemon-alias ALIAS',
    '  --relay-token TOKEN',
    '  --real-dashboard',
    '  --agent-app-shopping',
    '  --agent-app-session-isolation',
    '  --artifacts-dir DIR',
    '  --browser-screenshot',
    '  --keep-tmp',
    '  --hold-ms MS',
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
    cloudApiUrl: process.env.CHARIOX_DRILL_CLOUD_API_URL?.trim() || null,
    cloudAccountId: process.env.CHARIOX_DRILL_CLOUD_ACCOUNT_ID?.trim() || null,
    cloudSessionToken: process.env.CHARIOX_DRILL_CLOUD_SESSION_TOKEN?.trim() || null,
    relayMode: process.env.CHARIOX_DRILL_RELAY_MODE?.trim() || 'auto',
    kernelPort: optionalPort(process.env.CHARIOX_DRILL_KERNEL_PORT),
    relayPort: optionalPort(process.env.CHARIOX_DRILL_RELAY_PORT),
    servePort: optionalPort(process.env.CHARIOX_DRILL_SERVE_PORT),
    daemonAlias: process.env.CHARIOX_DRILL_DAEMON_ALIAS?.trim() || null,
    relayToken: process.env.CHARIOX_DRILL_RELAY_TOKEN?.trim() || null,
    realDashboard: false,
    agentAppShopping: false,
    agentAppSessionIsolation: false,
    artifactsDir: path.join(repoRoot, '.artifacts', 'cloud-publication-deployments'),
    browserScreenshot: false,
    keepTmp: false,
    holdMs: 0,
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
    else if (arg === '--cloud-api-url') options.cloudApiUrl = argv[++index]
    else if (arg === '--cloud-account-id') options.cloudAccountId = argv[++index]
    else if (arg === '--cloud-session-token') options.cloudSessionToken = argv[++index]
    else if (arg === '--relay-mode') options.relayMode = argv[++index]
    else if (arg === '--kernel-port') options.kernelPort = requiredPort(argv[++index], '--kernel-port')
    else if (arg === '--relay-port') options.relayPort = requiredPort(argv[++index], '--relay-port')
    else if (arg === '--serve-port') options.servePort = requiredPort(argv[++index], '--serve-port')
    else if (arg === '--daemon-alias') options.daemonAlias = argv[++index]
    else if (arg === '--relay-token') options.relayToken = argv[++index]
    else if (arg === '--real-dashboard') options.realDashboard = true
    else if (arg === '--agent-app-shopping') options.agentAppShopping = true
    else if (arg === '--agent-app-session-isolation') options.agentAppSessionIsolation = true
    else if (arg === '--artifacts-dir') options.artifactsDir = path.resolve(argv[++index])
    else if (arg === '--browser-screenshot') options.browserScreenshot = true
    else if (arg === '--keep-tmp') options.keepTmp = true
    else if (arg === '--hold-ms') options.holdMs = Number.parseInt(argv[++index] ?? '', 10)
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
  if (!['auto', 'local', 'cloud'].includes(options.relayMode)) throw new Error('--relay-mode must be auto, local, or cloud')
  if (Boolean(options.cloudApiUrl) !== Boolean(options.cloudAccountId)) {
    throw new Error('--cloud-api-url and --cloud-account-id must be provided together')
  }
  if (!Number.isFinite(options.holdMs) || options.holdMs < 0) throw new Error('--hold-ms must be a non-negative integer')
  if (!Number.isFinite(options.debugHoldMs) || options.debugHoldMs < 0) throw new Error('--debug-hold-ms must be a non-negative integer')
  return options
}

function normalizeMode(value) {
  if (value === 'hosted-container' || value === 'hosted_container') return 'hosted_container'
  if (value === 'local-runtime' || value === 'local_runtime') return 'local_runtime'
  throw new Error('--mode must be hosted-container or local-runtime')
}

function optionalPort(value) {
  if (!value?.trim()) return null
  return requiredPort(value, 'drill port environment variable')
}

function requiredPort(value, label) {
  const port = Number.parseInt(value ?? '', 10)
  if (!Number.isInteger(port) || port < 1 || port > 65_535) throw new Error(`${label} must be a valid TCP port`)
  return port
}

function normalizeTransport(value) {
  if (value === 'watchdog') return 'schedule'
  if (['human_http', 'schedule'].includes(value)) return value
  throw new Error('--transport must be human_http or schedule')
}

function defaultModel(provider, transport, realDashboard) {
  if (provider === 'dev-stub') {
    if (realDashboard) return 'workflow-html-final-node'
    if (transport === 'schedule') return 'workflow-single-turn-node'
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

function nowStamp() {
  return new Date().toISOString().replace(/[:.]/g, '-')
}

function tail(value, max = 4000) {
  return typeof value === 'string' ? value.slice(-max) : ''
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  await mkdir(options.artifactsDir, { recursive: true })
  const root = path.join(repoRoot, '.artifacts', 'live-cloud-publication-deployment-drill', `chariox-cloud-publication-drill-${nowStamp()}`)
  await prepareDrillArtifacts(root)
  const workspace = path.join(root, 'workspace')
  const exportDir = path.join(root, 'exported')
  const packageDir = path.join(root, 'package')
  const configHome = path.join(root, 'config')
  const serverConfigHome = path.join(root, 'server-config')
  const stateHome = path.join(root, 'state')
  const charioxHome = path.join(root, 'chariox-home')
  const kernelPort = options.kernelPort ?? await freePort()
  const relayPort = options.relayPort ?? await freePort()
  const mcpPort = await freePort()
  const opencodePort = await freePort()
  const codexPort = await freePort()
  const servePort = options.servePort ?? await freePort()
  const actionPort = options.mode === 'local_runtime' ? await freePort() : 33119
  const daemonAlias = options.daemonAlias ?? `cloud-publication-drill-${process.pid}-${Date.now()}`
  const relayToken = options.relayToken ?? `cloud-publication-drill-${process.pid}-${Date.now()}`
  const useLocalRelay = shouldUseLocalRelay(options)
  const baseEnv = {
    ...process.env,
    CHARIOX_HOME: charioxHome,
    XDG_CONFIG_HOME: configHome,
    XDG_STATE_HOME: stateHome,
    CHARIOX_KERNEL_PORT: String(kernelPort),
    CHARIOX_MCP_PORT: String(mcpPort),
    CHARIOX_OPENCODE_PORT: String(opencodePort),
    CHARIOX_CODEX_PORT: String(codexPort),
    CHARIOX_DAEMON_ID: daemonAlias,
    CHARIOX_DAEMON_ALIAS: daemonAlias,
    CHARIOX_DAEMON_SOCKET: path.join(root, 'daemon.sock'),
    CHARIOX_SESSION_HISTORY_DIR: path.join(root, 'history'),
    ...(useLocalRelay ? {
      CHARIOX_RELAY_URL: `ws://127.0.0.1:${relayPort}`,
      CHARIOX_RELAY_TOKEN: relayToken,
    } : {}),
  }
  const env = options.provider === 'dev-stub'
    ? withDevStubProviderInventory(baseEnv)
    : baseEnv
  if (process.env.HOME) {
    if (!env.CODEX_HOME) env.CODEX_HOME = path.join(process.env.HOME, '.codex')
    if (!env.CHARIOX_CLAUDE_CONFIG) env.CHARIOX_CLAUDE_CONFIG = path.join(process.env.HOME, '.claude.json')
  }
  let kernel = null
  let relay = null
  let serve = null
  let actionServer = null
  let client = null
  let deploymentId = null
  let profile = null
  let succeeded = false
  let failure = null
  const sessionIds = []
  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(path.join(configHome, 'chariox'), { recursive: true })
    await mkdir(path.join(serverConfigHome, 'chariox'), { recursive: true })
    await writeFile(path.join(configHome, 'chariox', 'config.toml'), 'version = 1\n', 'utf8')
    await writeFile(path.join(serverConfigHome, 'chariox', 'config.toml'), 'version = 1\n', 'utf8')
    if (!options.cloudApiUrl) await copyCloudProfile(configHome).catch(() => {})

    const kernelBinary = await buildRustBinary('chariox-kernel')
    const relayBinary = await buildRustBinary('chariox-relay')
    if (useLocalRelay) {
      relay = startProcess(relayBinary, [], {
        ...env,
        CHARIOX_RELAY_HOST: '127.0.0.1',
        CHARIOX_RELAY_PORT: String(relayPort),
        CHARIOX_RELAY_TOKEN: relayToken,
      }, 'relay')
      await waitForTcpPort('127.0.0.1', relayPort)
    }
    kernel = startProcess(kernelBinary, [], env, 'kernel')
    const kernelUrl = `ws://127.0.0.1:${kernelPort}`
    await waitForKernel(kernelUrl)
    client = new LocalIpcClient(kernelUrl, { kernelPingIntervalMs: 60_000, kernelMaxMissedPongs: 10 })
    if (options.mode === 'local_runtime' && !useLocalRelay) {
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
    profile = explicitCloudProfile(options) ?? relayCloudProfile(await loadPreferences())
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
        CHARIOX_KERNEL_URL: kernelUrl,
        CHARIOX_PUBLICATION_PACKAGE: packageDir,
        CHARIOX_PUBLICATION_CLOUD_DEPLOYMENT_ID: deployment.id,
        CHARIOX_PUBLICATION_CLOUD_API_URL: profile.apiUrl,
        CHARIOX_PUBLICATION_CLOUD_ACCOUNT_ID: profile.accountId,
        ...(profile.cloudSessionToken ? { CHARIOX_PUBLICATION_CLOUD_SESSION_TOKEN: profile.cloudSessionToken } : {}),
      }, 'chariox-serve-local-runtime')
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
    const logs = options.agentAppShopping
      ? await waitForDeploymentActionAuditLogs(profile, deployment.id)
      : await listPublicationDeploymentLogs(profile, deployment.id).catch((error) => [{ level: 'error', occurredAt: new Date().toISOString(), message: String(error) }])
    const statePath = path.join(options.artifactsDir, `${options.slug}-deployment.json`)
    await writeFile(statePath, `${JSON.stringify({ deployment: readyDeployment, publicationContext, evidence, logs }, null, 2)}\n`)
    logStep('evidence_written', { statePath, ...evidence })
    if (options.holdMs > 0) {
      logStep('hold', {
        ms: options.holdMs,
        deploymentId: readyDeployment.id,
        publicBaseUrl: readyDeployment.publicBaseUrl,
      })
      await delay(options.holdMs)
    }
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (deploymentId && profile) {
      if (options.mode === 'local_runtime') {
        await markLocalDeploymentUnavailable(profile, deploymentId).catch((error) => {
          logStep('cleanup_warning', { action: 'mark-local-backend-unavailable', error: errorMessage(error) })
        })
      }
      await changePublicationDeployment(profile, deploymentId, 'stop').catch((error) => {
        logStep('cleanup_warning', { action: 'stop-deployment', error: errorMessage(error) })
      })
    }
    await stopProcess(serve).catch((error) => logStep('cleanup_warning', { process: 'serve', error: errorMessage(error) }))
    await stopProcess(actionServer).catch((error) => logStep('cleanup_warning', { process: 'actionServer', error: errorMessage(error) }))
    for (const id of sessionIds.reverse()) await client?.send(endSessionRequest(id)).catch(() => {})
    await client?.close?.().catch(() => {})
    await stopProcess(kernel).catch((error) => logStep('cleanup_warning', { process: 'kernel', error: errorMessage(error) }))
    await stopProcess(relay).catch((error) => logStep('cleanup_warning', { process: 'relay', error: errorMessage(error) }))
    if (options.keepTmp && succeeded) {
      logStep('kept_tmp', { root })
    } else {
      await finalizeDrillArtifacts({
        rootDir: root,
        passed: succeeded,
        preserveOnFailure: true,
        failure,
        metadata: {
          drill: 'live-cloud-publication-deployment',
          mode: options.mode,
          transport: options.transport,
          slug: options.slug,
          provider: options.provider,
          model: options.model,
          credentialProfile: options.credentialProfile,
          cloudApiUrl: options.cloudApiUrl,
          relayMode: options.relayMode,
          realDashboard: options.realDashboard,
          agentAppShopping: options.agentAppShopping,
          agentAppSessionIsolation: options.agentAppSessionIsolation,
          browserScreenshot: options.browserScreenshot,
          deploymentId,
          sessionCount: sessionIds.length,
          ports: {
            kernelPort,
            relayPort,
            mcpPort,
            opencodePort,
            codexPort,
            servePort,
            actionPort,
          },
          processLogs: {
            kernelStdout: tail(kernel?.logs?.stdout),
            kernelStderr: tail(kernel?.logs?.stderr),
            relayStdout: tail(relay?.logs?.stdout),
            relayStderr: tail(relay?.logs?.stderr),
            serveStdout: tail(serve?.logs?.stdout),
            serveStderr: tail(serve?.logs?.stderr),
            actionStdout: tail(actionServer?.logs?.stdout),
            actionStderr: tail(actionServer?.logs?.stderr),
          },
        },
      })
    }
  }
}

function explicitCloudProfile(options) {
  if (!options.cloudApiUrl || !options.cloudAccountId) return null
  return {
    apiUrl: options.cloudApiUrl,
    accountId: options.cloudAccountId,
    ...(options.cloudSessionToken ? { cloudSessionToken: options.cloudSessionToken } : {}),
  }
}

function shouldUseLocalRelay(options) {
  if (options.relayMode === 'local') return true
  if (options.relayMode === 'cloud') return false
  return options.mode !== 'local_runtime' || Boolean(options.cloudApiUrl)
}

async function markLocalDeploymentUnavailable(profile, deploymentId) {
  const response = await fetch(
    `${profile.apiUrl.replace(/\/+$/, '')}/publication-deployments/${encodeURIComponent(deploymentId)}/local-backend`,
    {
      method: 'POST',
      headers: {
        accept: 'application/json',
        'content-type': 'application/json',
        ...(profile.cloudSessionToken ? { authorization: `Bearer ${profile.cloudSessionToken}` } : {}),
      },
      body: JSON.stringify({
        accountId: profile.accountId,
        status: 'unavailable',
        lastError: 'Local publication drill completed and released its runtime.',
      }),
    },
  )
  if (!response.ok) {
    throw new Error(`Cloud publication backend cleanup failed with HTTP ${response.status}: ${await response.text()}`)
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
  const endpointKind = transport === 'schedule' ? 'schedule' : transport
  const endpoint = (await client.send(createWorkflowEndpointRequest(session.id, workflow.id, node.id, endpointKind))).WorkflowEndpointCreated.endpoint
  const publication = (await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, publicationOptions(transport, node.id)))).WorkflowPublicationCreated.publication
  let schedule = null
  if (transport === 'schedule') {
    schedule = (await client.send(createWorkflowScheduleRequest(
      session.id,
      workflow.id,
      endpoint.id,
      { kind: 'interval', every_seconds: 60 },
      realDashboard ? REAL_DASHBOARD_PROMPT : 'cloud deployment schedule drill',
      'queue',
      1,
    ))).WorkflowScheduleCreated.schedule
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
      parseShellCommand(`workflow trigger export ${publication.id} ${exportDir} --kernel-url ${kernelUrl}`),
      createDefaultShellContext({ workspace, worktree: workspace, sessionId: session.id, workflowId: workflow.id }),
      { client },
    )
    if (!exportResult.ok) throw new Error(`publication export failed: ${exportResult.message}`)
  }
  if (schedule) {
    const snapshotPath = path.join(exportDir, 'workflow.snapshot.json')
    const snapshot = JSON.parse(await readFile(snapshotPath, 'utf8'))
    if (snapshot.schedules?.[0]?.id !== schedule.id) {
      throw new Error(`expected exported schedule ${schedule.id}, got ${JSON.stringify(snapshot.schedules)}`)
    }
    snapshot.schedules[0].next_run_at_ms = 0
    await writeFile(snapshotPath, `${JSON.stringify(snapshot, null, 2)}\n`)
  }
  await makePortablePackage(exportDir, packageDir)
  return {
    sessionId: session.id,
    workflowId: workflow.id,
    endpointId: endpoint.id,
    publicationId: publication.id,
    scheduleId: schedule?.id ?? null,
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
    parser: { kind: 'path_template', template: '/final/:prompt' },
    traceExposure: { nodes: { [nodeId]: ['output_summary', 'assistant_messages', 'thinking', 'tool_use'] } },
    mode: 'async',
  }
  if (transport === 'human_http') return { ...base, transport: { kind: 'human_http' } }
  if (transport === 'schedule') return { ...base, route: null, methods: [], transport: { kind: 'schedule_only' }, parser: null }
  throw new Error(`unsupported transport ${transport}`)
}

function routeForTransport(transport) {
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
    'function sessionCart(req){const key=String(req.headers["x-chariox-agent-app-session"]||"anonymous");if(!carts.has(key))carts.set(key,[]);return {key,cart:carts.get(key)};}',
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


main().catch((error) => {
  console.error(`[cloud-publication-drill] failed: ${error.stack ?? error.message}`)
  process.exit(1)
})
