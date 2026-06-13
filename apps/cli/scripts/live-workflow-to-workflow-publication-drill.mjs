import { spawn } from 'node:child_process'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { access, mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts } from './lib/drill-artifacts.mjs'

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
  createWorkflowPublicationRequest,
  createWorkflowRequest,
  endSessionRequest,
  getProviderRunRequest,
  launchProviderRunRequest,
  listSessionsRequest,
  spawnAgentRequest,
  updateWorkflowNodeInstructionsRequest,
} = requests

function logStep(name, details = null) {
  if (details == null) console.log(`[w2w-publication-drill] ${name}`)
  else console.log(`[w2w-publication-drill] ${name}`, JSON.stringify(details))
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
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
}

async function buildServerGateway() {
  const result = await run('pnpm', ['--filter', '@arroba/server', 'run', 'build'])
  if (result.code !== 0) throw new Error(`server gateway build failed\n${result.stdout}\n${result.stderr}`)
  const serverEntry = path.join(repoRoot, 'apps/server/dist/index.js')
  await access(serverEntry)
  return serverEntry
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

async function startKernelEnvironment(root, kernelBinary, name) {
  const workspace = path.join(root, `${name}-workspace`)
  const home = path.join(root, `${name}-home`)
  const configHome = path.join(root, `${name}-config`)
  const stateHome = path.join(root, `${name}-state`)
  const kernelPort = await freePort()
  const mcpPort = await freePort()
  const opencodePort = await freePort()
  const codexPort = await freePort()
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  await mkdir(workspace, { recursive: true })
  await mkdir(path.join(configHome, 'arroba'), { recursive: true })
  await writeFile(path.join(configHome, 'arroba', 'config.toml'), 'version = 1\n', 'utf8')
  const env = {
    ...process.env,
    HOME: home,
    XDG_CONFIG_HOME: configHome,
    XDG_STATE_HOME: stateHome,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(mcpPort),
    ARROBA_OPENCODE_PORT: String(opencodePort),
    ARROBA_CODEX_PORT: String(codexPort),
    ARROBA_DAEMON_ID: `w2w-publication-${name}-${process.pid}`,
    ARROBA_DAEMON_SOCKET: path.join(root, `${name}.sock`),
    ARROBA_SESSION_HISTORY_DIR: path.join(root, `${name}-history`),
  }
  const kernel = startProcess(kernelBinary, [], env, `kernel-${name}`)
  await waitForKernel(kernelUrl)
  const client = new LocalIpcClient(kernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  return { name, workspace, env, kernel, client, kernelUrl, sessionId: null }
}

async function createPublishedWorkflow(envState, alias, route, parser, gatewayPort) {
  const session = variant(
    await envState.client.send(createSessionRequest(envState.workspace, envState.workspace, `${alias}-session`)),
    'SessionCreated',
  ).session
  envState.sessionId = session.id
  await envState.client.send(attachToSessionRequest(session.id, `${alias}-drill-${process.pid}`))
  const agent = variant(
    await envState.client.send(spawnAgentRequest(session.id, 'dev-stub', `${alias}-agent`, 'publication-drill-model', envState.workspace, 'low')),
    'AgentSpawned',
  ).agent
  const providerRun = variant(
    await envState.client.send(launchProviderRunRequest(session.id, 'dev-stub', 'default', 'publication-drill-model', 'low', agent.id)),
    'ProviderRunLaunchAccepted',
  ).provider_run
  await waitForProviderRunReady(envState.client, providerRun.id)
  const workflow = variant(await envState.client.send(createWorkflowRequest(session.id, alias)), 'WorkflowCreated').workflow
  const node = variant(await envState.client.send(addWorkflowNodeRequest(session.id, workflow.id, agent.id)), 'WorkflowNodeAdded').node
  await envState.client.send(updateWorkflowNodeInstructionsRequest(
    session.id,
    workflow.id,
    node.id,
    `For the workflow-to-workflow publication drill, complete ${alias} deterministically.`,
  ))
  const endpoint = variant(
    await envState.client.send(createWorkflowEndpointRequest(session.id, workflow.id, node.id, 'http')),
    'WorkflowEndpointCreated',
  ).endpoint
  const publication = variant(
    await envState.client.send(createWorkflowPublicationRequest(session.id, workflow.id, endpoint.id, {
      alias: `${alias}_http`,
      route,
      methods: ['GET'],
      auth: { mode: 'anonymous' },
      parser,
      mode: 'async',
    })),
    'WorkflowPublicationCreated',
  ).publication
  const gatewayUrl = `http://127.0.0.1:${gatewayPort}`
  const serverEntry = path.join(repoRoot, 'apps/server/dist/index.js')
  const gateway = startProcess(
    process.execPath,
    [serverEntry],
    {
      ...envState.env,
      HOST: '127.0.0.1',
      PORT: String(gatewayPort),
      ARROBA_KERNEL_URL: envState.kernelUrl,
      ARROBA_PUBLICATION_SESSION_ID: session.id,
      ARROBA_PUBLICATION_ID: publication.id,
    },
    `gateway-${alias}`,
  )
  try {
    await waitForGateway(gatewayUrl)
  } catch (error) {
    await stopProcess(gateway)
    throw new Error([
      `${alias} gateway did not become ready: ${error?.message ?? error}`,
      `entry=${serverEntry}`,
      `stdout=${gateway.logs.stdout.slice(-2000)}`,
      `stderr=${gateway.logs.stderr.slice(-4000)}`,
    ].join('\n'))
  }
  return { session, workflow, endpoint, publication, gateway, gatewayUrl }
}

async function main() {
  const root = await mkdtemp(path.join(os.tmpdir(), 'arroba-w2w-publication-drill-'))
  let home = null
  let worker = null
  let gatewayA = null
  let gatewayB = null
  let succeeded = false
  let failure = null
  let workflowAInfo = null
  let workflowBInfo = null
  let observation = null
  try {
    const [kernelBinary] = await Promise.all([
      buildKernel(),
      buildServerGateway(),
    ])
    home = await startKernelEnvironment(root, kernelBinary, 'home')
    worker = await startKernelEnvironment(root, kernelBinary, 'worker')

    const gatewayBPort = await freePort()
    logStep('publish_workflow_b')
    const workflowB = await createPublishedWorkflow(
      worker,
      'workflow-b',
      '/b/*',
      { kind: 'path_template', template: '/b/:task' },
      gatewayBPort,
    )
    gatewayB = workflowB.gateway
    workflowBInfo = {
      sessionId: workflowB.session.id,
      workflowId: workflowB.workflow.id,
      endpointId: workflowB.endpoint.id,
      publicationId: workflowB.publication.id,
      gatewayUrl: workflowB.gatewayUrl,
    }

    const observationPath = path.join(root, 'workflow-a-parser-observation.json')
    const parserPath = path.join(root, 'workflow-a-parser.mjs')
    await writeFile(parserPath, [
      "let input = '';",
      "process.stdin.on('data', chunk => { input += chunk.toString(); });",
      "process.stdin.on('end', async () => {",
      "  const request = JSON.parse(input || '{}');",
      "  const sourcePath = String(request.url || '/a/unknown').split('?')[0];",
      "  const task = encodeURIComponent(sourcePath.split('/').filter(Boolean).at(-1) || 'unknown');",
      `  const response = await fetch(${JSON.stringify(`${workflowB.gatewayUrl}/b/`)} + task);`,
      "  const body = await response.json();",
      "  const output = {",
      "    source: 'workflow-a',",
      "    requested_path: sourcePath,",
      "    workflow_b_status: response.status,",
      "    workflow_b_run_id: body.workflow_run && body.workflow_run.id,",
      "    workflow_b_accepted: body.accepted === true,",
      "  };",
      `  await import('node:fs/promises').then(fs => fs.writeFile(${JSON.stringify(observationPath)}, JSON.stringify(output, null, 2)));`,
      "  process.stdout.write(JSON.stringify(output));",
      "});",
      "",
    ].join('\n'), 'utf8')

    const gatewayAPort = await freePort()
    logStep('publish_workflow_a')
    const workflowA = await createPublishedWorkflow(
      home,
      'workflow-a',
      '/a/*',
      {
        kind: 'custom_command',
        command: process.execPath,
        args: [parserPath],
      },
      gatewayAPort,
    )
    gatewayA = workflowA.gateway
    workflowAInfo = {
      sessionId: workflowA.session.id,
      workflowId: workflowA.workflow.id,
      endpointId: workflowA.endpoint.id,
      publicationId: workflowA.publication.id,
      gatewayUrl: workflowA.gatewayUrl,
    }

    logStep('invoke_workflow_a_calls_b')
    const response = await fetch(`${workflowA.gatewayUrl}/a/build-feature`)
    const body = await response.json()
    if (response.status !== 202 || !body.workflow_run?.id) {
      throw new Error(`expected workflow A HTTP 202 with run metadata, got ${response.status}: ${JSON.stringify(body)}`)
    }
    observation = JSON.parse(await readFile(observationPath, 'utf8'))
    if (observation.workflow_b_status !== 202 || !observation.workflow_b_run_id || observation.workflow_b_accepted !== true) {
      throw new Error(`workflow A parser did not invoke workflow B correctly: ${JSON.stringify(observation)}`)
    }

    logStep('ok', {
      workflowARunId: body.workflow_run.id,
      workflowBRunId: observation.workflow_b_run_id,
      workflowAEndpoint: workflowA.gatewayUrl,
      workflowBEndpoint: workflowB.gatewayUrl,
    })
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (home?.client && home.sessionId) await home.client.send(endSessionRequest(home.sessionId)).catch(() => {})
    if (worker?.client && worker.sessionId) await worker.client.send(endSessionRequest(worker.sessionId)).catch(() => {})
    await home?.client?.close?.().catch(() => {})
    await worker?.client?.close?.().catch(() => {})
    await stopProcess(gatewayA)
    await stopProcess(gatewayB)
    await stopProcess(home?.kernel)
    await stopProcess(worker?.kernel)
    if (!succeeded) {
      console.error('[w2w-publication-drill] home kernel logs', home?.kernel?.logs ?? null)
      console.error('[w2w-publication-drill] worker kernel logs', worker?.kernel?.logs ?? null)
      console.error('[w2w-publication-drill] gateway A logs', gatewayA?.logs ?? null)
      console.error('[w2w-publication-drill] gateway B logs', gatewayB?.logs ?? null)
    }
    await finalizeDrillArtifacts({
      rootDir: root,
      passed: succeeded,
      preserveOnFailure: true,
      failure,
      metadata: {
        drill: 'workflow-to-workflow-publication',
        homeKernelUrl: home?.kernelUrl ?? null,
        workerKernelUrl: worker?.kernelUrl ?? null,
        homeSessionId: home?.sessionId ?? null,
        workerSessionId: worker?.sessionId ?? null,
        workflowA: workflowAInfo,
        workflowB: workflowBInfo,
        observation,
        homeKernelStdoutTail: home?.kernel?.logs?.stdout?.slice(-4000) ?? '',
        homeKernelStderrTail: home?.kernel?.logs?.stderr?.slice(-4000) ?? '',
        workerKernelStdoutTail: worker?.kernel?.logs?.stdout?.slice(-4000) ?? '',
        workerKernelStderrTail: worker?.kernel?.logs?.stderr?.slice(-4000) ?? '',
        gatewayAStdoutTail: gatewayA?.logs?.stdout?.slice(-4000) ?? '',
        gatewayAStderrTail: gatewayA?.logs?.stderr?.slice(-4000) ?? '',
        gatewayBStdoutTail: gatewayB?.logs?.stdout?.slice(-4000) ?? '',
        gatewayBStderrTail: gatewayB?.logs?.stderr?.slice(-4000) ?? '',
      },
      log: logStep,
    })
  }
}

main().catch((error) => {
  console.error(`[w2w-publication-drill] failed: ${error.stack ?? error.message}`)
  process.exit(1)
})
