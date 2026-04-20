#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { mkdir, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

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
      console.log('Usage: node apps/cli/scripts/live-remote-restart-drill.mjs [--timeout-ms MS] [--poll-ms MS] [--keep-artifacts-on-failure]')
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
}

function makePorts() {
  const base = 43000 + Math.floor(Math.random() * 1000)
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
  if (details === undefined) console.log(`[remote-restart-drill] ${name}`)
  else console.log(`[remote-restart-drill] ${name}`, JSON.stringify(details))
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

async function buildBinary(manifestPath, binName) {
  const result = await run('cargo', ['build', '--manifest-path', manifestPath, '--bin', binName])
  if (result.code !== 0) {
    throw new Error(`${binName} build failed\n${result.stdout}\n${result.stderr}`)
  }
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
  }
}

function spawnProcess(command, args, options) {
  return spawn(command, args, { ...options, stdio: ['ignore', 'ignore', 'inherit'] })
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
        client.send(requests.listSessionsRequest()),
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

async function waitForRemoteMachine(client, requests, machineRef, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const machines = unwrapVariant(
        await sendWithTimeout(
          client,
          requests.listRemoteMachinesRequest(),
          Math.min(5_000, Math.max(1_000, pollMs + 1_000)),
          `list remote machines for ${machineRef}`,
        ),
        'RemoteMachinesListed',
      ).machines || []
      if (machines.some((machine) => machine.machine_id === machineRef || machine.machine_alias === machineRef)) {
        return machines
      }
    } catch (error) {
      lastError = error
    }
    await sleep(pollMs)
  }
  throw new Error(`remote machine ${machineRef} did not become visible: ${lastError?.message ?? lastError}`)
}

async function waitForRemoteKernel(client, machineRef, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let last = null
  let lastError = null
  while (Date.now() < deadline) {
    try {
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
    } catch (error) {
      lastError = error
    }
    await sleep(pollMs)
  }
  throw new Error(`remote machine ${machineRef} did not advertise a dev-stub worker kernel; last=${JSON.stringify(last)} error=${lastError?.message ?? lastError}`)
}

function requireCondition(condition, message, details) {
  if (!condition) throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
}

async function sendWithTimeout(client, request, timeoutMs, description) {
  return await Promise.race([
    client.send(request),
    sleep(timeoutMs).then(() => {
      throw new Error(`timed out during ${description}`)
    }),
  ])
}

async function attachClient(LocalIpcClient, requests, kernelUrl, sessionId, clientId) {
  const client = new LocalIpcClient(kernelUrl)
  const attachment = unwrap(await client.send(requests.attachToSessionRequest(sessionId, clientId)), 'SessionAttached').attachment
  const events = []
  client.onKernelEvent((event) => events.push({ ...event, observed_at_ms: Date.now() }))
  await client.subscribeToKernelEvents(sessionId, attachment.id)
  return { client, attachment, events }
}

async function promptRemoteAgent({ client, requests, sessionId, attachmentId, agentId, label, events, timeoutMs, pollMs }) {
  const baseline = events.filter((event) => event.event === 'assistant_message_completed').length
  const submitted = unwrapVariant(
    await sendWithTimeout(
      client,
      requests.submitPromptRequest(sessionId, attachmentId, agentId, `Reply with exactly ${label}.`, []),
      Math.min(10_000, timeoutMs),
      `submit prompt ${label}`,
    ),
    'PromptSubmitted',
  )
  const startedPrompt = submitted.outcome?.Started?.prompt ?? submitted.outcome?.started?.prompt ?? null
  const queuedPrompt = submitted.outcome?.Queued?.prompt ?? submitted.outcome?.queued?.prompt ?? null
  requireCondition(
    startedPrompt,
    `remote prompt ${label} was queued instead of started`,
    { outcome: submitted.outcome, queuedPrompt },
  )
  const promptId = startedPrompt.id
  const deadline = Date.now() + timeoutMs
  let completeAttempted = false
  let lastCompleteError = null
  const firstCompleteAttemptAt = Date.now() + pollMs
  while (Date.now() < deadline) {
    await sendWithTimeout(
      client,
      requests.pumpTerminalOutputRequest(sessionId, attachmentId),
      Math.min(5_000, pollMs + 1_000),
      `pump terminal output ${label}`,
    ).catch(() => {})
    const completions = events.filter((event) => event.event === 'assistant_message_completed')
    if (completions.length > baseline) {
      return { promptId, completion: completions.at(-1) }
    }
    if (!completeAttempted && Date.now() >= firstCompleteAttemptAt && Date.now() + pollMs < deadline) {
      completeAttempted = true
      try {
        unwrapVariant(await sendWithTimeout(
          client,
          requests.completePromptRequest(sessionId),
          Math.min(10_000, timeoutMs),
          `complete prompt ${label}`,
        ), 'PromptCompleted')
      } catch (error) {
        lastCompleteError = error
        completeAttempted = false
      }
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for remote prompt ${label}; promptId=${promptId}; lastCompleteError=${lastCompleteError?.message ?? lastCompleteError}`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-remote-restart-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const ports = makePorts()
  const relayToken = `relay-token-${process.pid}-${Date.now()}`
  const homeDaemonId = `remote-restart-home-${process.pid}-${Date.now()}`
  const workerDaemonId = `remote-restart-worker-${process.pid}-${Date.now()}`
  const workerMachineId = `machine-worker-${process.pid}`
  const workerMachineAlias = `builder-${process.pid}`
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
  let attached = null
  let sessionId = null
  let requests = null
  let succeeded = false
  try {
    await mkdir(workspace, { recursive: true })
    const modules = await Promise.all([
      import('../../../packages/kernel-client/dist/ipc.js'),
      import('../../../packages/kernel-client/dist/ipc-requests.js'),
    ])
    const { LocalIpcClient } = modules[0]
    requests = modules[1]
    const kernelBinary = await buildBinary(path.join(repoRoot, 'apps/kernel/Cargo.toml'), 'arroba-kernel')
    const relayBinary = await buildBinary(path.join(repoRoot, 'apps/relay/Cargo.toml'), 'arroba-relay')

    relayChild = spawnProcess(relayBinary, [], { cwd: repoRoot, env: relayEnv })
    homeChild = spawnProcess(kernelBinary, [], { cwd: repoRoot, env: homeEnv })
    workerChild = spawnProcess(kernelBinary, [], { cwd: repoRoot, env: workerEnv })
    await waitForKernel(LocalIpcClient, requests, homeKernelUrl)
    await waitForRelayTarget(LocalIpcClient, requests, relayUrl, relayToken, 'home')
    await waitForRelayTarget(LocalIpcClient, requests, relayUrl, relayToken, 'worker')

    const setupClient = new LocalIpcClient(homeKernelUrl)
    await waitForRemoteMachine(setupClient, requests, workerMachineId, options.timeoutMs, options.pollMs)
    await waitForRemoteKernel(setupClient, workerMachineId, options.timeoutMs, options.pollMs)
    const created = unwrap(await setupClient.send(requests.createSessionRequest(workspace, workspace)), 'SessionCreated')
    sessionId = created.session.id
    const attachedSetup = await attachClient(LocalIpcClient, requests, homeKernelUrl, sessionId, `remote-restart-${process.pid}`)
    attached = attachedSetup
    await setupClient.close().catch(() => {})

    const spawned = unwrapVariant(await attached.client.send({
      SpawnAgent: {
        session_id: sessionId,
        alias: 'remote-dev',
        provider: 'dev-stub',
        model: 'remote-restart-model',
        effort: 'low',
        worktree_id: workspace,
        machine_ref: workerMachineId,
      },
    }), 'AgentSpawned')
    const remoteAgentId = spawned.agent.id
    const initialBinding = spawned.agent.remote_execution
    requireCondition(initialBinding?.leased_agent_id, 'remote agent did not receive a worker lease', spawned.agent)
    log('remote-agent-spawned', { sessionId, remoteAgentId, leasedAgentId: initialBinding.leased_agent_id })

    await promptRemoteAgent({
      client: attached.client,
      requests,
      sessionId,
      attachmentId: attached.attachment.id,
      agentId: remoteAgentId,
      label: 'REMOTE_RESTART_BEFORE_OK',
      events: attached.events,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    log('baseline-prompt-ok')

    await attached.client.close().catch(() => {})
    attached = null
    await terminateChild(homeChild)
    homeChild = spawnProcess(kernelBinary, [], { cwd: repoRoot, env: homeEnv })
    await waitForKernel(LocalIpcClient, requests, homeKernelUrl)
    attached = await attachClient(LocalIpcClient, requests, homeKernelUrl, sessionId, `remote-restart-home-${process.pid}`)
    await waitForRemoteMachine(attached.client, requests, workerMachineId, options.timeoutMs, options.pollMs)
    const afterHomeState = unwrapVariant(await attached.client.send(requests.getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const afterHomeAgent = (afterHomeState.agents || afterHomeState.session?.agents || []).find((agent) => agent.id === remoteAgentId)
    requireCondition(afterHomeAgent?.remote_execution?.leased_agent_id === initialBinding.leased_agent_id, 'home restart did not restore remote agent binding', afterHomeState)
    await promptRemoteAgent({
      client: attached.client,
      requests,
      sessionId,
      attachmentId: attached.attachment.id,
      agentId: remoteAgentId,
      label: 'REMOTE_RESTART_HOME_OK',
      events: attached.events,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    log('home-restart-ok', { leasedAgentId: afterHomeAgent.remote_execution.leased_agent_id })

    await terminateChild(workerChild)
    workerChild = spawnProcess(kernelBinary, [], { cwd: repoRoot, env: workerEnv })
    await waitForRemoteMachine(attached.client, requests, workerMachineId, options.timeoutMs, options.pollMs)
    await waitForRemoteKernel(attached.client, workerMachineId, options.timeoutMs, options.pollMs)
    await promptRemoteAgent({
      client: attached.client,
      requests,
      sessionId,
      attachmentId: attached.attachment.id,
      agentId: remoteAgentId,
      label: 'REMOTE_RESTART_WORKER_OK',
      events: attached.events,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    const afterWorkerState = unwrapVariant(await attached.client.send(requests.getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const afterWorkerAgent = (afterWorkerState.agents || afterWorkerState.session?.agents || []).find((agent) => agent.id === remoteAgentId)
    requireCondition(afterWorkerAgent?.remote_execution?.leased_agent_id, 'worker restart did not leave remote agent bound after prompt-time repair', afterWorkerState)
    requireCondition(afterWorkerAgent.remote_execution.leased_agent_id !== initialBinding.leased_agent_id, 'worker restart did not refresh the stale leased agent id', afterWorkerAgent)
    log('worker-restart-ok', { leasedAgentId: afterWorkerAgent.remote_execution.leased_agent_id })

    await attached.client.close().catch(() => {})
    attached = null
    await terminateChild(homeChild)
    await terminateChild(workerChild)
    homeChild = spawnProcess(kernelBinary, [], { cwd: repoRoot, env: homeEnv })
    workerChild = spawnProcess(kernelBinary, [], { cwd: repoRoot, env: workerEnv })
    await waitForKernel(LocalIpcClient, requests, homeKernelUrl)
    await waitForRelayTarget(LocalIpcClient, requests, relayUrl, relayToken, 'worker')
    attached = await attachClient(LocalIpcClient, requests, homeKernelUrl, sessionId, `remote-restart-both-${process.pid}`)
    await waitForRemoteMachine(attached.client, requests, workerMachineId, options.timeoutMs, options.pollMs)
    await waitForRemoteKernel(attached.client, workerMachineId, options.timeoutMs, options.pollMs)
    await promptRemoteAgent({
      client: attached.client,
      requests,
      sessionId,
      attachmentId: attached.attachment.id,
      agentId: remoteAgentId,
      label: 'REMOTE_RESTART_BOTH_OK',
      events: attached.events,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    const finalState = unwrapVariant(await attached.client.send(requests.getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const finalAgent = (finalState.agents || finalState.session?.agents || []).find((agent) => agent.id === remoteAgentId)
    requireCondition(finalAgent?.remote_execution?.leased_agent_id, 'both restart did not leave remote agent bound after repair', finalState)
    log('both-restart-ok', { leasedAgentId: finalAgent.remote_execution.leased_agent_id })

    console.log(JSON.stringify({
      status: 'ok',
      sessionId,
      remoteAgentId,
      workerMachineId,
      bindings: {
        initial: initialBinding.leased_agent_id,
        afterWorker: afterWorkerAgent.remote_execution.leased_agent_id,
        final: finalAgent.remote_execution.leased_agent_id,
      },
    }, null, 2))
    succeeded = true
  } finally {
    if (attached) {
      if (sessionId && requests) await attached.client.send(requests.endSessionRequest(sessionId)).catch(() => {})
      await attached.client.close().catch(() => {})
    }
    await terminateChild(homeChild)
    await terminateChild(workerChild)
    await terminateChild(relayChild)
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    } else {
      log('kept-artifacts', { rootDir })
    }
  }
}

main().catch((error) => {
  console.error(`[remote-restart-drill] failed: ${error.stack ?? error.message}`)
  process.exit(1)
})
