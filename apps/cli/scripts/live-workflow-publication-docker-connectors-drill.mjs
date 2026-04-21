import { spawn } from 'node:child_process'
import { generateKeyPairSync } from 'node:crypto'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const dockerClientDir = path.join(scriptDir, 'docker', 'publication-connectors')

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')

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
  setWorkflowLaunchPolicyRequest,
  spawnAgentRequest,
  updateWorkflowNodeInstructionsRequest,
} = requests

function logStep(name, details = null) {
  if (details == null) console.log(`[publication-docker-connectors-drill] ${name}`)
  else console.log(`[publication-docker-connectors-drill] ${name}`, JSON.stringify(details))
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

async function buildKernel() {
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
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
  const previousTlsReject = process.env.NODE_TLS_REJECT_UNAUTHORIZED
  if (baseUrl.startsWith('https://')) process.env.NODE_TLS_REJECT_UNAUTHORIZED = '0'
  try {
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
  } finally {
    if (baseUrl.startsWith('https://')) {
      if (previousTlsReject === undefined) delete process.env.NODE_TLS_REJECT_UNAUTHORIZED
      else process.env.NODE_TLS_REJECT_UNAUTHORIZED = previousTlsReject
    }
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
    '/CN=localhost',
    '-addext',
    'subjectAltName=IP:127.0.0.1,DNS:localhost,DNS:host.docker.internal',
    '-days',
    '1',
  ]
  let result = await run('openssl', args, { cwd: root })
  if (result.code !== 0 && result.stderr.includes('addext')) {
    result = await run('openssl', args.filter((arg, index) => arg !== '-addext' && args[index - 1] !== '-addext'), { cwd: root })
  }
  if (result.code !== 0) throw new Error(`openssl self-signed certificate generation failed\n${result.stdout}\n${result.stderr}`)
  return { keyFile, certFile }
}

function discordPublicKeyHex(publicKey) {
  return publicKey.export({ format: 'der', type: 'spki' }).subarray(-32).toString('hex')
}

function dockerHostArgs() {
  if (process.platform === 'linux') return ['--add-host', 'host.docker.internal:host-gateway']
  return []
}

async function buildDockerImage(tag) {
  const version = await run('docker', ['version'])
  if (version.code !== 0) {
    throw new Error(`Docker is required for this drill. Start Docker/Colima first, then rerun.\nstdout:\n${version.stdout}\nstderr:\n${version.stderr}`)
  }
  const built = await run('docker', ['build', '-t', tag, dockerClientDir], { cwd: repoRoot })
  if (built.code !== 0) throw new Error(`failed to build Docker connector client image\nstdout:\n${built.stdout}\nstderr:\n${built.stderr}`)
}

async function runDockerClient(imageTag, connector, baseUrl, env = {}) {
  const args = [
    'run',
    '--rm',
    ...dockerHostArgs(),
    '-e',
    `CONNECTOR=${connector}`,
    '-e',
    `BASE_URL=${baseUrl}`,
    '-e',
    'NODE_TLS_REJECT_UNAUTHORIZED=0',
  ]
  for (const [key, value] of Object.entries(env)) {
    args.push('-e', `${key}=${value}`)
  }
  args.push(imageTag)
  const result = await run('docker', args)
  if (result.code !== 0) {
    throw new Error(`Docker connector client failed for ${connector}\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
  }
  let body
  try {
    body = JSON.parse(result.stdout.trim())
  } catch {
    throw new Error(`Docker connector client returned invalid JSON for ${connector}: ${result.stdout}`)
  }
  logStep(`${connector}_ok`, {
    workflowRunId: body.result?.workflow_run?.id ?? null,
    queued: body.result?.queued === true,
  })
  return body
}

async function withGateway(publication, gatewayPort, env, kernelUrl, sessionId, fn, options = {}) {
  const gateway = startProcess(
    process.execPath,
    [path.join(repoRoot, 'apps/server/dist/index.js')],
    {
      ...env,
      HOST: '0.0.0.0',
      PORT: String(gatewayPort),
      ARROBA_KERNEL_URL: kernelUrl,
      ARROBA_PUBLICATION_SESSION_ID: sessionId,
      ARROBA_PUBLICATION_ID: publication.id,
      ...(options.env ?? {}),
    },
    `gateway-${publication.alias ?? publication.id}`,
  )
  try {
    await waitForGateway(options.localBaseUrl ?? `http://127.0.0.1:${gatewayPort}`)
    return await fn(gateway)
  } finally {
    await stopProcess(gateway)
  }
}

async function main() {
  const root = await mkdtemp(path.join(os.tmpdir(), 'arroba-publication-docker-connectors-drill-'))
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
  const containerHttpUrl = `http://host.docker.internal:${gatewayPort}`
  const containerHttpsUrl = `https://host.docker.internal:${gatewayHttpsPort}`
  const imageTag = `arroba-publication-connectors-drill:${process.pid}`
  const env = {
    ...process.env,
    HOME: home,
    XDG_CONFIG_HOME: configHome,
    XDG_STATE_HOME: stateHome,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(mcpPort),
    ARROBA_OPENCODE_PORT: String(opencodePort),
    ARROBA_CODEX_PORT: String(codexPort),
    ARROBA_DAEMON_ID: `publication-docker-connectors-drill-${process.pid}`,
    ARROBA_DAEMON_SOCKET: path.join(root, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(root, 'history'),
  }

  let kernel = null
  let client = null
  let sessionId = null
  let succeeded = false
  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(path.join(configHome, 'arroba'), { recursive: true })
    await writeFile(path.join(configHome, 'arroba', 'config.toml'), 'version = 1\n', 'utf8')

    logStep('build_docker_client')
    await buildDockerImage(imageTag)

    const tls = await createSelfSignedCertificate(root)
    const kernelBinary = await buildKernel()
    kernel = startProcess(kernelBinary, [], env, 'kernel')
    await waitForKernel(kernelUrl)
    client = new LocalIpcClient(kernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })

    logStep('create_session')
    const session = variant(await client.send(createSessionRequest(workspace, workspace, 'publication-docker-connectors-drill')), 'SessionCreated').session
    sessionId = session.id
    await client.send(attachToSessionRequest(session.id, `publication-docker-connectors-drill-${process.pid}`))

    logStep('spawn_agent')
    const agent = variant(
      await client.send(spawnAgentRequest(session.id, 'dev-stub', 'publisher', 'publication-docker-model', workspace, 'low')),
      'AgentSpawned',
    ).agent
    const providerRun = variant(
      await client.send(launchProviderRunRequest(session.id, 'dev-stub', 'default', 'publication-docker-model', 'low', agent.id)),
      'ProviderRunLaunchAccepted',
    ).provider_run
    await waitForProviderRunReady(client, providerRun.id)

    logStep('create_workflow')
    const workflow = variant(await client.send(createWorkflowRequest(session.id, 'docker-published')), 'WorkflowCreated').workflow
    await client.send(setWorkflowLaunchPolicyRequest(session.id, 'queue'))
    const node = variant(await client.send(addWorkflowNodeRequest(session.id, workflow.id, agent.id)), 'WorkflowNodeAdded').node
    await client.send(updateWorkflowNodeInstructionsRequest(
      session.id,
      workflow.id,
      node.id,
      'For the Docker connector drill, acknowledge the request and complete the workflow.',
    ))
    const endpoint = variant(
      await client.send(createWorkflowEndpointRequest(session.id, workflow.id, node.id, 'docker-connectors')),
      'WorkflowEndpointCreated',
    ).endpoint

    logStep('create_publications')
    const httpPublication = variant(await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
      alias: 'docker_http',
      route: '/docker/*',
      methods: ['GET'],
      auth: { mode: 'anonymous' },
      parser: { kind: 'path_template', template: '/docker/:task' },
      mode: 'async',
    })), 'WorkflowPublicationCreated').publication
    const slackSecret = 'docker-slack-secret'
    const telegramSecret = 'docker-telegram-secret'
    const whatsAppSecret = 'docker-whatsapp-secret'
    const signalSecret = 'docker-signal-secret'
    const discordKeyPair = generateKeyPairSync('ed25519')
    const discordPrivateKeyPem = discordKeyPair.privateKey.export({ format: 'pem', type: 'pkcs8' }).toString()
    const slackPublication = variant(await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
      alias: 'docker_slack',
      route: '/slack/*',
      methods: ['POST'],
      auth: {
        mode: 'arroba',
        connectors: [{ kind: 'slack', signing_secret_env: 'ARROBA_DOCKER_SLACK_SECRET' }],
        external_identities: [{
          connector: 'slack',
          external_id: 'T-DOCKER:U-DOCKER',
          principal: { id: 'docker-slack-user', type: 'user', allowed_connectors: ['slack'] },
        }],
      },
      parser: { kind: 'webhook' },
      mode: 'async',
    })), 'WorkflowPublicationCreated').publication
    const telegramPublication = variant(await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
      alias: 'docker_telegram',
      route: '/telegram/*',
      methods: ['POST'],
      auth: {
        mode: 'arroba',
        connectors: [{ kind: 'telegram', webhook_secret_env: 'ARROBA_DOCKER_TELEGRAM_SECRET' }],
        external_identities: [{
          connector: 'telegram',
          external_id: '456',
          principal: { id: 'docker-telegram-user', type: 'user', allowed_connectors: ['telegram'] },
        }],
      },
      parser: { kind: 'webhook' },
      mode: 'async',
    })), 'WorkflowPublicationCreated').publication
    const discordPublication = variant(await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
      alias: 'docker_discord',
      route: '/discord/*',
      methods: ['POST'],
      auth: {
        mode: 'arroba',
        connectors: [{ kind: 'discord', public_key_env: 'ARROBA_DOCKER_DISCORD_PUBLIC_KEY' }],
        external_identities: [{
          connector: 'discord',
          external_id: 'guild-docker:user-docker',
          principal: { id: 'docker-discord-user', type: 'user', allowed_connectors: ['discord'] },
        }],
      },
      parser: { kind: 'webhook' },
      mode: 'async',
    })), 'WorkflowPublicationCreated').publication
    const whatsAppPublication = variant(await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
      alias: 'docker_whatsapp',
      route: '/whatsapp/*',
      methods: ['POST'],
      auth: {
        mode: 'arroba',
        connectors: [{ kind: 'whatsapp', app_secret_env: 'ARROBA_DOCKER_WHATSAPP_SECRET' }],
        external_identities: [{
          connector: 'whatsapp',
          external_id: '15551234567',
          principal: { id: 'docker-whatsapp-user', type: 'user', allowed_connectors: ['whatsapp'] },
        }],
      },
      parser: { kind: 'webhook' },
      mode: 'async',
    })), 'WorkflowPublicationCreated').publication
    const signalPublication = variant(await client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
      alias: 'docker_signal',
      route: '/signal/*',
      methods: ['POST'],
      auth: {
        mode: 'arroba',
        connectors: [{ kind: 'signal', webhook_secret_env: 'ARROBA_DOCKER_SIGNAL_SECRET' }],
        external_identities: [{
          connector: 'signal',
          external_id: 'signal-source-uuid',
          principal: { id: 'docker-signal-user', type: 'user', allowed_connectors: ['signal'] },
        }],
      },
      parser: { kind: 'webhook' },
      mode: 'async',
    })), 'WorkflowPublicationCreated').publication

    await withGateway(httpPublication, gatewayPort, env, kernelUrl, session.id, async () => {
      await runDockerClient(imageTag, 'http', containerHttpUrl)
      await runDockerClient(imageTag, 'ws', `ws://host.docker.internal:${gatewayPort}`)
    })

    await withGateway(httpPublication, gatewayHttpsPort, env, kernelUrl, session.id, async () => {
      await runDockerClient(imageTag, 'https', containerHttpsUrl)
      await runDockerClient(imageTag, 'wss', `wss://host.docker.internal:${gatewayHttpsPort}`)
    }, {
      localBaseUrl: `https://127.0.0.1:${gatewayHttpsPort}`,
      env: {
        ARROBA_PUBLICATION_TLS_KEY_FILE: tls.keyFile,
        ARROBA_PUBLICATION_TLS_CERT_FILE: tls.certFile,
      },
    })

    await withGateway(slackPublication, gatewayPort, env, kernelUrl, session.id, async () => {
      await runDockerClient(imageTag, 'slack', containerHttpUrl, { SLACK_SECRET: slackSecret })
    }, { env: { ARROBA_DOCKER_SLACK_SECRET: slackSecret } })

    await withGateway(telegramPublication, gatewayPort, env, kernelUrl, session.id, async () => {
      await runDockerClient(imageTag, 'telegram', containerHttpUrl, { TELEGRAM_SECRET: telegramSecret })
    }, { env: { ARROBA_DOCKER_TELEGRAM_SECRET: telegramSecret } })

    await withGateway(discordPublication, gatewayPort, env, kernelUrl, session.id, async () => {
      await runDockerClient(imageTag, 'discord', containerHttpUrl, { DISCORD_PRIVATE_KEY_PEM: discordPrivateKeyPem })
    }, { env: { ARROBA_DOCKER_DISCORD_PUBLIC_KEY: discordPublicKeyHex(discordKeyPair.publicKey) } })

    await withGateway(whatsAppPublication, gatewayPort, env, kernelUrl, session.id, async () => {
      await runDockerClient(imageTag, 'whatsapp', containerHttpUrl, { WHATSAPP_SECRET: whatsAppSecret })
    }, { env: { ARROBA_DOCKER_WHATSAPP_SECRET: whatsAppSecret } })

    await withGateway(signalPublication, gatewayPort, env, kernelUrl, session.id, async () => {
      await runDockerClient(imageTag, 'signal', containerHttpUrl, { SIGNAL_SECRET: signalSecret })
    }, { env: { ARROBA_DOCKER_SIGNAL_SECRET: signalSecret } })

    logStep('ok')
    succeeded = true
  } finally {
    if (client && sessionId) await client.send(endSessionRequest(sessionId)).catch(() => {})
    await client?.close?.().catch(() => {})
    await stopProcess(kernel)
    await run('docker', ['image', 'rm', '-f', imageTag]).catch(() => {})
    if (!succeeded) {
      console.error('[publication-docker-connectors-drill] kernel logs', kernel?.logs ?? null)
    }
    await rm(root, { recursive: true, force: true })
  }
}

main().catch((error) => {
  console.error(`[publication-docker-connectors-drill] failed: ${error.stack ?? error.message}`)
  process.exit(1)
})
