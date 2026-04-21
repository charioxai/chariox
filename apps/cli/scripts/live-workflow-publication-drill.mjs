import { spawn } from 'node:child_process'
import { createHmac, generateKeyPairSync, sign } from 'node:crypto'
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
  createWorkflowPublicationPairCodeRequest,
  createWorkflowPublicationRequest,
  createWorkflowRequest,
  endSessionRequest,
  getProviderRunRequest,
  launchProviderRunRequest,
  revokeWorkflowPublicationSenderRequest,
  listSessionsRequest,
  setWorkflowLaunchPolicyRequest,
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

function slackHeaders(secret, body, contentType = 'application/json') {
  const timestamp = String(Math.floor(Date.now() / 1000))
  return {
    'content-type': contentType,
    'x-slack-request-timestamp': timestamp,
    'x-slack-signature': `v0=${createHmac('sha256', secret).update(`v0:${timestamp}:${body}`).digest('hex')}`,
  }
}

function discordHeaders(privateKey, body, options = {}) {
  const timestamp = String(Math.floor(Date.now() / 1000))
  const signature = sign(null, Buffer.from(`${timestamp}${body}`), privateKey).toString('hex')
  return {
    'content-type': 'application/json',
    'x-signature-timestamp': timestamp,
    'x-signature-ed25519': options.tamperSignature ? `00${signature.slice(2)}` : signature,
  }
}

function discordPublicKeyHex(publicKey) {
  return publicKey.export({ format: 'der', type: 'spki' }).subarray(-32).toString('hex')
}

function whatsAppHeaders(secret, body) {
  return {
    'content-type': 'application/json',
    'x-hub-signature-256': `sha256=${createHmac('sha256', secret).update(body).digest('hex')}`,
  }
}

function whatsAppPayload() {
  return {
    object: 'whatsapp_business_account',
    entry: [{
      id: 'business-id',
      changes: [{
        field: 'messages',
        value: {
          messaging_product: 'whatsapp',
          metadata: {
            display_phone_number: '15557654321',
            phone_number_id: 'phone-number-id',
          },
          contacts: [{ wa_id: '15551234567', profile: { name: 'Publication Drill' } }],
          messages: [{ from: '15551234567', id: 'wamid.1', text: { body: 'ship-publication' }, type: 'text' }],
        },
      }],
    }],
  }
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

async function buildKernel() {
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (result.code !== 0) {
    throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  }
  return path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
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
  let sessionId = null
  let succeeded = false
  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(path.join(configHome, 'arroba'), { recursive: true })
    await writeFile(path.join(configHome, 'arroba', 'config.toml'), 'version = 1\n', 'utf8')
    const tls = await createSelfSignedCertificate(root)

    const kernelBinary = await buildKernel()
    kernel = startProcess(kernelBinary, [], env, 'kernel')
    await waitForKernel(kernelUrl)
    client = new LocalIpcClient(kernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })

    logStep('create_session')
    const session = variant(await client.send(createSessionRequest(workspace, workspace, 'publication-drill')), 'SessionCreated').session
    sessionId = session.id
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
    await client.send(setWorkflowLaunchPolicyRequest(session.id, 'queue'))
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
        auth: { mode: 'anonymous' },
        parser: { kind: 'path_template', template: '/qa/:task' },
        mode: 'async',
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
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_PUBLICATION_CONFIG: path.join(exportDir, 'publication.config.json'),
      },
      'gateway-exported',
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
      '--config',
      path.join(exportDir, 'publication.config.json'),
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

    logStep('create_slack_publication')
    const slackSecret = 'publication-drill-slack-secret'
    const slackPublication = variant(
      await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
        alias: 'slack_connector',
        route: '/slack/*',
        methods: ['POST'],
        auth: {
          mode: 'arroba',
          connectors: [{ kind: 'slack', signing_secret_env: 'ARROBA_PUBLICATION_DRILL_SLACK_SECRET' }],
          external_identities: [{
            connector: 'slack',
            external_id: 'T-PUB:U-PUB',
            principal: { id: 'publication-drill-slack-user', type: 'user', allowed_connectors: ['slack'] },
          }],
        },
        parser: { kind: 'webhook' },
        mode: 'async',
      })),
      'WorkflowPublicationCreated',
    ).publication
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: session.id,
        ARROBA_PUBLICATION_ID: slackPublication.id,
        ARROBA_PUBLICATION_DRILL_SLACK_SECRET: slackSecret,
      },
      'gateway-slack',
    )
    await waitForGateway(gatewayUrl)

    logStep('slack_url_verification')
    const challengePayload = JSON.stringify({ type: 'url_verification', challenge: 'arroba-slack-challenge' })
    const challengeResponse = await fetch(`${gatewayUrl}/slack/events`, {
      method: 'POST',
      headers: slackHeaders(slackSecret, challengePayload),
      body: challengePayload,
    })
    const challengeBody = await challengeResponse.text()
    if (challengeResponse.status !== 200 || challengeBody !== 'arroba-slack-challenge') {
      throw new Error(`expected Slack challenge response, got ${challengeResponse.status}: ${challengeBody}`)
    }

    logStep('slack_signed_invoke')
    const slackPayload = new URLSearchParams({
      team_id: 'T-PUB',
      user_id: 'U-PUB',
      command: '/arroba',
      text: 'ship-publication',
    }).toString()
    const slackResponse = await fetch(`${gatewayUrl}/slack/commands`, {
      method: 'POST',
      headers: slackHeaders(slackSecret, slackPayload, 'application/x-www-form-urlencoded'),
      body: slackPayload,
    })
    const slackBody = await slackResponse.json()
    if (slackResponse.status !== 202 || !hasAcceptedRunMetadata(slackBody)) {
      throw new Error(`expected Slack connector HTTP 202, got ${slackResponse.status}: ${JSON.stringify(slackBody)}`)
    }
    await stopProcess(gateway)
    gateway = null

    logStep('create_telegram_publication')
    const telegramSecret = 'publication-drill-telegram-secret'
    const telegramPublication = variant(
      await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
        alias: 'telegram_connector',
        route: '/telegram/*',
        methods: ['POST'],
        auth: {
          mode: 'arroba',
          connectors: [{ kind: 'telegram', webhook_secret_env: 'ARROBA_PUBLICATION_DRILL_TELEGRAM_SECRET' }],
          external_identities: [{
            connector: 'telegram',
            external_id: '456',
            principal: { id: 'publication-drill-telegram-user', type: 'user', allowed_connectors: ['telegram'] },
          }],
        },
        parser: { kind: 'webhook' },
        mode: 'async',
      })),
      'WorkflowPublicationCreated',
    ).publication
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: session.id,
        ARROBA_PUBLICATION_ID: telegramPublication.id,
        ARROBA_PUBLICATION_DRILL_TELEGRAM_SECRET: telegramSecret,
      },
      'gateway-telegram',
    )
    await waitForGateway(gatewayUrl)

    logStep('telegram_secret_reject')
    const telegramPayload = {
      update_id: 123,
      message: {
        message_id: 1,
        chat: { id: 789, type: 'private' },
        from: { id: 456, username: 'publication_drill' },
        text: 'ship-publication',
      },
    }
    const telegramRejected = await fetch(`${gatewayUrl}/telegram/webhook`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-telegram-bot-api-secret-token': 'wrong-secret' },
      body: JSON.stringify(telegramPayload),
    })
    if (telegramRejected.status !== 401) {
      throw new Error(`expected Telegram connector HTTP 401, got ${telegramRejected.status}: ${await telegramRejected.text()}`)
    }

    logStep('telegram_signed_invoke')
    const telegramResponse = await fetch(`${gatewayUrl}/telegram/webhook`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-telegram-bot-api-secret-token': telegramSecret },
      body: JSON.stringify(telegramPayload),
    })
    const telegramBody = await telegramResponse.json()
    if (telegramResponse.status !== 202 || !hasAcceptedRunMetadata(telegramBody)) {
      throw new Error(`expected Telegram connector HTTP 202, got ${telegramResponse.status}: ${JSON.stringify(telegramBody)}`)
    }
    await stopProcess(gateway)
    gateway = null

    logStep('create_discord_publication')
    const discordKeyPair = generateKeyPairSync('ed25519')
    const discordPublication = variant(
      await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
        alias: 'discord_connector',
        route: '/discord/*',
        methods: ['POST'],
        auth: {
          mode: 'arroba',
          connectors: [{ kind: 'discord', public_key_env: 'ARROBA_PUBLICATION_DRILL_DISCORD_PUBLIC_KEY' }],
          external_identities: [{
            connector: 'discord',
            external_id: 'guild-pub:user-pub',
            principal: { id: 'publication-drill-discord-user', type: 'user', allowed_connectors: ['discord'] },
          }],
        },
        parser: { kind: 'webhook' },
        mode: 'async',
      })),
      'WorkflowPublicationCreated',
    ).publication
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: session.id,
        ARROBA_PUBLICATION_ID: discordPublication.id,
        ARROBA_PUBLICATION_DRILL_DISCORD_PUBLIC_KEY: discordPublicKeyHex(discordKeyPair.publicKey),
      },
      'gateway-discord',
    )
    await waitForGateway(gatewayUrl)

    logStep('discord_ping')
    const discordPingPayload = JSON.stringify({ type: 1 })
    const discordPing = await fetch(`${gatewayUrl}/discord/interactions`, {
      method: 'POST',
      headers: discordHeaders(discordKeyPair.privateKey, discordPingPayload),
      body: discordPingPayload,
    })
    const discordPingBody = await discordPing.json()
    if (discordPing.status !== 200 || discordPingBody.type !== 1) {
      throw new Error(`expected Discord ping response, got ${discordPing.status}: ${JSON.stringify(discordPingBody)}`)
    }

    logStep('discord_signature_reject')
    const discordPayload = JSON.stringify({
      type: 2,
      guild_id: 'guild-pub',
      member: { user: { id: 'user-pub', username: 'publication_drill' } },
      data: { name: 'arroba', options: [{ name: 'prompt', value: 'ship-publication' }] },
    })
    const discordRejected = await fetch(`${gatewayUrl}/discord/interactions`, {
      method: 'POST',
      headers: discordHeaders(discordKeyPair.privateKey, discordPayload, { tamperSignature: true }),
      body: discordPayload,
    })
    if (discordRejected.status !== 401) {
      throw new Error(`expected Discord connector HTTP 401, got ${discordRejected.status}: ${await discordRejected.text()}`)
    }

    logStep('discord_signed_invoke')
    const discordResponse = await fetch(`${gatewayUrl}/discord/interactions`, {
      method: 'POST',
      headers: discordHeaders(discordKeyPair.privateKey, discordPayload),
      body: discordPayload,
    })
    const discordBody = await discordResponse.json()
    if (discordResponse.status !== 202 || !hasAcceptedRunMetadata(discordBody)) {
      throw new Error(`expected Discord connector HTTP 202, got ${discordResponse.status}: ${JSON.stringify(discordBody)}`)
    }
    await stopProcess(gateway)
    gateway = null

    logStep('create_whatsapp_publication')
    const whatsAppSecret = 'publication-drill-whatsapp-secret'
    const whatsAppVerifyToken = 'publication-drill-whatsapp-verify-token'
    const whatsAppPublication = variant(
      await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
        alias: 'whatsapp_connector',
        route: '/whatsapp/*',
        methods: ['GET', 'POST'],
        auth: {
          mode: 'arroba',
          connectors: [{
            kind: 'whatsapp',
            app_secret_env: 'ARROBA_PUBLICATION_DRILL_WHATSAPP_SECRET',
            verify_token_env: 'ARROBA_PUBLICATION_DRILL_WHATSAPP_VERIFY_TOKEN',
          }],
          external_identities: [{
            connector: 'whatsapp',
            external_id: '15551234567',
            principal: { id: 'publication-drill-whatsapp-user', type: 'user', allowed_connectors: ['whatsapp'] },
          }],
        },
        parser: { kind: 'webhook' },
        mode: 'async',
      })),
      'WorkflowPublicationCreated',
    ).publication
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: session.id,
        ARROBA_PUBLICATION_ID: whatsAppPublication.id,
        ARROBA_PUBLICATION_DRILL_WHATSAPP_SECRET: whatsAppSecret,
        ARROBA_PUBLICATION_DRILL_WHATSAPP_VERIFY_TOKEN: whatsAppVerifyToken,
      },
      'gateway-whatsapp',
    )
    await waitForGateway(gatewayUrl)

    logStep('whatsapp_verify_challenge')
    const whatsAppChallenge = await fetch(`${gatewayUrl}/whatsapp/webhook?hub.mode=subscribe&hub.verify_token=${encodeURIComponent(whatsAppVerifyToken)}&hub.challenge=arroba-whatsapp-challenge`)
    const whatsAppChallengeBody = await whatsAppChallenge.text()
    if (whatsAppChallenge.status !== 200 || whatsAppChallengeBody !== 'arroba-whatsapp-challenge') {
      throw new Error(`expected WhatsApp challenge response, got ${whatsAppChallenge.status}: ${whatsAppChallengeBody}`)
    }

    logStep('whatsapp_signature_reject')
    const whatsAppBodyText = JSON.stringify(whatsAppPayload())
    const whatsAppRejected = await fetch(`${gatewayUrl}/whatsapp/webhook`, {
      method: 'POST',
      headers: whatsAppHeaders('wrong-secret', whatsAppBodyText),
      body: whatsAppBodyText,
    })
    if (whatsAppRejected.status !== 401) {
      throw new Error(`expected WhatsApp connector HTTP 401, got ${whatsAppRejected.status}: ${await whatsAppRejected.text()}`)
    }

    logStep('whatsapp_signed_invoke')
    const whatsAppResponse = await fetch(`${gatewayUrl}/whatsapp/webhook`, {
      method: 'POST',
      headers: whatsAppHeaders(whatsAppSecret, whatsAppBodyText),
      body: whatsAppBodyText,
    })
    const whatsAppResponseBody = await whatsAppResponse.json()
    if (whatsAppResponse.status !== 202 || !hasAcceptedRunMetadata(whatsAppResponseBody)) {
      throw new Error(`expected WhatsApp connector HTTP 202, got ${whatsAppResponse.status}: ${JSON.stringify(whatsAppResponseBody)}`)
    }
    await stopProcess(gateway)
    gateway = null

    logStep('create_signal_publication')
    const signalSecret = 'publication-drill-signal-secret'
    const signalPublication = variant(
      await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
        alias: 'signal_connector',
        route: '/signal/*',
        methods: ['POST'],
        auth: {
          mode: 'arroba',
          connectors: [{ kind: 'signal', webhook_secret_env: 'ARROBA_PUBLICATION_DRILL_SIGNAL_SECRET' }],
          external_identities: [{
            connector: 'signal',
            external_id: 'signal-source-uuid',
            principal: { id: 'publication-drill-signal-user', type: 'user', allowed_connectors: ['signal'] },
          }],
        },
        parser: { kind: 'webhook' },
        mode: 'async',
      })),
      'WorkflowPublicationCreated',
    ).publication
    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: session.id,
        ARROBA_PUBLICATION_ID: signalPublication.id,
        ARROBA_PUBLICATION_DRILL_SIGNAL_SECRET: signalSecret,
      },
      'gateway-signal',
    )
    await waitForGateway(gatewayUrl)

    const signalPayload = {
      envelope: {
        sourceUuid: 'signal-source-uuid',
        sourceNumber: '+15551234567',
        dataMessage: { message: 'ship-publication' },
      },
    }
    logStep('signal_secret_reject')
    const signalRejected = await fetch(`${gatewayUrl}/signal/webhook`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-signal-webhook-secret': 'wrong-secret' },
      body: JSON.stringify(signalPayload),
    })
    if (signalRejected.status !== 401) {
      throw new Error(`expected Signal connector HTTP 401, got ${signalRejected.status}: ${await signalRejected.text()}`)
    }

    logStep('signal_signed_invoke')
    const signalResponse = await fetch(`${gatewayUrl}/signal/webhook`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-signal-webhook-secret': signalSecret },
      body: JSON.stringify(signalPayload),
    })
    const signalBody = await signalResponse.json()
    if (signalResponse.status !== 202 || !hasAcceptedRunMetadata(signalBody)) {
      throw new Error(`expected Signal connector HTTP 202, got ${signalResponse.status}: ${JSON.stringify(signalBody)}`)
    }
    await stopProcess(gateway)
    gateway = null

    logStep('create_paired_publication')
    const pairedPublication = variant(
      await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
        alias: 'paired_http',
        route: '/paired/*',
        methods: ['GET'],
        auth: { mode: 'arroba', paired_senders: { enabled: true } },
        parser: { kind: 'path_template', template: '/paired/:task' },
        mode: 'async',
      })),
      'WorkflowPublicationCreated',
    ).publication
    const pairCode = variant(
      await client.send(createWorkflowPublicationPairCodeRequest(session.id, pairedPublication.id, null, 1)),
      'WorkflowPublicationPairCodeCreated',
    ).pair_code.pair_code

    gateway = startProcess(
      process.execPath,
      [path.join(repoRoot, 'apps/server/dist/index.js')],
      {
        ...env,
        HOST: '127.0.0.1',
        PORT: String(gatewayPort),
        ARROBA_KERNEL_URL: kernelUrl,
        ARROBA_PUBLICATION_SESSION_ID: session.id,
        ARROBA_PUBLICATION_ID: pairedPublication.id,
      },
      'gateway-paired',
    )
    await waitForGateway(gatewayUrl)

    logStep('paired_rejects_missing_sender')
    const missingAuth = await fetch(`${gatewayUrl}/paired/ship-publication`)
    if (missingAuth.status !== 401) {
      throw new Error(`expected HTTP 401 without paired sender credential, got ${missingAuth.status}: ${await missingAuth.text()}`)
    }

    logStep('paired_redeem')
    const pairResponse = await fetch(`${gatewayUrl}/.well-known/arroba/publication/pair`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ pair_code: pairCode, display_name: 'drill sender' }),
    })
    const pairBody = await pairResponse.json()
    if (pairResponse.status !== 200 || !pairBody.credential || !pairBody.sender?.sender_id) {
      throw new Error(`expected successful sender pairing, got ${pairResponse.status}: ${JSON.stringify(pairBody)}`)
    }

    logStep('paired_invoke')
    const pairedResponse = await fetch(`${gatewayUrl}/paired/ship-publication`, {
      headers: { authorization: `Bearer ${pairBody.credential}` },
    })
    const pairedBody = await pairedResponse.json()
    if (pairedResponse.status !== 202 || !hasAcceptedRunMetadata(pairedBody)) {
      throw new Error(`expected HTTP 202 from paired publication, got ${pairedResponse.status}: ${JSON.stringify(pairedBody)}`)
    }

    await client.send(revokeWorkflowPublicationSenderRequest(session.id, pairedPublication.id, pairBody.sender.sender_id))
    const revokedResponse = await fetch(`${gatewayUrl}/paired/ship-publication`, {
      headers: { authorization: `Bearer ${pairBody.credential}` },
    })
    if (revokedResponse.status !== 401) {
      throw new Error(`expected HTTP 401 after sender revoke, got ${revokedResponse.status}: ${await revokedResponse.text()}`)
    }

    logStep('ok', {
      anonymousPublicationId: publication.id,
      pairedPublicationId: pairedPublication.id,
      pairedWorkflowRunId: pairedBody.workflow_run?.id ?? null,
    })
    succeeded = true
  } finally {
    if (client && sessionId) {
      await client.send(endSessionRequest(sessionId)).catch(() => {})
    }
    await client?.close?.().catch(() => {})
    await stopProcess(gateway)
    await stopProcess(kernel)
    if (!succeeded) {
      console.error('[publication-drill] kernel logs', kernel?.logs ?? null)
      console.error('[publication-drill] gateway logs', gateway?.logs ?? null)
    }
    await rm(root, { recursive: true, force: true })
  }
}

main().catch((error) => {
  console.error(`[publication-drill] failed: ${error.stack ?? error.message}`)
  process.exit(1)
})
