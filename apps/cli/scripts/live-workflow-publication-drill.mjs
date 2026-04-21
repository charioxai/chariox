import { spawn } from 'node:child_process'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')

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
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const gatewayUrl = `http://127.0.0.1:${gatewayPort}`
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
    if (!body.accepted || !body.workflow_run?.id) {
      throw new Error(`gateway did not return accepted workflow run metadata: ${JSON.stringify(body)}`)
    }
    logStep('anonymous_ok', { publicationId: publication.id, workflowRunId: body.workflow_run.id })
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
    if (pairedResponse.status !== 202 || !pairedBody.workflow_run?.id) {
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
      pairedWorkflowRunId: pairedBody.workflow_run.id,
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
