#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')

const {
  createSessionRequest,
  attachToSessionRequest,
  spawnAgentRequest,
  submitPromptRequest,
  completePromptRequest,
  searchRecallRequest,
} = requests

const DRILL_ADAPTER = 'dev-stub'
const DRILL_PROVIDER = 'slow-structured'
const DRILL_MODEL = 'git-observation-model'

function parseArgs(argv) {
  const options = { keepArtifactsOnFailure: false }
  for (const arg of argv) {
    if (arg === '--') continue
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help' || arg === '-h') {
      console.log('Usage: node apps/cli/scripts/live-git-observation-drill.mjs [--keep-artifacts-on-failure]')
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
}

function makePorts() {
  const kernelPort = 50000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
}

function log(name, details) {
  if (details === undefined) console.log(`[git-observation-drill] ${name}`)
  else console.log(`[git-observation-drill] ${name}`, JSON.stringify(details))
}

function tomlString(value) {
  return String(value).replaceAll('\\', '\\\\').replaceAll('"', '\\"')
}

async function sleep(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms))
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

async function buildKernel() {
  await mustRun('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  return path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
}

function startDaemon(binary, env) {
  const logs = { stdout: '', stderr: '' }
  const child = spawn(binary, [], {
    cwd: repoRoot,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  child.stdout.on('data', (chunk) => { logs.stdout += chunk.toString() })
  child.stderr.on('data', (chunk) => { logs.stderr += chunk.toString() })
  child.logs = logs
  return child
}

async function waitForDaemon(kernelUrl, daemon) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    if (daemon?.exitCode !== null || daemon?.signalCode !== null) {
      throw new Error(`daemon exited before ready code=${daemon.exitCode} signal=${daemon.signalCode}\nstdout:\n${daemon.logs?.stdout ?? ''}\nstderr:\n${daemon.logs?.stderr ?? ''}`)
    }
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send({ GetDaemonHealth: null })
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`daemon did not become ready: ${lastError?.message ?? String(lastError)}\nstdout:\n${daemon?.logs?.stdout ?? ''}\nstderr:\n${daemon?.logs?.stderr ?? ''}`)
}

async function waitForHistoryMatch(client, query, filters, label, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs
  let lastEvents = []
  while (Date.now() < deadline) {
    const response = variant(await client.send(searchRecallRequest(query, filters)), 'RecallEvents')
    lastEvents = response.events ?? []
    if (lastEvents.length > 0) return lastEvents
    await sleep(250)
  }
  throw new Error(`timed out waiting for history match ${label}; last=${JSON.stringify(lastEvents)}`)
}

function variant(response, key) {
  if (!response || !response[key]) {
    throw new Error(`expected ${key}, got ${JSON.stringify(response)}`)
  }
  return response[key]
}

function oneOfVariant(response, keys) {
  for (const key of keys) {
    if (response?.[key]) return response[key]
  }
  throw new Error(`expected one of ${keys.join(', ')}, got ${JSON.stringify(response)}`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const ports = makePorts()
  const root = path.join(repoRoot, 'apps/cli/.tmp-live-git-observation-drill', `${process.pid}-${Date.now()}`)
  const home = path.join(root, 'home')
  const configHome = path.join(root, 'config')
  const stateHome = path.join(root, 'state')
  const workspace = path.join(root, 'workspace')
  const statePath = path.join(root, 'state.db')
  const historyPath = path.join(root, 'history.db')
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  let daemon = null
  let client = null
  let succeeded = false
  let failure = null
  let sessionId = null
  let agentId = null
  let promptId = null
  let eventId = null

  try {
    await prepareDrillArtifacts(root)
    await mkdir(workspace, { recursive: true })
    await mkdir(home, { recursive: true })
    await mkdir(path.join(configHome, 'arroba'), { recursive: true })
    await writeFile(
      path.join(configHome, 'arroba', 'config.toml'),
      `version = 1

[history.operational]
path = "${tomlString(historyPath)}"

[history.archive]
mode = "disabled"

[state]
path = "${tomlString(statePath)}"
snapshot_interval_events = 1
`,
      'utf8',
    )
    await mustRun('git', ['init'], { cwd: workspace })
    await mustRun('git', ['config', 'user.email', 'drill@example.com'], { cwd: workspace })
    await mustRun('git', ['config', 'user.name', 'Drill Agent'], { cwd: workspace })
    await writeFile(path.join(workspace, 'README.md'), 'seed\n', 'utf8')
    await mustRun('git', ['add', 'README.md'], { cwd: workspace })
    await mustRun('git', ['commit', '-m', 'seed drill repo'], { cwd: workspace })

    const env = {
      ...process.env,
      HOME: home,
      XDG_CONFIG_HOME: configHome,
      XDG_STATE_HOME: stateHome,
      XDG_DATA_HOME: path.join(home, '.local/share'),
      XDG_CACHE_HOME: path.join(home, '.cache'),
      ARROBA_DAEMON_ID: `git-observation-${process.pid}`,
      ARROBA_MACHINE_ID: `machine-git-observation-${process.pid}`,
      ARROBA_KERNEL_PORT: String(ports.kernelPort),
      ARROBA_MCP_PORT: String(ports.mcpPort),
      ARROBA_OPENCODE_PORT: String(ports.opencodePort),
      ARROBA_CODEX_PORT: String(ports.codexPort),
      ARROBA_DAEMON_SOCKET: path.join(root, 'daemon.sock'),
      ARROBA_SESSION_HISTORY_DIR: path.join(root, 'session-history'),
      ARROBA_PROVIDER_DEV_STUB: '1',
    }

    const kernelBinary = await buildKernel()
    daemon = startDaemon(kernelBinary, env)
    await waitForDaemon(kernelUrl, daemon)
    client = new LocalIpcClient(kernelUrl)

    const created = variant(await client.send(createSessionRequest(workspace, workspace, 'git-observation')), 'SessionCreated')
    const session = created.session
    sessionId = session.id
    const attached = variant(await client.send(attachToSessionRequest(session.id, `git-observation-${process.pid}`)), 'SessionAttached')
    const agent = variant(
      await client.send(spawnAgentRequest(session.id, DRILL_ADAPTER, 'git-worker', DRILL_MODEL, workspace, 'low')),
      'AgentSpawned',
    ).agent
    agentId = agent.id
    const launched = oneOfVariant(
      await client.send({
        LaunchProviderRun: {
          session_id: session.id,
          agent_id: agent.id,
          adapter_key: DRILL_ADAPTER,
          provider: DRILL_PROVIDER,
          account_profile: 'default',
          model: DRILL_MODEL,
          variant: 'low',
          structured_endpoint: null,
          provider_session_id: null,
          native_tui: false,
        },
      }),
      ['ProviderRunLaunchAccepted', 'ProviderRunLaunched'],
    )
    const providerRun = launched.provider_run
    log('provider-launched', {
      sessionId: session.id,
      agentId: agent.id,
      providerRunId: providerRun?.id ?? null,
      workingDirectory: providerRun?.working_directory ?? null,
    })

    const marker = `GIT_OBSERVATION_${process.pid}_${Date.now()}`
    const subject = `drill commit ${marker}`
    const prompt = `Create and commit a file containing marker ${marker}.`
    const submitted = variant(await client.send(submitPromptRequest(session.id, attached.attachment.id, agent.id, prompt, [])), 'PromptSubmitted')
    promptId = submitted.outcome.Started?.prompt?.id ?? submitted.outcome.Started?.prompt_id ?? null
    if (!promptId) throw new Error(`expected started prompt id, got ${JSON.stringify(submitted.outcome)}`)
    await sleep(1_000)

    await writeFile(path.join(workspace, 'feature.txt'), `${marker}\n`, 'utf8')
    await mustRun('git', ['add', 'feature.txt'], { cwd: workspace })
    await mustRun('git', ['commit', '-m', subject], { cwd: workspace })
    await client.send(completePromptRequest(session.id))

    const subjectResults = variant(
      await client.send(searchRecallRequest(subject, {
        session_id: session.id,
        kind: 'git_commit_detected',
        provider: DRILL_PROVIDER,
        model: DRILL_MODEL,
        limit: 10,
      })),
      'RecallEvents',
    ).events
    if (subjectResults.length !== 1) {
      throw new Error(`expected one git commit event by subject, got ${JSON.stringify(subjectResults)}`)
    }
    const event = subjectResults[0]
    if (event.prompt_id !== promptId) {
      throw new Error(`expected git event prompt_id ${promptId}, got ${event.prompt_id}`)
    }
    if (event.agent_id !== agent.id) {
      throw new Error(`expected git event agent_id ${agent.id}, got ${event.agent_id}`)
    }
    if (!event.metadata?.changed_paths?.includes('feature.txt')) {
      throw new Error(`expected changed_paths to include feature.txt, got ${JSON.stringify(event.metadata)}`)
    }

    const pathResults = variant(
      await client.send(searchRecallRequest('feature.txt', {
        session_id: session.id,
        kind: 'git_commit_detected',
        limit: 10,
      })),
      'RecallEvents',
    ).events
    if (!pathResults.some((candidate) => candidate.event_id === event.event_id)) {
      throw new Error(`expected path search to find commit event ${event.event_id}`)
    }
    eventId = event.event_id

    log('git-observation-ok', {
      sessionId: session.id,
      agentId: agent.id,
      promptId,
      commitSha: event.metadata?.commit_sha ?? null,
      changedPaths: event.metadata?.changed_paths ?? [],
    })
    console.log(JSON.stringify({ status: 'ok', sessionId: session.id, promptId, eventId: event.event_id }, null, 2))
    succeeded = true
  } catch (error) {
    failure = error
    console.error(error?.stack ?? error)
    process.exitCode = 1
  } finally {
    await client?.close?.().catch(() => {})
    if (daemon && !daemon.killed) {
      daemon.kill('SIGTERM')
      await sleep(250)
      if (!daemon.killed) daemon.kill('SIGKILL')
    }
    await finalizeDrillArtifacts({
      rootDir: root,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: 'git-observation',
        kernelUrl,
        workspace,
        home,
        configHome,
        stateHome,
        statePath,
        historyPath,
        sessionHistoryDir: path.join(root, 'session-history'),
        sessionId,
        agentId,
        promptId,
        eventId,
      },
      log,
    })
    if (!succeeded && options.keepArtifactsOnFailure) {
      console.error(`[git-observation-drill] preserved artifacts at ${root}`)
    }
  }
}

await main()
