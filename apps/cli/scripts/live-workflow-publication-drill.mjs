import { spawn } from 'node:child_process'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { WebSocket } from 'ws'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')
const { createDefaultShellContext, parseShellCommand } = await import('../../../packages/kernel-client/dist/shell-core.js')
const { executeShellCommand } = await import('../../../packages/kernel-client/dist/shell-executor.js')

const {
  addWorkflowNodeRequest,
  attachToSessionRequest,
  createSessionRequest,
  createWorkflowEndpointRequest,
  createWorkflowPublicationRequest,
  createWorkflowRequest,
  endSessionRequest,
  getProviderRunRequest,
  launchProviderRunRequest,
  listSessionsRequest,
  setWorkflowNodeCanCompleteRunRequest,
  spawnAgentRequest,
  updateWorkflowNodeInstructionsRequest,
} = requests

function logStep(name, details = null) {
  if (details == null) console.log(`[publication-drill] ${name}`)
  else console.log(`[publication-drill] ${name}`, JSON.stringify(details))
}

function variant(response, key) {
  return response?.[key] ?? response
}

function hasAcceptedRunMetadata(body) {
  return !!body && (body.workflow_run?.id || body.queued === true)
}

function sseEventNames(body) {
  return [...body.matchAll(/^event: (.+)$/gm)].map((match) => match[1])
}

function createWebSocketReader(socket) {
  const queue = []
  const waiters = []
  let socketError = null
  socket.on('message', (data) => {
    let parsed
    try {
      parsed = JSON.parse(data.toString())
    } catch (error) {
      socketError = error
      return
    }
    const waiter = waiters.shift()
    if (waiter) waiter(parsed)
    else queue.push(parsed)
  })
  socket.on('error', (error) => {
    socketError = error
  })
  return {
    read: async () => {
      if (socketError) throw socketError
      const queued = queue.shift()
      if (queued !== undefined) return queued
      return await new Promise((resolve, reject) => {
        const timeout = setTimeout(() => reject(new Error('timed out waiting for websocket message')), 20_000)
        waiters.push((value) => {
          clearTimeout(timeout)
          resolve(value)
        })
      })
    },
  }
}

async function invokePublicationWebSocket(url, input, options = {}) {
  const socket = new WebSocket(url, options)
  const reader = createWebSocketReader(socket)
  try {
    const ready = await reader.read()
    if (ready.type !== 'ready') {
      throw new Error(`expected websocket ready message, got ${JSON.stringify(ready)}`)
    }
    socket.send(JSON.stringify({
      type: 'artifact_begin',
      artifact_id: 'ws-artifact-1',
      name: 'ws-input.txt',
      mime_type: 'text/plain',
      size_bytes: 9,
    }))
    const begun = await reader.read()
    if (begun.type !== 'artifact_ack' || begun.status !== 'begun') {
      throw new Error(`expected websocket artifact begin ack, got ${JSON.stringify(begun)}`)
    }
    socket.send(JSON.stringify({ type: 'artifact_chunk', artifact_id: 'ws-artifact-1', data: 'd3MtcHVibA==' }))
    const chunk = await reader.read()
    if (chunk.type !== 'artifact_ack' || chunk.status !== 'chunk') {
      throw new Error(`expected websocket artifact chunk ack, got ${JSON.stringify(chunk)}`)
    }
    socket.send(JSON.stringify({ type: 'artifact_end', artifact_id: 'ws-artifact-1' }))
    const readyArtifact = await reader.read()
    if (readyArtifact.type !== 'artifact' || readyArtifact.status !== 'ready') {
      throw new Error(`expected websocket artifact ready message, got ${JSON.stringify(readyArtifact)}`)
    }
    socket.send(JSON.stringify({ type: 'invoke', input }))
    const accepted = await reader.read()
    if (accepted.type !== 'accepted' || (!accepted.workflow_run?.id && !accepted.queued)) {
      throw new Error(`expected websocket accepted run metadata, got ${JSON.stringify(accepted)}`)
    }
    return accepted
  } finally {
    socket.close()
  }
}

async function run(command, args, options = {}) {
  return await new Promise((resolve) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('error', (error) => resolve({ code: 1, stdout, stderr: String(error) }))
    child.on('close', (code) => resolve({ code, stdout, stderr }))
  })
}

async function buildRustBinary(binaryName) {
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', binaryName])
  if (result.code !== 0) {
    throw new Error(`${binaryName} build failed\n${result.stdout}\n${result.stderr}`)
  }
  return path.join(repoRoot, 'apps/kernel/target/debug', binaryName)
}

async function freePort() {
  return await new Promise((resolve, reject) => {
    const server = net.createServer()
    server.listen(0, '127.0.0.1', () => {
      const address = server.address()
      const port = typeof address === 'object' && address ? address.port : null
      server.close(() => port ? resolve(port) : reject(new Error('could not allocate port')))
    })
    server.on('error', reject)
  })
}

function startProcess(command, args, env, name) {
  const logs = { stdout: '', stderr: '' }
  const child = spawn(command, args, {
    cwd: repoRoot,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  child.stdout.on('data', (chunk) => { logs.stdout += chunk.toString() })
  child.stderr.on('data', (chunk) => { logs.stderr += chunk.toString() })
  child.logs = logs
  child.name = name
  return child
}

async function stopProcess(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  await new Promise((resolve) => {
    const timeout = setTimeout(() => {
      if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
    }, 3_000)
    child.once('exit', () => {
      clearTimeout(timeout)
      resolve()
    })
    child.kill('SIGTERM')
  })
}

async function waitForProcessExit(child, timeoutMs = 10_000) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return { code: child.exitCode, signal: child.signalCode }
  }
  return await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      reject(new Error(`${child.name ?? 'process'} did not exit within ${timeoutMs}ms`))
    }, timeoutMs)
    child.once('exit', (code, signal) => {
      clearTimeout(timeout)
      resolve({ code, signal })
    })
  })
}

async function assertGatewayDoesNotListen(baseUrl, timeoutMs = 1_500) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${baseUrl}/health`)
      if (response.ok) {
        throw new Error(`gateway unexpectedly listened at ${baseUrl}`)
      }
    } catch (error) {
      if (/unexpectedly listened/.test(error.message)) throw error
    }
    await new Promise((resolve) => setTimeout(resolve, 150))
  }
}

async function waitForKernel(kernelUrl) {
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(listSessionsRequest())
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await new Promise((resolve) => setTimeout(resolve, 250))
    }
  }
  throw new Error(`kernel did not become ready: ${lastError?.message ?? String(lastError)}`)
}

async function waitForGateway(baseUrl) {
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${baseUrl}/health`)
      if (response.ok) return
      lastError = new Error(`health status ${response.status}`)
    } catch (error) {
      lastError = error
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`gateway did not become ready: ${lastError?.message ?? String(lastError)}`)
}

async function waitForProviderRunReady(client, providerRunId) {
  const deadline = Date.now() + 20_000
  while (Date.now() < deadline) {
    const providerRun = variant(await client.send(getProviderRunRequest(providerRunId)), 'ProviderRun').provider_run
    if (providerRun?.state && providerRun.state !== 'Starting') {
      if (providerRun.state !== 'Running' && providerRun.state !== 'Parked') {
        throw new Error(`provider run ${providerRunId} reached unexpected state ${providerRun.state}`)
      }
      return providerRun
    }
    await new Promise((resolve) => setTimeout(resolve, 250))
  }
  throw new Error(`provider run ${providerRunId} did not become ready`)
}

async function createSelfSignedCertificate(root) {
  const keyFile = path.join(root, 'gateway.key')
  const certFile = path.join(root, 'gateway.crt')
  const args = [
    'req',
    '-x509',
    '-newkey',
    'rsa:2048',
    '-nodes',
    '-keyout',
    keyFile,
    '-out',
    certFile,
    '-subj',
    '/CN=127.0.0.1',
    '-addext',
    'subjectAltName=IP:127.0.0.1,DNS:localhost',
    '-days',
    '1',
  ]
  let result = await run('openssl', args, { cwd: root })
  if (result.code !== 0 && result.stderr.includes('addext')) {
    result = await run('openssl', args.filter((arg, index) => arg !== '-addext' && args[index - 1] !== '-addext'), { cwd: root })
  }
  if (result.code !== 0) {
    throw new Error(`openssl self-signed certificate generation failed\n${result.stdout}\n${result.stderr}`)
  }
  return { keyFile, certFile }
}

async function main() {
  const root = await mkdtemp(path.join(os.tmpdir(), 'arroba-publication-drill-'))
  const workspace = path.join(root, 'workspace')
  const apiSseWorkspace = path.join(root, 'api-sse-workspace')
  const mcpWorkspace = path.join(root, 'mcp-workspace')
  const home = path.join(root, 'home')
  const configHome = path.join(root, 'config')
  const stateHome = path.join(root, 'state')
  const kernelPort = await freePort()
  const mcpPort = await freePort()
  const opencodePort = await freePort()
  const codexPort = await freePort()
  const gatewayPort = await freePort()
  const gatewayHttpsPort = await freePort()
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const gatewayUrl = `http://127.0.0.1:${gatewayPort}`
  const gatewayHttpsUrl = `https://127.0.0.1:${gatewayHttpsPort}`
  const env = {
    ...process.env,
    HOME: home,
    XDG_CONFIG_HOME: configHome,
    XDG_STATE_HOME: stateHome,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(mcpPort),
    ARROBA_OPENCODE_PORT: String(opencodePort),
    ARROBA_CODEX_PORT: String(codexPort),
    ARROBA_DAEMON_ID: `publication-drill-${process.pid}`,
    ARROBA_DAEMON_SOCKET: path.join(root, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(root, 'history'),
  }

  let kernel = null
  let gateway = null
  let client = null
  const sessionIds = []
  let succeeded = false
  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(apiSseWorkspace, { recursive: true })
    await mkdir(mcpWorkspace, { recursive: true })
    await mkdir(path.join(configHome, 'arroba'), { recursive: true })
    await writeFile(path.join(configHome, 'arroba', 'config.toml'), 'version = 1\n', 'utf8')
    const tls = await createSelfSignedCertificate(root)

    const kernelBinary = await buildRustBinary('arroba-kernel')
    const cliBinary = await buildRustBinary('arroba-cli')
    kernel = startProcess(kernelBinary, [], env, 'kernel')
    await waitForKernel(kernelUrl)
    client = new LocalIpcClient(kernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })

    logStep('create_session')
    const session = variant(await client.send(createSessionRequest(workspace, workspace, 'publication-drill')), 'SessionCreated').session
    sessionIds.push(session.id)
    await client.send(attachToSessionRequest(session.id, `publication-drill-${process.pid}`))

    logStep('spawn_agent')
    const agent = variant(
      await client.send(spawnAgentRequest(session.id, 'dev-stub', 'publisher', 'publication-drill-model', workspace, 'low')),
      'AgentSpawned',
    ).agent
    const providerRun = variant(
      await client.send(launchProviderRunRequest(session.id, 'dev-stub', 'default', 'publication-drill-model', 'low', agent.id)),
      'ProviderRunLaunchAccepted',
    ).provider_run
    await waitForProviderRunReady(client, providerRun.id)

    logStep('create_workflow')
    const workflow = variant(await client.send(createWorkflowRequest(session.id, 'published')), 'WorkflowCreated').workflow
    const node = variant(await client.send(addWorkflowNodeRequest(session.id, workflow.id, agent.id)), 'WorkflowNodeAdded').node
    await client.send(updateWorkflowNodeInstructionsRequest(
      session.id,
      workflow.id,
      node.id,
      'For the publication drill, acknowledge the request and complete the workflow.',
    ))
    const endpoint = variant(
      await client.send(createWorkflowEndpointRequest(session.id, workflow.id, node.id, 'http')),
      'WorkflowEndpointCreated',
    ).endpoint

    logStep('create_publication')
    const publication = variant(
      await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
        alias: 'public_http',
        route: '/qa/*',
        methods: ['GET'],
        parser: { kind: 'path_template', template: '/qa/:task' },
        mode: 'async',
      })),
      'WorkflowPublicationCreated',
    ).publication

    logStep('create_api_sse_publication')
    const apiSsePublication = variant(
      await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
        alias: 'public_api_sse',
        route: '/invoke',
        methods: ['POST'],
        transport: { kind: 'api_sse_json' },
        parser: { kind: 'json' },
        mode: 'async',
      })),
      'WorkflowPublicationCreated',
    ).publication

    logStep('create_api_sse_final_session')
    const apiSseSession = variant(
      await client.send(createSessionRequest(apiSseWorkspace, apiSseWorkspace, 'publication-drill-api-sse-final')),
      'SessionCreated',
    ).session
    sessionIds.push(apiSseSession.id)
    await client.send(attachToSessionRequest(apiSseSession.id, `publication-drill-api-sse-final-${process.pid}`))
    const apiSseAgent = variant(
      await client.send(spawnAgentRequest(apiSseSession.id, 'dev-stub', 'api-sse-final', 'workflow-single-turn-node', apiSseWorkspace, 'low')),
      'AgentSpawned',
    ).agent
    const apiSseProviderRun = variant(
      await client.send(launchProviderRunRequest(apiSseSession.id, 'dev-stub', 'default', 'workflow-single-turn-node', 'low', apiSseAgent.id)),
      'ProviderRunLaunchAccepted',
    ).provider_run
    await waitForProviderRunReady(client, apiSseProviderRun.id)
    const apiSseWorkflow = variant(await client.send(createWorkflowRequest(apiSseSession.id, 'published-api-sse-final')), 'WorkflowCreated').workflow
    const apiSseNode = variant(await client.send(addWorkflowNodeRequest(apiSseSession.id, apiSseWorkflow.id, apiSseAgent.id)), 'WorkflowNodeAdded').node
    await client.send(updateWorkflowNodeInstructionsRequest(
      apiSseSession.id,
      apiSseWorkflow.id,
      apiSseNode.id,
      'Complete this publication drill workflow with the deterministic final output envelope emitted by the dev stub.',
    ))
    await client.send(setWorkflowNodeCanCompleteRunRequest(apiSseSession.id, apiSseWorkflow.id, apiSseNode.id, true))
    const apiSseFinalEndpoint = variant(
      await client.send(createWorkflowEndpointRequest(apiSseSession.id, apiSseWorkflow.id, apiSseNode.id, 'api')),
      'WorkflowEndpointCreated',
    ).endpoint
    const apiSseFinalPublication = variant(
      await client.send(createWorkflowPublicationRequest(apiSseSession.id, apiSseWorkflow.id, apiSseFinalEndpoint.id, {
        alias: 'public_api_sse_final',
        route: '/invoke',
        methods: ['POST'],
        transport: { kind: 'api_sse_json' },
        parser: { kind: 'json' },
        mode: 'async',
      })),
      'WorkflowPublicationCreated',
    ).publication
    logStep('create_mcp_session')
    const mcpSession = variant(
      await client.send(createSessionRequest(mcpWorkspace, mcpWorkspace, 'publication-drill-mcp')),
      'SessionCreated',
    ).session
    sessionIds.push(mcpSession.id)
    await client.send(attachToSessionRequest(mcpSession.id, `publication-drill-mcp-${process.pid}`))
    const mcpAgent = variant(
      await client.send(spawnAgentRequest(mcpSession.id, 'dev-stub', 'mcp-final', 'workflow-single-turn-node', mcpWorkspace, 'low')),
      'AgentSpawned',
    ).agent
    const mcpProviderRun = variant(
      await client.send(launchProviderRunRequest(mcpSession.id, 'dev-stub', 'default', 'workflow-single-turn-node', 'low', mcpAgent.id)),
      'ProviderRunLaunchAccepted',
    ).provider_run
    await waitForProviderRunReady(client, mcpProviderRun.id)
    const mcpWorkflow = variant(await client.send(createWorkflowRequest(mcpSession.id, 'published-mcp')), 'WorkflowCreated').workflow
    const mcpNode = variant(await client.send(addWorkflowNodeRequest(mcpSession.id, mcpWorkflow.id, mcpAgent.id)), 'WorkflowNodeAdded').node
    await client.send(updateWorkflowNodeInstructionsRequest(
      mcpSession.id,
      mcpWorkflow.id,
      mcpNode.id,
      'Complete this publication drill workflow with the deterministic final output envelope emitted by the dev stub.',
    ))
    await client.send(setWorkflowNodeCanCompleteRunRequest(mcpSession.id, mcpWorkflow.id, mcpNode.id, true))
    const mcpEndpoint = variant(
      await client.send(createWorkflowEndpointRequest(mcpSession.id, mcpWorkflow.id, mcpNode.id, 'mcp')),
      'WorkflowEndpointCreated',
    ).endpoint
    const mcpPublication = variant(
      await client.send(createWorkflowPublicationRequest(mcpSession.id, mcpWorkflow.id, mcpEndpoint.id, {
        alias: 'public_mcp',
        route: '/mcp',
        methods: ['POST'],
        transport: { kind: 'mcp' },
        parser: { kind: 'json' },
        mode: 'sync',
      })),
      'WorkflowPublicationCreated',
    ).publication

    logStep('start_gateway', { publicationId: publication.id })
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: session.id,
        ARROBA_PUBLICATION_ID: publication.id,
      },
      'gateway',
    )
    await waitForGateway(gatewayUrl)

    logStep('invoke_browser_html')
    const rootHtmlResponse = await fetch(`${gatewayUrl}/`, { headers: { accept: 'text/html' } })
    const rootHtml = await rootHtmlResponse.text()
    if (rootHtmlResponse.status !== 200 || !rootHtml.includes('invoke-form')) {
      throw new Error(`expected browser root form HTML, got ${rootHtmlResponse.status}: ${rootHtml.slice(0, 200)}`)
    }
    if (!rootHtml.includes('type="file" name="artifact" multiple')) {
      throw new Error(`expected browser root form artifact upload input, got: ${rootHtml.slice(0, 300)}`)
    }
    const browserResponse = await fetch(`${gatewayUrl}/qa/browser-publication`, { headers: { accept: 'text/html' } })
    const browserHtml = await browserResponse.text()
    if (browserResponse.status !== 200 || !browserHtml.includes('EventSource')) {
      throw new Error(`expected browser invocation HTML with SSE subscription, got ${browserResponse.status}: ${browserHtml.slice(0, 200)}`)
    }

    logStep('invoke_browser_upload')
    const uploadResponse = await fetch(`${gatewayUrl}/.well-known/arroba/publication/human-http/invoke`, {
      method: 'POST',
      headers: { accept: 'text/html', 'content-type': 'application/json' },
      body: JSON.stringify({
        prompt: 'browser-upload-publication',
        artifacts: [{
          name: 'upload.txt',
          type: 'text/plain',
          size_bytes: 18,
          data_url: 'data:text/plain;base64,cHVibGljYXRpb24tdXBsb2Fk',
        }],
      }),
    })
    const uploadHtml = await uploadResponse.text()
    if (uploadResponse.status !== 200 || !uploadHtml.includes('EventSource')) {
      throw new Error(`expected browser upload invocation HTML with SSE subscription, got ${uploadResponse.status}: ${uploadHtml.slice(0, 200)}`)
    }

    logStep('invoke_http')
    const response = await fetch(`${gatewayUrl}/qa/ship-publication`)
    const body = await response.json()
    if (response.status !== 202) {
      throw new Error(`expected HTTP 202 from async publication, got ${response.status}: ${JSON.stringify(body)}`)
    }
    if (!body.accepted || !hasAcceptedRunMetadata(body)) {
      throw new Error(`gateway did not return accepted workflow run metadata: ${JSON.stringify(body)}`)
    }
    logStep('anonymous_ok', { publicationId: publication.id, workflowRunId: body.workflow_run?.id ?? null, queued: body.queued === true })

    logStep('parser_failure_400')
    const badParserResponse = await fetch(`${gatewayUrl}/qa/a/b`)
    if (badParserResponse.status !== 400) {
      throw new Error(`expected parser failure HTTP 400, got ${badParserResponse.status}: ${await badParserResponse.text()}`)
    }

    logStep('invoke_websocket')
    const webSocketAccepted = await invokePublicationWebSocket(
      `ws://127.0.0.1:${gatewayPort}/.well-known/arroba/publication/ws`,
      { task: 'websocket-publication' },
    )
    logStep('websocket_ok', { workflowRunId: webSocketAccepted.workflow_run?.id ?? null, queued: webSocketAccepted.queued === true })

    await stopProcess(gateway)
    gateway = null

    logStep('invoke_api_sse')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: session.id,
        ARROBA_PUBLICATION_ID: apiSsePublication.id,
      },
      'gateway-api-sse',
    )
    await waitForGateway(gatewayUrl)
    const apiSseResponse = await fetch(`${gatewayUrl}/invoke`, {
      method: 'POST',
      headers: { accept: 'text/event-stream', 'content-type': 'application/json' },
      body: JSON.stringify({
        prompt: 'api-sse-publication',
        artifacts: [{
          name: 'input.txt',
          type: 'text/plain',
          base64: 'YXBpLXN0cmVhbQ==',
        }],
      }),
    })
    const apiSseBody = await apiSseResponse.text()
    if (apiSseResponse.status !== 200 || !apiSseBody.includes('event: queued')) {
      throw new Error(`expected API SSE queued event, got ${apiSseResponse.status}: ${apiSseBody.slice(0, 400)}`)
    }
    logStep('api_sse_queued_ok')
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_api_sse_final')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: apiSseSession.id,
        ARROBA_PUBLICATION_ID: apiSseFinalPublication.id,
      },
      'gateway-api-sse-final',
    )
    await waitForGateway(gatewayUrl)
    const apiSseFinalResponse = await fetch(`${gatewayUrl}/invoke`, {
      method: 'POST',
      headers: { accept: 'text/event-stream', 'content-type': 'application/json' },
      body: JSON.stringify({ prompt: 'api-sse-final-publication' }),
    })
    const apiSseFinalBody = await apiSseFinalResponse.text()
    const apiSseFinalEvents = sseEventNames(apiSseFinalBody)
    if (
      apiSseFinalResponse.status !== 200
      || !apiSseFinalEvents.includes('queued')
      || !apiSseFinalEvents.includes('started')
      || !apiSseFinalEvents.includes('final')
      || (!apiSseFinalBody.includes('"value":1842') && !apiSseFinalBody.includes('\\"value\\":1842'))
    ) {
      const errorIndex = apiSseFinalBody.lastIndexOf('event: error')
      const diagnostic = errorIndex >= 0 ? apiSseFinalBody.slice(errorIndex, errorIndex + 800) : apiSseFinalBody.slice(0, 2_000)
      throw new Error(`expected API SSE queued/started/final with deterministic output, got ${apiSseFinalResponse.status} ${JSON.stringify(apiSseFinalEvents)}: ${diagnostic}`)
    }
    logStep('api_sse_final_ok', { events: apiSseFinalEvents })
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_mcp')
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: mcpSession.id,
        ARROBA_PUBLICATION_ID: mcpPublication.id,
      },
      'gateway-mcp',
    )
    await waitForGateway(gatewayUrl)
    const mcpToolsResponse = await fetch(`${gatewayUrl}/mcp`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list' }),
    })
    const mcpToolsBody = await mcpToolsResponse.json()
    const mcpToolName = mcpToolsBody.result?.tools?.[0]?.name
    if (mcpToolsResponse.status !== 200 || typeof mcpToolName !== 'string') {
      throw new Error(`expected MCP tools/list to expose publication tool, got ${mcpToolsResponse.status}: ${JSON.stringify(mcpToolsBody)}`)
    }
    const mcpCallResponse = await fetch(`${gatewayUrl}/mcp`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: 2,
        method: 'tools/call',
        params: { name: mcpToolName, arguments: { prompt: 'mcp-publication' } },
      }),
    })
    const mcpCallBody = await mcpCallResponse.json()
    const mcpText = mcpCallBody.result?.content?.[0]?.text ?? ''
    if (
      mcpCallResponse.status !== 200
      || mcpCallBody.result?.isError !== false
      || (!mcpText.includes('"value":1842') && !mcpText.includes('\\"value\\":1842'))
    ) {
      throw new Error(`expected MCP tools/call final output, got ${mcpCallResponse.status}: ${JSON.stringify(mcpCallBody).slice(0, 1_200)}`)
    }
    logStep('mcp_ok', { tool: mcpToolName, workflowRunId: mcpCallBody.result?.structuredContent?.workflow_run_id ?? null })
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_https')
    const previousTlsReject = process.env.NODE_TLS_REJECT_UNAUTHORIZED
    process.env.NODE_TLS_REJECT_UNAUTHORIZED = '0'
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayHttpsPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: session.id,
        ARROBA_PUBLICATION_ID: publication.id,
        ARROBA_PUBLICATION_TLS_KEY_FILE: tls.keyFile,
        ARROBA_PUBLICATION_TLS_CERT_FILE: tls.certFile,
      },
      'gateway-https',
    )
    let httpsBody = null
    let httpsResponse = null
    try {
      await waitForGateway(gatewayHttpsUrl)
      httpsResponse = await fetch(`${gatewayHttpsUrl}/qa/ship-publication-secure`)
      httpsBody = await httpsResponse.json()
      if (httpsResponse.status !== 202 || !hasAcceptedRunMetadata(httpsBody)) {
        throw new Error(`expected HTTPS 202 from async publication, got ${httpsResponse.status}: ${JSON.stringify(httpsBody)}`)
      }
      logStep('invoke_wss')
      const wssAccepted = await invokePublicationWebSocket(
        `wss://127.0.0.1:${gatewayHttpsPort}/.well-known/arroba/publication/ws`,
        { task: 'wss-publication' },
        { rejectUnauthorized: false },
      )
      logStep('wss_ok', { workflowRunId: wssAccepted.workflow_run?.id ?? null, queued: wssAccepted.queued === true })
    } finally {
      if (previousTlsReject === undefined) delete process.env.NODE_TLS_REJECT_UNAUTHORIZED
      else process.env.NODE_TLS_REJECT_UNAUTHORIZED = previousTlsReject
    }
    await stopProcess(gateway)
    gateway = null

    logStep('export_publication_package')
    const exportDir = path.join(root, 'exported-publication')
    const exportResult = await executeShellCommand(
      parseShellCommand(`workflow publication export ${publication.id} ${exportDir} --kernel-url ${kernelUrl}`),
      createDefaultShellContext({
        workspace,
        worktree: workspace,
        sessionId: session.id,
        workflowId: workflow.id,
      }),
      { client },
    )
    if (!exportResult.ok) {
      throw new Error(`publication export failed: ${exportResult.message}`)
    }
    gateway = startProcess(
      cliBinary,
      ['serve', exportDir, String(gatewayPort), '--kernel-url', kernelUrl],
      {
        ...env,
        HOST: '127.0.0.1',
      },
      'arroba-serve-exported',
    )
    await waitForGateway(gatewayUrl)
    const exportedResponse = await fetch(`${gatewayUrl}/qa/exported-publication`)
    const exportedBody = await exportedResponse.json()
    if (exportedResponse.status !== 202 || !hasAcceptedRunMetadata(exportedBody)) {
      throw new Error(`expected exported package HTTP 202, got ${exportedResponse.status}: ${JSON.stringify(exportedBody)}`)
    }
    await stopProcess(gateway)
    gateway = null

    logStep('invoke_ipc_exported')
    const ipcResult = await run(process.execPath, [
      path.join(repoRoot, 'apps/server/dist/workflow-call.js'),
      '--package',
      path.join(exportDir, 'publication.json'),
      '--input',
      JSON.stringify({ task: 'ipc-exported-publication' }),
      '--mode',
      'async',
    ], { env })
    if (ipcResult.code !== 0) {
      throw new Error(`expected IPC exported invocation to succeed\nstdout:\n${ipcResult.stdout}\nstderr:\n${ipcResult.stderr}`)
    }
    const ipcBody = JSON.parse(ipcResult.stdout)
    if (!ipcBody.accepted || !hasAcceptedRunMetadata(ipcBody)) {
      throw new Error(`expected IPC accepted run metadata, got ${ipcResult.stdout}`)
    }

    logStep('missing_requirements_fail_before_listen')
    await writeFile(path.join(exportDir, 'requirements.json'), JSON.stringify({
      schema_version: 1,
      skills: [{ name: 'missing-publication-skill' }],
    }, null, 2))
    gateway = startProcess(
      cliBinary,
      ['serve', exportDir, String(gatewayPort), '--kernel-url', kernelUrl],
      {
        ...env,
        HOST: '127.0.0.1',
      },
      'arroba-serve-missing-requirements',
    )
    const failedServe = await waitForProcessExit(gateway)
    await assertGatewayDoesNotListen(gatewayUrl)
    if (failedServe.code === 0) {
      throw new Error('expected missing-requirements serve to fail')
    }
    if (!/publication requirements are missing: skill:missing-publication-skill/.test(gateway.logs.stderr)) {
      throw new Error(`expected missing-requirements error, got stderr:\n${gateway.logs.stderr}`)
    }
    gateway = null

    logStep('ok', {
      anonymousPublicationId: publication.id,
      workflowRunId: body.workflow_run?.id ?? null,
    })
    succeeded = true
  } finally {
    if (client) {
      for (const id of sessionIds.reverse()) {
        await client.send(endSessionRequest(id)).catch(() => {})
      }
    }
    await client?.close?.().catch(() => {})
    await stopProcess(gateway)
    await stopProcess(kernel)
    if (!succeeded) {
      console.error('[publication-drill] kernel logs', kernel?.logs ?? null)
      console.error('[publication-drill] gateway logs', gateway?.logs ?? null)
    }
    await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 })
  }
}

main().catch((error) => {
  console.error(`[publication-drill] failed: ${error.stack ?? error.message}`)
  process.exit(1)
})
