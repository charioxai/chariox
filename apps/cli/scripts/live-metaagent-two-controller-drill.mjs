#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { createWriteStream } from 'node:fs'
import { mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const DEFAULT_TIMEOUT_MS = 900_000
const DEFAULT_POLL_MS = 1_000
const DEFAULT_PROVIDER = process.env.CHARIOX_METAAGENT_TWO_CONTROLLER_PROVIDER ?? 'codex'
const DEFAULT_MODEL = process.env.CHARIOX_METAAGENT_TWO_CONTROLLER_MODEL ?? 'gpt-5.5'
const DEFAULT_EFFORT = process.env.CHARIOX_METAAGENT_TWO_CONTROLLER_EFFORT ?? 'medium'
const ALPHA_PHRASE = 'TWO_META_ALPHA_WORKER_VISIBLE'
const BETA_PHRASE = 'TWO_META_BETA_WORKER_VISIBLE'

function parseArgs(argv) {
  const options = {
    provider: DEFAULT_PROVIDER,
    model: DEFAULT_MODEL,
    effort: DEFAULT_EFFORT,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    keepArtifactsOnFailure: true,
    preserveOnSuccess: true,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--') continue
    else if (arg === '--provider') options.provider = String(argv[++index] ?? '').trim()
    else if (arg === '--model') options.model = String(argv[++index] ?? '').trim()
    else if (arg === '--effort') options.effort = String(argv[++index] ?? '').trim()
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++index])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++index])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--discard-artifacts-on-failure') options.keepArtifactsOnFailure = false
    else if (arg === '--preserve-on-success') options.preserveOnSuccess = true
    else if (arg === '--discard-artifacts-on-success') options.preserveOnSuccess = false
    else if (arg === '--help' || arg === '-h') {
      console.log([
        'Usage: node apps/cli/scripts/live-metaagent-two-controller-drill.mjs [options]',
        '',
        'Runs a real-provider /meta drill with two agents in Meta mode in one session.',
        'Each controller must spawn and prompt only its own worker, then complete its own task.',
      ].join('\n'))
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  if (!options.provider || options.provider === 'dev-stub') {
    throw new Error('two-controller meta drill requires a real provider; dev-stub is not valid evidence')
  }
  if (!options.model) throw new Error('--model is required')
  if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) throw new Error('--timeout-ms must be positive')
  if (!Number.isFinite(options.pollMs) || options.pollMs <= 0) throw new Error('--poll-ms must be positive')
  return options
}

function makePorts() {
  const kernelPort = 63600 + Math.floor(Math.random() * 700)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
}

function log(name, details) {
  if (details === undefined) console.log(`[metaagent-two-controller-drill] ${name}`)
  else console.log(`[metaagent-two-controller-drill] ${name}`, JSON.stringify(details))
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

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

async function runChecked(command, args, options = {}) {
  const result = await run(command, args, options)
  if (result.code !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
  }
  return result
}

function assert(condition, message, details) {
  if (!condition) throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
}

function unwrap(response, key) {
  return response?.[key] ?? response
}

function unwrapVariant(response, ...keys) {
  return keys.map((key) => response?.[key]).find((value) => value != null) ?? response
}

async function buildKernel() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/chariox-kernel')
  await runChecked('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'chariox-kernel'])
  const exists = await stat(binary).then((info) => info.isFile()).catch(() => false)
  if (!exists) throw new Error(`kernel build did not produce ${binary}`)
  return binary
}

async function waitForDaemon(shellBin, kernelUrl, workspace, scriptsDir, env) {
  const scriptPath = path.join(scriptsDir, 'wait.chariox')
  await writeFile(scriptPath, 'session list\n', 'utf8')
  const deadline = Date.now() + 20_000
  let last = null
  while (Date.now() < deadline) {
    last = await run(process.execPath, [shellBin, 'run', scriptPath, '--kernel-url', kernelUrl, '--workspace', workspace, '--worktree', workspace], { env })
    if (last.code === 0) return
    await sleep(250)
  }
  throw new Error(`daemon did not become ready\nstdout:\n${last?.stdout ?? ''}\nstderr:\n${last?.stderr ?? ''}`)
}

async function initGitWorktree(root) {
  await runChecked('git', ['init', '-b', 'main'], { cwd: root })
  await runChecked('git', ['config', 'user.email', 'metaagent-two-controller-drill@example.com'], { cwd: root })
  await runChecked('git', ['config', 'user.name', 'Metaagent Two Controller Drill'], { cwd: root })
}

async function writeFixture(workspace) {
  await writeFile(path.join(workspace, 'README.md'), [
    '# Two Meta Controller Drill',
    '',
    `Alpha worker marker: ${ALPHA_PHRASE}.`,
    `Beta worker marker: ${BETA_PHRASE}.`,
    '',
  ].join('\n'), 'utf8')
  await writeFile(path.join(workspace, '.gitignore'), '.chariox-wait.chariox\n.charioxignore\n', 'utf8')
}

async function getSession(client, requests, sessionId) {
  return unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded').session
}

async function waitForMetaModeAgents(client, requests, sessionId, agentIds, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastSession = null
  while (Date.now() < deadline) {
    lastSession = await getSession(client, requests, sessionId)
    const agents = new Map((lastSession.agents ?? []).map((agent) => [agent.id, agent]))
    if (agentIds.every((agentId) => agents.get(agentId)?.meta_mode)) return lastSession
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for agents to enter Meta mode: ${JSON.stringify(lastSession)}`)
}

async function readHistoryEntries(historyDir) {
  const files = (await readdir(historyDir).catch(() => []))
    .filter((file) => file.endsWith('.jsonl'))
    .sort()
  const entries = []
  for (const file of files) {
    const text = await readFile(path.join(historyDir, file), 'utf8').catch(() => '')
    for (const [index, line] of text.split(/\r?\n/).entries()) {
      if (!line.trim()) continue
      try {
        entries.push({ file, line: index + 1, raw: line, ...JSON.parse(line) })
      } catch {
        entries.push({ file, line: index + 1, raw: line })
      }
    }
  }
  return entries
}

function parseTool(text) {
  if (typeof text !== 'string' || !text.trim().startsWith('{')) return null
  try {
    return JSON.parse(text)
  } catch {
    return null
  }
}

function toolOutputIncludes(tool, phrase) {
  if (typeof tool?.output !== 'string') return false
  return tool.output.includes(phrase)
}

function taskFor(session, metaagentId) {
  return (session.metaagent_tasks ?? []).find((entry) => entry.metaagent_id === metaagentId) ?? null
}

function ownedWorkers(session, metaagentId, controllerIds) {
  return (session.agents ?? []).filter((agent) => (
    !controllerIds.has(agent.id)
    && (agent.controlled_by_metaagent_id === metaagentId
      || agent.controlledByMetaagentId === metaagentId)
  ))
}

function buildMetaPrompt(label, workerAlias, phrase) {
  return `/meta ${[
    `Coordinate a tiny ${label} worker-ownership check.`,
    `Spawn exactly one regular worker named ${workerAlias}.`,
    `Ask that worker to inspect README.md and include the exact phrase ${phrase} in its final response.`,
    'Supervise with whatever Chariox meta tools are appropriate.',
    'Do not use or control any worker that you did not spawn.',
    'When your own worker evidence proves the phrase was produced, complete this Meta-mode task with a concise summary including that exact phrase.',
  ].join(' ')}`
}

async function observeTwoControllers({ client, requests, sessionId, historyDir, alphaId, betaId, timeoutMs, pollMs }) {
  const controllerIds = new Set([alphaId, betaId])
  const state = {
    [alphaId]: { label: 'alpha', phrase: ALPHA_PHRASE, spawned: false, prompted: false, completed: false, sawPhrase: false },
    [betaId]: { label: 'beta', phrase: BETA_PHRASE, spawned: false, prompted: false, completed: false, sawPhrase: false },
  }
  const seenTools = new Set()
  let finalSession = null
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    finalSession = await getSession(client, requests, sessionId)
    for (const entry of await readHistoryEntries(historyDir)) {
      if (entry.kind !== 'provider_tool' || !state[entry.agent_id]) continue
      const tool = parseTool(entry.text)
      if (!tool?.tool) continue
      const key = `${entry.file}:${entry.line}:${entry.agent_id}:${tool.tool}:${tool.status ?? ''}`
      if (seenTools.has(key)) continue
      seenTools.add(key)
      log('controller-tool-observed', {
        controller: state[entry.agent_id].label,
        tool: tool.tool,
        status: tool.status ?? null,
        input: tool.input ?? null,
      })
      assert(
        String(tool.tool).startsWith('chariox.meta.'),
        `${state[entry.agent_id].label} controller must not use provider-native tools during the Meta-mode task`,
        tool,
      )
      const command = String(tool.input?.command ?? '').trim()
      if (tool.tool.includes('run_command') && command.startsWith('agent spawn ')) state[entry.agent_id].spawned = true
      if (tool.tool.includes('run_command') && command.startsWith('prompt ')) state[entry.agent_id].prompted = true
      if (tool.tool.includes('complete_task') && tool.status === 'completed') state[entry.agent_id].completed = true
      if (toolOutputIncludes(tool, state[entry.agent_id].phrase)) state[entry.agent_id].sawPhrase = true
    }
    const alphaTask = taskFor(finalSession, alphaId)
    const betaTask = taskFor(finalSession, betaId)
    const alphaWorkers = ownedWorkers(finalSession, alphaId, controllerIds)
    const betaWorkers = ownedWorkers(finalSession, betaId, controllerIds)
    log('two-controller-state', {
      alphaTask: alphaTask?.status ?? null,
      betaTask: betaTask?.status ?? null,
      alphaWorkers: alphaWorkers.map((agent) => ({ id: agent.id, alias: agent.alias ?? null })),
      betaWorkers: betaWorkers.map((agent) => ({ id: agent.id, alias: agent.alias ?? null })),
    })
    for (const task of [alphaTask, betaTask]) {
      if (task?.status === 'blocked' || task?.status === 'aborted') {
        throw new Error(`Meta-mode task ended as ${task.status}: ${task.blocked_reason ?? task.aborted_reason ?? 'no reason'}`)
      }
    }
    if (alphaTask?.status === 'completed' && betaTask?.status === 'completed') {
      assert(alphaWorkers.length === 1, 'alpha controller should own exactly one worker', alphaWorkers)
      assert(betaWorkers.length === 1, 'beta controller should own exactly one worker', betaWorkers)
      assert(alphaWorkers[0].id !== betaWorkers[0].id, 'controllers must not share a worker', { alphaWorkers, betaWorkers })
      const agents = new Map((finalSession.agents ?? []).map((agent) => [agent.id, agent]))
      assert(!agents.get(alphaId)?.meta_mode, 'alpha controller should exit Meta mode after completion', agents.get(alphaId))
      assert(!agents.get(betaId)?.meta_mode, 'beta controller should exit Meta mode after completion', agents.get(betaId))
      for (const [controllerId, current] of Object.entries(state)) {
        assert(current.spawned, `${current.label} controller should spawn a worker`, current)
        assert(current.prompted, `${current.label} controller should prompt a worker`, current)
        assert(current.completed, `${current.label} controller should call complete_task`, current)
        const task = taskFor(finalSession, controllerId)
        assert((task?.completion_summary ?? '').includes(current.phrase), `${current.label} completion summary should include worker marker`, task)
      }
      return { session: finalSession, alphaTask, betaTask, alphaWorkers, betaWorkers, state }
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for two-controller validation: ${JSON.stringify({ state, session: finalSession }, null, 2)}`)
}

async function cleanupSession(kernelUrl, sessionId) {
  if (!sessionId) return
  const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
  const { endSessionRequest } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
  const client = new LocalIpcClient(kernelUrl)
  try {
    await client.send(endSessionRequest(sessionId)).catch(() => {})
  } finally {
    await client.close().catch(() => {})
  }
}

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null || child.signalCode != null) return
  child.kill(signal)
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(5_000)])
  if (child.exitCode == null && child.signalCode == null) child.kill('SIGKILL')
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-metaagent-two-controller-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home = path.join(rootDir, 'home')
  const scriptsDir = path.join(rootDir, 'scripts')
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const shellBin = path.join(repoRoot, 'apps/shell/dist/shell.js')
  const env = {
    ...process.env,
    HOME: process.env.HOME ?? home,
    CHARIOX_KERNEL_PORT: String(ports.kernelPort),
    CHARIOX_MCP_PORT: String(ports.mcpPort),
    CHARIOX_OPENCODE_PORT: String(ports.opencodePort),
    CHARIOX_CODEX_PORT: String(ports.codexPort),
    CHARIOX_HOME: home,
    CHARIOX_DAEMON_ID: `metaagent-two-controller-drill-${process.pid}-${Date.now()}`,
    CHARIOX_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    CHARIOX_LOG_DIR: path.join(rootDir, 'logs'),
    CHARIOX_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
  }
  let daemon = null
  let client = null
  let sessionId = null
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(workspace, { recursive: true })
    await mkdir(home, { recursive: true })
    await mkdir(scriptsDir, { recursive: true })
    await writeFixture(workspace)
    await initGitWorktree(workspace)
    await runChecked('git', ['add', '.'], { cwd: workspace })
    await runChecked('git', ['commit', '-m', 'Add two-controller fixture'], { cwd: workspace })

    const kernelBinary = await buildKernel()
    const kernelStdout = createWriteStream(path.join(rootDir, 'kernel.stdout.log'), { flags: 'a' })
    const kernelStderr = createWriteStream(path.join(rootDir, 'kernel.stderr.log'), { flags: 'a' })
    daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ['ignore', 'pipe', 'pipe'] })
    daemon.stdout.pipe(kernelStdout)
    daemon.stderr.pipe(kernelStderr)
    daemon.once('exit', (code, signal) => log('daemon-exited', { code, signal }))
    await waitForDaemon(shellBin, kernelUrl, workspace, scriptsDir, env)
    log('daemon-ready', { kernelUrl, provider: options.provider, model: options.model, effort: options.effort })

    const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
    const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')
    client = new LocalIpcClient(kernelUrl, {
      kernelMaxMissedPongs: Math.max(180, Math.ceil(options.timeoutMs / 5_000)),
    })
    const created = unwrap(await client.send(requests.createSessionRequest(
      workspace,
      workspace,
      'two-meta-controller-drill',
      {
        provider: options.provider,
        model: options.model,
        effort: options.effort,
        execution_mode: 'build',
        permission_level: 'yolo',
      },
      null,
      'off',
    )), 'SessionCreated')
    sessionId = created.session.id
    const attachment = unwrap(await client.send(requests.attachToSessionRequest(sessionId, `metaagent-two-controller-drill-${Date.now()}`)), 'SessionAttached').attachment
    const initialSession = await getSession(client, requests, sessionId)
    const alpha = (initialSession.agents ?? []).find((agent) => agent.id === initialSession.focused_agent_id) ?? (initialSession.agents ?? [])[0]
    assert(alpha, 'session should contain the alpha controller')
    await client.send(requests.aliasAgentRequest(sessionId, alpha.id, 'alpha-controller'))
    await client.send(requests.updateAgentProfileRequest({ sessionId, agentId: alpha.id, provider: options.provider, model: options.model, effort: options.effort }))
    const beta = unwrap(await client.send(requests.spawnAgentRequest(
      sessionId,
      options.provider,
      'beta-controller',
      options.model,
      workspace,
      options.effort,
      'build',
      'yolo',
    )), 'AgentSpawned').agent
    log('controllers-ready', { alphaId: alpha.id, betaId: beta.id })

    await client.send(requests.submitPromptRequest(sessionId, attachment.id, alpha.id, buildMetaPrompt('alpha', 'alpha-worker', ALPHA_PHRASE), []))
    await client.send(requests.submitPromptRequest(sessionId, attachment.id, beta.id, buildMetaPrompt('beta', 'beta-worker', BETA_PHRASE), []))
    log('meta-prompts-submitted', { alphaId: alpha.id, betaId: beta.id })
    await waitForMetaModeAgents(client, requests, sessionId, [alpha.id, beta.id], options.timeoutMs, options.pollMs)
    log('controllers-entered-meta-mode', { alphaId: alpha.id, betaId: beta.id })

    const observed = await observeTwoControllers({
      client,
      requests,
      sessionId,
      historyDir: env.CHARIOX_SESSION_HISTORY_DIR,
      alphaId: alpha.id,
      betaId: beta.id,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    console.log(JSON.stringify({
      status: 'ok',
      mode: 'metaagent-two-controller-drill',
      sessionId,
      provider: options.provider,
      model: options.model,
      alpha: {
        id: alpha.id,
        taskStatus: observed.alphaTask.status,
        workers: observed.alphaWorkers.map((agent) => ({ id: agent.id, alias: agent.alias ?? null })),
      },
      beta: {
        id: beta.id,
        taskStatus: observed.betaTask.status,
        workers: observed.betaWorkers.map((agent) => ({ id: agent.id, alias: agent.alias ?? null })),
      },
    }, null, 2))
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client) await client.close().catch(() => {})
    await cleanupSession(kernelUrl, sessionId)
    await terminateChild(daemon)
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      preserveOnSuccess: options.preserveOnSuccess,
      failure,
      metadata: {
        drill: 'metaagent-two-controller',
        provider: options.provider,
        model: options.model,
        effort: options.effort,
        kernelUrl,
        sessionId,
        workspace,
      },
      log,
    })
  }
}

main().catch((error) => {
  console.error(`[metaagent-two-controller-drill] failed: ${error.stack ?? error.message}`)
  process.exit(1)
})
