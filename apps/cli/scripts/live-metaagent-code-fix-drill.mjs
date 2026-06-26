#!/usr/bin/env node
import { createHash } from 'node:crypto'
import { spawn } from 'node:child_process'
import { mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const DEFAULT_TIMEOUT_MS = 900_000
const DEFAULT_POLL_MS = 1_000
const DEFAULT_PROVIDER = process.env.ARROBA_METAAGENT_CODE_FIX_PROVIDER ?? 'codex'
const DEFAULT_MODEL = process.env.ARROBA_METAAGENT_CODE_FIX_MODEL ?? 'gpt-5.5'
const DEFAULT_EFFORT = process.env.ARROBA_METAAGENT_CODE_FIX_EFFORT ?? 'medium'
const USER_PROMPT = 'The repo has a small failing JavaScript project. Delegate the investigation, fix, and verification to regular agent(s), get the project to a passing state, then mark this task complete with a concise report of what changed and how it was verified.'

function parseArgs(argv) {
  const options = {
    provider: DEFAULT_PROVIDER,
    model: DEFAULT_MODEL,
    effort: DEFAULT_EFFORT,
    accountProfile: 'default',
    keepArtifactsOnFailure: true,
    preserveOnSuccess: true,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--') continue
    else if (arg === '--provider') options.provider = String(argv[++index] ?? '').trim()
    else if (arg === '--model') options.model = String(argv[++index] ?? '').trim()
    else if (arg === '--effort') options.effort = String(argv[++index] ?? '').trim()
    else if (arg === '--account-profile') options.accountProfile = String(argv[++index] ?? '').trim()
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++index])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++index])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--discard-artifacts-on-failure') options.keepArtifactsOnFailure = false
    else if (arg === '--preserve-on-success') options.preserveOnSuccess = true
    else if (arg === '--discard-artifacts-on-success') options.preserveOnSuccess = false
    else if (arg === '--help' || arg === '-h') {
      console.log([
        'Usage: node apps/cli/scripts/live-metaagent-code-fix-drill.mjs [options]',
        '',
        'Runs an observe-only live metaagent behavior drill against a real provider:',
        '- creates a tiny JavaScript project with one real bug and one failing test',
        '- starts a real metaagent in plan mode',
        '- submits exactly one high-level prompt to that metaagent',
        '- observes kernel/session/metaagent events only after the prompt',
        '- passes only if a regular worker fixes the source and npm test passes',
        '',
        'Options:',
        `  --provider ${DEFAULT_PROVIDER}`,
        `  --model ${DEFAULT_MODEL}`,
        `  --effort ${DEFAULT_EFFORT}`,
        '  --account-profile default',
        `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
        `  --poll-ms ${DEFAULT_POLL_MS}`,
        '  --keep-artifacts-on-failure',
        '  --discard-artifacts-on-failure',
        '  --preserve-on-success',
        '  --discard-artifacts-on-success',
      ].join('\n'))
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  if (!options.provider || options.provider === 'dev-stub') {
    throw new Error('metaagent code-fix drill requires a real provider; dev-stub is not valid evidence')
  }
  if (!options.model) throw new Error('--model is required')
  if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) throw new Error('--timeout-ms must be positive')
  if (!Number.isFinite(options.pollMs) || options.pollMs <= 0) throw new Error('--poll-ms must be positive')
  return options
}

function makePorts() {
  const kernelPort = 60400 + Math.floor(Math.random() * 900)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
}

function log(name, details) {
  if (details === undefined) console.log(`[metaagent-code-fix-drill] ${name}`)
  else console.log(`[metaagent-code-fix-drill] ${name}`, JSON.stringify(details))
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

async function runChecked(command, args, options = {}) {
  const result = await run(command, args, options)
  if (result.code !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
  }
  return result
}

async function initGitWorktree(root) {
  await runChecked('git', ['init', '-b', 'main'], { cwd: root })
  await runChecked('git', ['config', 'user.email', 'metaagent-code-fix-drill@example.com'], { cwd: root })
  await runChecked('git', ['config', 'user.name', 'Metaagent Code Fix Drill'], { cwd: root })
}

async function writeFixture(workspace) {
  await mkdir(path.join(workspace, 'src'), { recursive: true })
  await mkdir(path.join(workspace, 'test'), { recursive: true })
  await writeFile(path.join(workspace, '.gitignore'), [
    '.arroba-wait.arroba',
    '.arrobaignore',
    '',
  ].join('\n'), 'utf8')
  await writeFile(path.join(workspace, 'package.json'), `${JSON.stringify({
    type: 'module',
    scripts: { test: 'node --test' },
  }, null, 2)}\n`, 'utf8')
  await writeFile(path.join(workspace, 'README.md'), [
    '# Todo Bug Fixture',
    '',
    'This tiny package has one failing test. The intended implementation is small; use `npm test` to verify the fix.',
    '',
  ].join('\n'), 'utf8')
  await writeFile(path.join(workspace, 'src', 'todo.mjs'), [
    'export function completeTodo(todos, id) {',
    '  return todos.map((todo) => todo.id === id ? { ...todo, done: false } : todo)',
    '}',
    '',
    'export function openTodos(todos) {',
    '  return todos.filter((todo) => !todo.done)',
    '}',
    '',
  ].join('\n'), 'utf8')
  await writeFile(path.join(workspace, 'test', 'todo.test.mjs'), [
    "import assert from 'node:assert/strict'",
    "import test from 'node:test'",
    "import { completeTodo, openTodos } from '../src/todo.mjs'",
    '',
    "test('completeTodo marks only the selected item done', () => {",
    "  const todos = [{ id: 'a', done: false }, { id: 'b', done: false }]",
    "  assert.deepEqual(completeTodo(todos, 'a'), [{ id: 'a', done: true }, { id: 'b', done: false }])",
    '})',
    '',
    "test('openTodos returns unfinished items', () => {",
    "  assert.deepEqual(openTodos([{ id: 'a', done: true }, { id: 'b', done: false }]), [{ id: 'b', done: false }])",
    '})',
    '',
  ].join('\n'), 'utf8')
}

async function buildKernel() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  await runChecked('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  const existing = await stat(binary).then((info) => info.isFile()).catch(() => false)
  if (!existing) throw new Error(`kernel build did not produce ${binary}`)
  return binary
}

async function waitForDaemon(shellBin, kernelUrl, workspace, scriptsDir, env) {
  const scriptPath = path.join(scriptsDir, 'wait.arroba')
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

function requireOutput(output, pattern, label) {
  if (!pattern.test(output)) {
    throw new Error(`missing ${label}: ${pattern}\n--- output ---\n${output}`)
  }
}

function unwrap(response, key) {
  return response?.[key] ?? response
}

function unwrapVariant(response, ...keys) {
  return keys.map((key) => response?.[key]).find((value) => value != null) ?? response
}

function assert(condition, message, details) {
  if (!condition) {
    throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
  }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function fileHash(text) {
  return createHash('sha256').update(text).digest('hex')
}

async function hashFile(file) {
  return fileHash(await readFile(file, 'utf8'))
}

async function gitChangedFiles(workspace) {
  const result = await runChecked('git', ['diff', '--name-only'], { cwd: workspace })
  return result.stdout.split(/\r?\n/).map((line) => line.trim()).filter(Boolean)
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
        entries.push({ file, line: index + 1, ...JSON.parse(line) })
      } catch {
        entries.push({ file, line: index + 1, parse_error: true, raw: line.slice(0, 300) })
      }
    }
  }
  return entries
}

function parseProviderToolText(text) {
  if (typeof text !== 'string' || !text.trim().startsWith('{')) return null
  try {
    return JSON.parse(text)
  } catch {
    return null
  }
}

function metaagentToolIsAllowed(toolName) {
  if (typeof toolName !== 'string') return false
  return toolName.startsWith('arroba.')
    || toolName.startsWith('mcp__arroba__')
    || toolName.startsWith('mcp__arroba.')
}

function listMetaagentEventsRequest(sessionId, metaagentId, limit = 100) {
  return {
    ListMetaagentEvents: {
      session_id: sessionId,
      metaagent_id: metaagentId,
      limit,
    },
  }
}

async function getSession(client, requests, sessionId) {
  return unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded').session
}

async function waitForProviderRun(client, requests, providerRunId, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    last = unwrap(await client.send(requests.getProviderRunRequest(providerRunId)), 'ProviderRun').provider_run
    if (last?.state === 'Running' || last?.state === 'Active' || last?.runtime_mcp_server_url) return last
    if (last?.state === 'Ended') throw new Error(`provider run ended before becoming active: ${JSON.stringify(last)}`)
    await sleep(pollMs)
  }
  throw new Error(`provider run did not become active: ${JSON.stringify(last)}`)
}

async function waitForAgentProviderRun(client, requests, sessionId, agentId, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastSession = null
  while (Date.now() < deadline) {
    lastSession = await getSession(client, requests, sessionId)
    const providerRunId = lastSession.active_provider_run_id
    if (providerRunId) {
      const run = unwrap(await client.send(requests.getProviderRunRequest(providerRunId)), 'ProviderRun').provider_run
      if (run?.agent_instance_id === agentId || run?.agent_id === agentId) {
        return await waitForProviderRun(client, requests, providerRunId, timeoutMs, pollMs)
      }
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for provider run for agent ${agentId}: ${JSON.stringify(lastSession)}`)
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

function summarizeEvent(event) {
  return {
    sequence: event.sequence,
    kind: event.kind,
    sourceAgentId: event.source_agent_id ?? null,
    delivery: event.prompt_delivery_status ?? null,
    title: event.title,
    summary: event.summary,
  }
}

async function observeUntilComplete({
  client,
  requests,
  sessionId,
  metaagentId,
  workspace,
  historyDir,
  beforeAgentIds,
  baselineSourceHash,
  baselineTestHash,
  options,
  env,
}) {
  const deadline = Date.now() + options.timeoutMs
  const seenEvents = new Set()
  let lastTaskKey = null
  let lastWorkerKey = null
  let lastDiffKey = null
  let testPassed = false
  let testResult = null
  let finalSession = null
  let finalEvents = []
  const seenHistoryTools = new Set()
  const workerHistoryToolEvidence = new Set()

  while (Date.now() < deadline) {
    const session = await getSession(client, requests, sessionId)
    finalSession = session
    const metaagent = (session.agents ?? []).find((agent) => agent.id === metaagentId)
    const task = (session.metaagent_tasks ?? []).find((entry) => entry.metaagent_id === metaagentId)
    const workers = (session.agents ?? []).filter((agent) => !beforeAgentIds.has(agent.id) && agent.id !== metaagentId)
    const workerIds = new Set(workers.map((agent) => agent.id))

    const taskKey = task
      ? `${task.status}:${task.revision}:${task.plan_markdown?.length ?? 0}:${task.completion_summary ?? ''}:${task.blocked_reason ?? ''}:${task.aborted_reason ?? ''}`
      : 'none'
    if (taskKey !== lastTaskKey) {
      lastTaskKey = taskKey
      log('task-observed', {
        status: task?.status ?? null,
        revision: task?.revision ?? null,
        planLength: task?.plan_markdown?.length ?? 0,
        completed: Boolean(task?.completed_at_ms),
        summary: task?.completion_summary ?? task?.blocked_reason ?? task?.aborted_reason ?? null,
      })
    }

    const workerKey = workers.map((agent) => `${agent.id}:${agent.alias ?? ''}:${agent.provider}:${agent.model ?? ''}`).sort().join('|')
    if (workerKey !== lastWorkerKey) {
      lastWorkerKey = workerKey
      log('workers-observed', workers.map((agent) => ({
        id: agent.id,
        alias: agent.alias ?? null,
        role: agent.role ?? null,
        provider: agent.provider,
        model: agent.model ?? null,
      })))
    }

    const historyEntries = await readHistoryEntries(historyDir)
    for (const entry of historyEntries) {
      if (entry.kind !== 'provider_tool') continue
      if (entry.agent_id !== metaagentId && !workerIds.has(entry.agent_id)) continue
      const tool = parseProviderToolText(entry.text)
      if (!tool?.tool) continue
      const historyKey = `${entry.file}:${entry.line}:${entry.agent_id}:${entry.merge_key ?? ''}:${tool.status ?? ''}`
      if (seenHistoryTools.has(historyKey)) continue
      seenHistoryTools.add(historyKey)
      if (workerIds.has(entry.agent_id) && tool.status === 'completed') {
        workerHistoryToolEvidence.add(historyKey)
      }
      if (entry.agent_id === metaagentId && !metaagentToolIsAllowed(tool.tool)) {
        throw new Error(`metaagent used disallowed provider-native tool ${tool.tool} at ${entry.file}:${entry.line}`)
      }
      log('history-tool-observed', {
        agentId: entry.agent_id,
        role: entry.agent_id === metaagentId ? 'meta' : 'worker',
        tool: tool.tool,
        status: tool.status ?? null,
        command: tool.input?.command ?? null,
        path: tool.input?.path ?? null,
      })
    }

    if (metaagent?.meta_mode) {
      const eventsPayload = unwrap(await client.send(listMetaagentEventsRequest(sessionId, metaagentId, 100)), 'MetaagentEventsListed')
      finalEvents = eventsPayload.events ?? []
      for (const event of finalEvents) {
        if (seenEvents.has(event.event_id)) continue
        seenEvents.add(event.event_id)
        log('metaagent-event', summarizeEvent(event))
      }
    }

    const changedFiles = await gitChangedFiles(workspace)
    const diffKey = changedFiles.join('|')
    if (diffKey !== lastDiffKey) {
      lastDiffKey = diffKey
      log('workspace-diff-observed', { changedFiles })
    }

    const currentSourceHash = await hashFile(path.join(workspace, 'src', 'todo.mjs'))
    const currentTestHash = await hashFile(path.join(workspace, 'test', 'todo.test.mjs'))
    if (currentSourceHash !== baselineSourceHash && currentTestHash === baselineTestHash && !testPassed) {
      testResult = await run('npm', ['test'], { cwd: workspace, env })
      if (testResult.code === 0) {
        testPassed = true
        log('tests-now-pass', { command: 'npm test' })
      } else {
        log('tests-still-failing', {
          code: testResult.code,
          stdoutTail: testResult.stdout.slice(-500),
          stderrTail: testResult.stderr.slice(-500),
        })
      }
    }

    if (task?.status === 'blocked' || task?.status === 'aborted') {
      throw new Error(`metaagent task ended as ${task.status}: ${task.blocked_reason ?? task.aborted_reason ?? 'no reason'}`)
    }

    const workerEventCount = finalEvents.filter((event) => event.source_agent_id && workerIds.has(event.source_agent_id)).length
    const workerEvidenceCount = workerEventCount + workerHistoryToolEvidence.size
    const sourceFixed = currentSourceHash !== baselineSourceHash
    const testsUnchanged = currentTestHash === baselineTestHash
    if (
      task?.status === 'completed'
      && workers.length > 0
      && workerEvidenceCount > 0
      && task.plan_markdown?.trim()
      && sourceFixed
      && testsUnchanged
      && testPassed
    ) {
      return {
        session,
        task,
        workers,
        events: finalEvents,
        workerEventCount,
        workerHistoryToolEvidenceCount: workerHistoryToolEvidence.size,
        testResult,
        changedFiles,
      }
    }

    await sleep(options.pollMs)
  }

  throw new Error(`timed out waiting for metaagent code-fix completion\nlast session=${JSON.stringify({
    task: finalSession?.metaagent_tasks?.find((entry) => entry.metaagent_id === metaagentId) ?? null,
    agents: finalSession?.agents?.map((agent) => ({ id: agent.id, alias: agent.alias, role: agent.role, provider: agent.provider })),
    events: finalEvents.map(summarizeEvent),
  }, null, 2)}`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-metaagent-code-fix-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home = path.join(rootDir, 'home')
  const scriptsDir = path.join(rootDir, 'scripts')
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const shellBin = path.join(repoRoot, 'apps/shell/dist/shell.js')
  const env = {
    ...process.env,
    HOME: process.env.HOME ?? home,
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: `metaagent-code-fix-drill-${process.pid}-${Date.now()}`,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
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

    const failing = await run('npm', ['test'], { cwd: workspace, env })
    assert(failing.code !== 0, 'fixture should start with one failing test', { stdout: failing.stdout, stderr: failing.stderr })
    await runChecked('git', ['add', '.'], { cwd: workspace })
    await runChecked('git', ['commit', '-m', 'Add failing todo fixture'], { cwd: workspace })

    const baselineSourceHash = await hashFile(path.join(workspace, 'src', 'todo.mjs'))
    const baselineTestHash = await hashFile(path.join(workspace, 'test', 'todo.test.mjs'))

    const kernelBinary = await buildKernel()
    daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ['ignore', 'ignore', 'inherit'] })
    await waitForDaemon(shellBin, kernelUrl, workspace, scriptsDir, env)
    log('daemon-ready', { kernelUrl, provider: options.provider, model: options.model, effort: options.effort })

    const setupScript = path.join(scriptsDir, 'setup.arroba')
    await writeFile(setupScript, [
      `set provider ${options.provider}`,
      `set model ${options.model}`,
      `set effort ${options.effort}`,
      'session new $workspace as session',
      'session mode build',
      'session permissions yolo',
      'agent list',
    ].join('\n'), 'utf8')
    const setup = await run(process.execPath, [
      shellBin,
      'run',
      setupScript,
      '--kernel-url',
      kernelUrl,
      '--workspace',
      workspace,
      '--worktree',
      workspace,
      '--var',
      `workspace=${workspace}`,
    ], { env })
    if (setup.code !== 0) throw new Error(`setup script failed\nstdout:\n${setup.stdout}\nstderr:\n${setup.stderr}`)
    sessionId = setup.stdout.match(/bound \$session = (\S+)/)?.[1] ?? null
    assert(sessionId, 'setup script did not bind session id', { stdout: setup.stdout })

    const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
    const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')
    client = new LocalIpcClient(kernelUrl)
    const attachment = unwrap(await client.send(requests.attachToSessionRequest(sessionId, `metaagent-code-fix-drill-${Date.now()}`)), 'SessionAttached').attachment
    let initialSession = await getSession(client, requests, sessionId)
    let metaagent = (initialSession.agents ?? []).find((agent) => agent.id === initialSession.focused_agent_id) ?? (initialSession.agents ?? [])[0]
    assert(metaagent, 'session should contain a default regular agent', initialSession)
    assert(!metaagent.meta_mode, 'default agent must start outside meta mode', metaagent)
    await client.send(requests.updateAgentProfileRequest({
      sessionId,
      agentId: metaagent.id,
      provider: options.provider,
      model: options.model,
      effort: options.effort,
    }))
    initialSession = await getSession(client, requests, sessionId)
    metaagent = (initialSession.agents ?? []).find((agent) => agent.id === metaagent.id)
    assert(
      metaagent?.provider === options.provider && metaagent?.model === options.model && metaagent?.effort === options.effort,
      'default agent profile should match requested drill provider/model/effort before /meta',
      { metaagent, expected: { provider: options.provider, model: options.model, effort: options.effort } },
    )
    const beforeAgentIds = new Set((initialSession.agents ?? []).map((agent) => agent.id))

    const metaPrompt = `/meta ${USER_PROMPT}`
    await client.send(requests.submitPromptRequest(sessionId, attachment.id, metaagent.id, metaPrompt, []))
    log('single-prompt-submitted', { metaagentId: metaagent.id, prompt: metaPrompt })

    const metaRun = await waitForAgentProviderRun(client, requests, sessionId, metaagent.id, options.timeoutMs, options.pollMs)
    assert(metaRun.adapter_key !== 'dev-stub' && metaRun.provider !== 'dev-stub', 'metaagent must run on a real provider', metaRun)
    assert(metaRun.execution_mode === 'plan', 'meta-mode provider run must be forced to plan mode', metaRun)
    const metaSession = await getSession(client, requests, sessionId)
    const metaModeAgent = (metaSession.agents ?? []).find((agent) => agent.id === metaagent.id)
    assert(metaModeAgent?.meta_mode, 'same regular agent should enter meta mode after /meta prompt', metaModeAgent)
    log('metaagent-run-observed', {
      providerRunId: metaRun.id,
      provider: metaRun.provider,
      adapterKey: metaRun.adapter_key,
      executionMode: metaRun.execution_mode,
      permissionLevel: metaRun.permission_level ?? null,
    })

    const observed = await observeUntilComplete({
      client,
      requests,
      sessionId,
      metaagentId: metaagent.id,
      workspace,
      historyDir: env.ARROBA_SESSION_HISTORY_DIR,
      beforeAgentIds,
      baselineSourceHash,
      baselineTestHash,
      options,
      env,
    })

    const fixedSource = await readFile(path.join(workspace, 'src', 'todo.mjs'), 'utf8')
    assert(fixedSource.includes('done: true'), 'source fix should mark the selected todo done', fixedSource)
    assert(observed.changedFiles.includes('src/todo.mjs'), 'source file must be changed', observed.changedFiles)
    assert(!observed.changedFiles.includes('test/todo.test.mjs'), 'test file must not be changed', observed.changedFiles)

    console.log(JSON.stringify({
      status: 'ok',
      mode: 'metaagent-code-fix-drill',
      kernelUrl,
      sessionId,
      metaagentId: metaagent.id,
      provider: options.provider,
      model: options.model,
      effort: options.effort,
      promptCount: 1,
      harnessRuntimeMcpCallsAfterPrompt: 0,
      harnessWorkspaceWritesAfterPrompt: 0,
      workerIds: observed.workers.map((agent) => agent.id),
      workerAliases: observed.workers.map((agent) => agent.alias),
      workerEventCount: observed.workerEventCount,
      workerHistoryToolEvidenceCount: observed.workerHistoryToolEvidenceCount,
      taskStatus: observed.task.status,
      planLength: observed.task.plan_markdown.length,
      changedFiles: observed.changedFiles,
      verifiedCommand: 'npm test',
      metaagentEventCount: observed.events.length,
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
        drill: 'metaagent-code-fix',
        provider: options.provider,
        model: options.model,
        effort: options.effort,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        kernelUrl,
        sessionId,
        workspace,
      },
      log,
    })
  }
  log('passed')
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack ?? error.message : error)
  process.exit(1)
})
