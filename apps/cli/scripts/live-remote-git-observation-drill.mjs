#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (resp, key) => resp?.[key] ?? resp
const unwrapVariant = (resp, ...keys) => keys.map((key) => resp?.[key]).find((value) => value != null) ?? resp

function parseArgs(argv) {
  const options = {
    timeoutMs: 120_000,
    pollMs: 500,
    keepArtifactsOnFailure: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--') continue
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++index])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++index])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help' || arg === '-h') {
      console.log('Usage: node apps/cli/scripts/live-remote-git-observation-drill.mjs [--timeout-ms MS] [--poll-ms MS] [--keep-artifacts-on-failure]')
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
}

function makePorts() {
  const base = 45000 + Math.floor(Math.random() * 1000)
  return {
    relayPort: base,
    homeKernelPort: base + 1000,
    workerKernelPort: base + 1001,
    homeMcpPort: base + 2000,
    workerMcpPort: base + 2001,
    homeOpenCodePort: base + 3000,
    workerOpenCodePort: base + 3001,
    homeCodexPort: base + 3002,
    workerCodexPort: base + 3003,
  }
}

function log(name, details) {
  if (details === undefined) console.log(`[remote-git-observation-drill] ${name}`)
  else console.log(`[remote-git-observation-drill] ${name}`, JSON.stringify(details))
}

async function run(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('error', reject)
    child.on('close', (code, signal) => resolve({ code, signal, stdout, stderr }))
  })
}

async function mustRun(command, args, options = {}) {
  const result = await run(command, args, options)
  if (result.code !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
  }
  return result
}

async function buildBinary(manifestPath, binName) {
  await mustRun('cargo', ['build', '--manifest-path', manifestPath, '--bin', binName])
  return path.join(path.dirname(manifestPath), 'target/debug', binName)
}

function daemonEnv({ baseEnv, ports, rootDir, relayToken, daemonId, daemonAlias, machineId, machineAlias, acceptRemoteLeases, kernelPort, mcpPort, opencodePort, codexPort }) {
  const daemonRoot = path.join(rootDir, daemonId)
  return {
    ...baseEnv,
    HOME: path.join(daemonRoot, 'home'),
    XDG_CONFIG_HOME: path.join(daemonRoot, 'config'),
    XDG_STATE_HOME: path.join(daemonRoot, 'state'),
    XDG_RUNTIME_DIR: path.join(daemonRoot, 'runtime'),
    XDG_DATA_HOME: path.join(daemonRoot, 'data'),
    XDG_CACHE_HOME: path.join(daemonRoot, 'cache'),
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(mcpPort),
    ARROBA_OPENCODE_PORT: String(opencodePort),
    ARROBA_CODEX_PORT: String(codexPort),
    ARROBA_RELAY_URL: `ws://127.0.0.1:${ports.relayPort}`,
    ARROBA_RELAY_TOKEN: relayToken,
    ARROBA_DAEMON_ID: daemonId,
    ARROBA_DAEMON_ALIAS: daemonAlias,
    ARROBA_MACHINE_ID: machineId,
    ARROBA_MACHINE_ALIAS: machineAlias,
    ARROBA_ACCEPT_REMOTE_LEASES: acceptRemoteLeases ? '1' : '0',
    ARROBA_DAEMON_SOCKET: path.join(daemonRoot, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(daemonRoot, 'history'),
    ARROBA_PROVIDER_DEV_STUB: '1',
  }
}

function spawnProcess(command, args, options) {
  const child = spawn(command, args, { ...options, stdio: ['ignore', 'pipe', 'pipe'] })
  child.logs = { stdout: '', stderr: '' }
  child.stdout.on('data', (chunk) => { child.logs.stdout += chunk.toString() })
  child.stderr.on('data', (chunk) => { child.logs.stderr += chunk.toString() })
  return child
}

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null || child.signalCode != null) return
  child.kill(signal)
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    sleep(5_000),
  ])
  if (child.exitCode == null && child.signalCode == null) {
    child.kill('SIGKILL')
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      sleep(2_000),
    ])
  }
}

async function sendWithTimeout(client, request, timeoutMs, description) {
  return await Promise.race([
    client.send(request),
    sleep(timeoutMs).then(() => {
      throw new Error(`timed out during ${description}`)
    }),
  ])
}

async function waitForKernel(LocalIpcClient, requests, kernelUrl) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl)
    try {
      await probe.send(requests.listSessionsRequest())
      await probe.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await probe.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`kernel did not become ready: ${lastError?.message ?? lastError}`)
}

async function waitForRelayTarget(LocalIpcClient, requests, relayUrl, relayToken, targetDaemonAlias) {
  let lastError = null
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const client = new LocalIpcClient(relayUrl, { relayAuthToken: relayToken, targetDaemonAlias })
    try {
      await Promise.race([
        client.send({ GetDaemonHealth: null }),
        sleep(2_000).then(() => { throw new Error('probe timeout') }),
      ])
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`relay target ${targetDaemonAlias} did not become reachable: ${lastError?.message ?? lastError}`)
}

async function waitForRemoteKernel(client, requests, machineRef, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const response = unwrapVariant(
      await sendWithTimeout(
        client,
        { ListRemoteMachineKernels: { machine_ref: machineRef } },
        Math.min(5_000, Math.max(1_000, pollMs + 1_000)),
        `list remote kernels for ${machineRef}`,
      ),
      'RemoteMachineKernelsListed',
    )
    last = response.kernels || []
    const kernel = last.find((candidate) => candidate.accepting_remote_leases && (candidate.available_providers || []).includes('dev-stub'))
    if (kernel) return kernel
    await sleep(pollMs)
  }
  throw new Error(`remote machine ${machineRef} did not advertise dev-stub; last=${JSON.stringify(last)}`)
}

async function waitForHistoryMatch(client, requests, query, filters, label, timeoutMs = 15_000, onPoll = null) {
  const deadline = Date.now() + timeoutMs
  let lastEvents = []
  while (Date.now() < deadline) {
    if (onPoll) await onPoll().catch(() => {})
    const response = unwrapVariant(
      await sendWithTimeout(client, requests.searchRecallRequest(query, filters), 5_000, `search history ${label}`),
      'RecallEvents',
    )
    lastEvents = response.events ?? []
    if (lastEvents.length > 0) return lastEvents
    await sleep(250)
  }
  throw new Error(`timed out waiting for history match ${label}; last=${JSON.stringify(lastEvents)}`)
}

function requireCondition(condition, message, details) {
  if (!condition) throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
}

function requireRemotePlacement(agent, workerKernel) {
  requireCondition(agent.remote_execution?.leased_agent_id, 'remote agent did not receive a worker lease', agent)
  requireCondition(
    agent.remote_execution.worker_kernel_id === workerKernel.kernel_id,
    `remote agent ran on ${agent.remote_execution.worker_kernel_id}, expected ${workerKernel.kernel_id}`,
    agent.remote_execution,
  )
  requireCondition(
    agent.remote_execution.worker_machine_id === workerKernel.machine_id,
    `remote agent ran on machine ${agent.remote_execution.worker_machine_id}, expected ${workerKernel.machine_id}`,
    agent.remote_execution,
  )
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-remote-git-observation-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'worker-repo')
  const ports = makePorts()
  const relayToken = `relay-token-${process.pid}-${Date.now()}`
  const homeDaemonId = `remote-git-home-${process.pid}-${Date.now()}`
  const workerDaemonId = `remote-git-worker-${process.pid}-${Date.now()}`
  const workerMachineId = `machine-remote-git-${process.pid}`
  const workerMachineAlias = `remote-git-builder-${process.pid}`
  const relayUrl = `ws://127.0.0.1:${ports.relayPort}`
  const homeKernelUrl = `ws://127.0.0.1:${ports.homeKernelPort}`
  const baseEnv = { ...process.env }

  const relayEnv = {
    ...baseEnv,
    ARROBA_RELAY_HOST: '127.0.0.1',
    ARROBA_RELAY_PORT: String(ports.relayPort),
    ARROBA_RELAY_TOKEN: relayToken,
  }
  const homeEnv = daemonEnv({
    baseEnv,
    ports,
    rootDir,
    relayToken,
    daemonId: homeDaemonId,
    daemonAlias: 'home',
    machineId: `machine-home-${process.pid}`,
    machineAlias: `home-${process.pid}`,
    acceptRemoteLeases: false,
    kernelPort: ports.homeKernelPort,
    mcpPort: ports.homeMcpPort,
    opencodePort: ports.homeOpenCodePort,
    codexPort: ports.homeCodexPort,
  })
  const workerEnv = daemonEnv({
    baseEnv,
    ports,
    rootDir,
    relayToken,
    daemonId: workerDaemonId,
    daemonAlias: 'worker',
    machineId: workerMachineId,
    machineAlias: workerMachineAlias,
    acceptRemoteLeases: true,
    kernelPort: ports.workerKernelPort,
    mcpPort: ports.workerMcpPort,
    opencodePort: ports.workerOpenCodePort,
    codexPort: ports.workerCodexPort,
  })

  let relayChild = null
  let homeChild = null
  let workerChild = null
  let client = null
  let sessionId = null
  let requests = null
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(workspace, { recursive: true })
    await mustRun('git', ['init'], { cwd: workspace })
    await mustRun('git', ['config', 'user.email', 'remote-drill@example.com'], { cwd: workspace })
    await mustRun('git', ['config', 'user.name', 'Remote Drill Agent'], { cwd: workspace })
    await writeFile(path.join(workspace, 'README.md'), 'seed\n', 'utf8')
    await mustRun('git', ['add', 'README.md'], { cwd: workspace })
    await mustRun('git', ['commit', '-m', 'seed remote drill repo'], { cwd: workspace })

    const [{ LocalIpcClient }, requestModule] = await Promise.all([
      import('../../../packages/kernel-client/dist/ipc.js'),
      import('../../../packages/kernel-client/dist/ipc-requests.js'),
    ])
    requests = requestModule
    const kernelBinary = await buildBinary(path.join(repoRoot, 'apps/kernel/Cargo.toml'), 'arroba-kernel')
    const relayBinary = await buildBinary(path.join(repoRoot, 'apps/relay/Cargo.toml'), 'arroba-relay')

    relayChild = spawnProcess(relayBinary, [], { cwd: repoRoot, env: relayEnv })
    homeChild = spawnProcess(kernelBinary, [], { cwd: repoRoot, env: homeEnv })
    workerChild = spawnProcess(kernelBinary, [], { cwd: repoRoot, env: workerEnv })
    await waitForKernel(LocalIpcClient, requests, homeKernelUrl)
    await waitForRelayTarget(LocalIpcClient, requests, relayUrl, relayToken, 'home')
    await waitForRelayTarget(LocalIpcClient, requests, relayUrl, relayToken, 'worker')

    client = new LocalIpcClient(homeKernelUrl)
    const workerKernel = await waitForRemoteKernel(client, requests, workerMachineId, options.timeoutMs, options.pollMs)
    const created = unwrap(await client.send(requests.createSessionRequest(workspace, workspace, 'remote-git-observation')), 'SessionCreated')
    sessionId = created.session.id
    const attached = unwrap(await client.send(requests.attachToSessionRequest(sessionId, `remote-git-${process.pid}`)), 'SessionAttached')
    const spawned = unwrapVariant(await client.send({
      SpawnAgent: {
        session_id: sessionId,
        alias: 'remote-git-dev',
        provider: 'dev-stub',
        model: 'remote-git-observation-model',
        effort: 'low',
        worktree_id: workspace,
        kernel_ref: workerKernel.kernel_id,
      },
    }), 'AgentSpawned')
    const remoteAgent = spawned.agent
    requireRemotePlacement(remoteAgent, workerKernel)
    log('remote-agent-spawned', {
      sessionId,
      remoteAgentId: remoteAgent.id,
      leasedAgentId: remoteAgent.remote_execution.leased_agent_id,
      workspace,
    })

    const marker = `REMOTE_GIT_OBSERVATION_${process.pid}_${Date.now()}`
    const subject = `remote drill commit ${marker}`
    const submitted = unwrapVariant(
      await sendWithTimeout(
        client,
        requests.submitPromptRequest(sessionId, attached.attachment.id, remoteAgent.id, `Echo marker ${marker}.`, []),
        10_000,
        'submit remote prompt',
      ),
      'PromptSubmitted',
    )
    const startedPrompt = submitted.outcome?.Started?.prompt ?? submitted.outcome?.started?.prompt ?? null
    requireCondition(startedPrompt?.id, 'remote prompt did not start', submitted)
    const promptId = startedPrompt.id
    await sleep(2_000)

    await writeFile(path.join(workspace, 'feature.txt'), `${marker}\n`, 'utf8')
    await mustRun('git', ['add', 'feature.txt'], { cwd: workspace })
    await mustRun('git', ['commit', '-m', subject], { cwd: workspace })
    const commitSha = (await mustRun('git', ['rev-parse', 'HEAD'], { cwd: workspace })).stdout.trim()
    await sendWithTimeout(client, requests.completePromptRequest(sessionId), 10_000, 'complete remote prompt')

    const commitEvents = await waitForHistoryMatch(client, requests, subject, {
      session_id: sessionId,
      kind: 'git_commit_detected',
      provider: 'dev-stub',
      model: 'remote-git-observation-model',
      limit: 10,
    }, 'remote git commit event', options.timeoutMs)
    const matchingEvent = commitEvents.find((event) => {
      const metadata = event.metadata ?? {}
      return metadata.commit_sha === commitSha
        && Array.isArray(metadata.changed_paths)
        && metadata.changed_paths.includes('feature.txt')
        && event.agent_id === remoteAgent.id
        && event.prompt_id === promptId
        && event.machine_id === workerMachineId
        && metadata.repo_root === workspace
        && metadata.worktree_path === workspace
    })
    requireCondition(matchingEvent, 'remote git event did not include expected home attribution and worker repo metadata', {
      commitSha,
      promptId,
      remoteAgentId: remoteAgent.id,
      workerMachineId,
      events: commitEvents,
    })

    log('remote-git-observation-ok', {
      sessionId,
      remoteAgentId: remoteAgent.id,
      promptId,
      commitSha,
      changedPaths: matchingEvent.metadata.changed_paths,
      repoRoot: matchingEvent.metadata.repo_root,
    })
    console.log(JSON.stringify({
      status: 'ok',
      sessionId,
      remoteAgentId: remoteAgent.id,
      promptId,
      eventId: matchingEvent.event_id,
      commitSha,
    }, null, 2))
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client) {
      if (sessionId && requests) await client.send(requests.endSessionRequest(sessionId)).catch(() => {})
      await client.close().catch(() => {})
    }
    await terminateChild(homeChild)
    await terminateChild(workerChild)
    await terminateChild(relayChild)
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: 'remote-git-observation',
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        relayUrl,
        homeKernelUrl,
        workerKernelUrl: `ws://127.0.0.1:${ports.workerKernelPort}`,
        homeDaemonId,
        workerDaemonId,
        workerMachineId,
        workerMachineAlias,
        sessionId,
        workspace,
      },
      log,
    })
  }
}

main().catch((error) => {
  console.error(`[remote-git-observation-drill] failed: ${error.stack ?? error.message}`)
  process.exit(1)
})
