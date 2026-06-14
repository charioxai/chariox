#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { mkdir, readFile, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const DEFAULT_TIMEOUT_MS = 75_000
const DEFAULT_POLL_MS = 250

function parseArgs(argv) {
  const options = {
    keepArtifactsOnFailure: false,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--') continue
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++index])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++index])
    else if (arg === '--help' || arg === '-h') {
      console.log([
        'Usage: node apps/cli/scripts/live-metaagent-autonomous-todo-drill.mjs [options]',
        '',
        'Runs a deterministic one-prompt metaagent drill:',
        '- creates a tiny failing JavaScript todo project',
        '- submits one high-level prompt to the metaagent',
        '- verifies task creation, planning, worker delegation, and completion',
        '- verifies the code fix is represented as worker output and tests pass',
        '',
        'Options:',
        `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
        `  --poll-ms ${DEFAULT_POLL_MS}`,
        '  --keep-artifacts-on-failure',
      ].join('\n'))
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  return options
}

function makePorts() {
  const kernelPort = 59500 + Math.floor(Math.random() * 1000)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
}

function log(name, details) {
  if (details === undefined) console.log(`[metaagent-autonomous-todo-drill] ${name}`)
  else console.log(`[metaagent-autonomous-todo-drill] ${name}`, JSON.stringify(details))
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
  await runChecked('git', ['config', 'user.email', 'metaagent-autonomous-todo-drill@example.com'], { cwd: root })
  await runChecked('git', ['config', 'user.name', 'Metaagent Autonomous Todo Drill'], { cwd: root })
}

async function writeTodoFixture(workspace) {
  await mkdir(path.join(workspace, 'src'), { recursive: true })
  await mkdir(path.join(workspace, 'test'), { recursive: true })
  await writeFile(path.join(workspace, 'package.json'), JSON.stringify({
    type: 'module',
    scripts: { test: 'node --test' },
  }, null, 2), 'utf8')
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
  const existing = await stat(binary).then((info) => info.isFile()).catch(() => false)
  if (existing) return binary
  await runChecked('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  return binary
}

async function waitForDaemon(shellBin, kernelUrl, workspace, env) {
  const scriptPath = path.join(workspace, 'wait.arroba')
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

async function launchRuntime(client, requests, sessionId, agentId, model, timeoutMs, pollMs) {
  const launched = unwrapVariant(
    await client.send(requests.launchProviderRunRequest(sessionId, 'dev-stub', 'default', model, 'low', agentId)),
    'ProviderRunLaunched',
    'ProviderRunLaunchAccepted',
  )
  const providerRun = launched.provider_run
  if (!providerRun?.id) throw new Error(`launch did not return provider run: ${JSON.stringify(launched)}`)
  const deadline = Date.now() + timeoutMs
  let last = providerRun
  while (Date.now() < deadline) {
    last = unwrap(await client.send(requests.getProviderRunRequest(providerRun.id)), 'ProviderRun').provider_run
    if (last?.runtime_mcp_server_url && last?.runtime_mcp_auth_token) return last
    if (last?.state === 'Ended') throw new Error(`provider run ended before exposing runtime MCP: ${JSON.stringify(last)}`)
    await sleep(pollMs)
  }
  throw new Error(`provider run did not expose runtime MCP binding: ${JSON.stringify(last)}`)
}

async function callRuntimeMcp(providerRun, method, params = {}) {
  const response = await fetch(providerRun.runtime_mcp_server_url, {
    method: 'POST',
    headers: {
      authorization: `Bearer ${providerRun.runtime_mcp_auth_token}`,
      'content-type': 'application/json',
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: `${method}-${Date.now()}-${Math.random().toString(16).slice(2)}`,
      method,
      params,
    }),
  })
  const text = await response.text()
  const json = JSON.parse(text)
  if (!response.ok || json.error) throw new Error(`runtime MCP ${method} failed: ${text}`)
  return json.result
}

async function callRuntimeTool(providerRun, name, args = {}) {
  const result = await callRuntimeMcp(providerRun, 'tools/call', {
    name,
    arguments: args,
  })
  return {
    ok: !result.isError,
    payload: result.structuredContent,
    raw: result,
  }
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

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-metaagent-autonomous-todo-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const home = path.join(rootDir, 'home')
  const scriptsDir = path.join(rootDir, 'scripts')
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const shellBin = path.join(repoRoot, 'apps/shell/dist/shell.js')
  const env = {
    ...process.env,
    HOME: home,
    ARROBA_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_MCP_PORT: String(ports.mcpPort),
    ARROBA_OPENCODE_PORT: String(ports.opencodePort),
    ARROBA_CODEX_PORT: String(ports.codexPort),
    ARROBA_DAEMON_ID: `metaagent-autonomous-todo-drill-${process.pid}-${Date.now()}`,
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
    await writeTodoFixture(workspace)
    await initGitWorktree(workspace)

    const failing = await run('npm', ['test'], { cwd: workspace, env })
    assert(failing.code !== 0, 'fixture should start with one failing test', { stdout: failing.stdout, stderr: failing.stderr })

    const kernelBinary = await buildKernel()
    daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ['ignore', 'ignore', 'inherit'] })
    await waitForDaemon(shellBin, kernelUrl, workspace, env)
    log('daemon-ready', { kernelUrl })

    const setupScript = path.join(scriptsDir, 'setup.arroba')
    await writeFile(setupScript, [
      'set provider dev-stub',
      'set model metaagent-autonomous-todo-default',
      'session new --meta $workspace as session',
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
    requireOutput(setup.stdout, /created metaagent session /, 'metaagent session creation')
    sessionId = setup.stdout.match(/bound \$session = (\S+)/)?.[1] ?? null
    assert(sessionId, 'setup script did not bind session id', { stdout: setup.stdout })

    const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
    const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')
    client = new LocalIpcClient(kernelUrl)
    const attachment = unwrap(await client.send(requests.attachToSessionRequest(sessionId, `metaagent-autonomous-todo-drill-${Date.now()}`)), 'SessionAttached').attachment

    let sessionState = unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded').session
    const metaagent = (sessionState.agents ?? []).find((agent) => agent.role === 'meta')
    assert(metaagent, 'session should contain a metaagent', sessionState)
    const metaRun = await launchRuntime(client, requests, sessionId, metaagent.id, 'metaagent-autonomous-todo-meta', options.timeoutMs, options.pollMs)
    assert(metaRun.execution_mode === 'plan', 'metaagent provider run must be forced to plan mode', { metaRun })

    const userPrompt = 'The repo has a small failing JavaScript project. Figure out what is wrong, organize the work with regular agents, and get the project to a passing state. Report back with what changed and how you verified it.'
    await client.send(requests.submitPromptRequest(sessionId, attachment.id, metaagent.id, userPrompt, []))
    sessionState = unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded').session
    const task = (sessionState.metaagent_tasks ?? []).find((entry) => entry.metaagent_id === metaagent.id)
    assert(task?.task_markdown === userPrompt && task.status === 'active', 'one prompt should create an active metaagent task', sessionState.metaagent_tasks)

    const readPackage = await callRuntimeTool(metaRun, 'arroba.read_artifact', { path: 'package.json' })
    assert(readPackage.ok, 'metaagent should read package.json while planning', readPackage.payload)
    const readSource = await callRuntimeTool(metaRun, 'arroba.read_artifact', { path: 'src/todo.mjs' })
    assert(readSource.ok, 'metaagent should read source while planning', readSource.payload)
    const plan = await callRuntimeTool(metaRun, 'arroba.meta.update_plan', {
      markdown: '- Inspect failing test\n- Delegate implementation fix to worker\n- Delegate verification to QA\n- Complete when npm test passes',
    })
    assert(plan.ok, 'metaagent should maintain a plan document', plan.payload)

    const fixerSpawn = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: 'agent spawn fixer todo-fixer-worker' })
    assert(fixerSpawn.ok, 'metaagent should spawn a fixer worker', fixerSpawn.payload)
    const qaSpawn = await callRuntimeTool(metaRun, 'arroba.meta.run_command', { command: 'agent spawn qa todo-qa-worker' })
    assert(qaSpawn.ok, 'metaagent should spawn a QA worker', qaSpawn.payload)
    const promptFixer = await callRuntimeTool(metaRun, 'arroba.meta.run_command', {
      command: 'prompt fixer "Find the failing todo test and implement the smallest code fix. Report changed files."',
    })
    assert(promptFixer.ok, 'metaagent should delegate implementation to fixer worker', promptFixer.payload)

    sessionState = unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded').session
    const fixer = (sessionState.agents ?? []).find((agent) => agent.alias === 'fixer')
    const qa = (sessionState.agents ?? []).find((agent) => agent.alias === 'qa')
    assert(fixer?.role !== 'meta' && qa?.role !== 'meta', 'delegated agents should be regular workers', { fixer, qa })
    const fixerRun = await launchRuntime(client, requests, sessionId, fixer.id, 'todo-fixer-worker', options.timeoutMs, options.pollMs)

    await writeFile(path.join(workspace, 'src', 'todo.mjs'), [
      'export function completeTodo(todos, id) {',
      '  return todos.map((todo) => todo.id === id ? { ...todo, done: true } : todo)',
      '}',
      '',
      'export function openTodos(todos) {',
      '  return todos.filter((todo) => !todo.done)',
      '}',
      '',
    ].join('\n'), 'utf8')
    await client.send(requests.appendNativeProviderOutputRequest(
      sessionId,
      attachment.id,
      fixerRun.id,
      'provider_tool',
      JSON.stringify({
        tool: 'workspace_write',
        status: 'completed',
        output: 'Worker fixer changed src/todo.mjs so completeTodo marks the selected todo done.',
      }),
      'metaagent-autonomous-todo-worker-output',
    ))

    const promptQa = await callRuntimeTool(metaRun, 'arroba.meta.run_command', {
      command: 'prompt qa "Run npm test and report whether the todo project passes."',
    })
    assert(promptQa.ok, 'metaagent should delegate verification to QA worker', promptQa.payload)
    const passing = await run('npm', ['test'], { cwd: workspace, env })
    assert(passing.code === 0, 'todo project should pass after worker fix', { stdout: passing.stdout, stderr: passing.stderr })
    const fixedSource = await readFile(path.join(workspace, 'src', 'todo.mjs'), 'utf8')
    assert(fixedSource.includes('done: true'), 'source fix should be present in worker-touched file', fixedSource)

    const complete = await callRuntimeTool(metaRun, 'arroba.meta.complete_task', {
      summary: 'Fixed completeTodo and verified with npm test.',
    })
    assert(complete.ok && complete.payload?.status === 'completed', 'metaagent should mark task completed after verification', complete.payload)

    console.log(JSON.stringify({
      status: 'ok',
      mode: 'metaagent-autonomous-todo-drill',
      kernelUrl,
      sessionId,
      metaagentId: metaagent.id,
      workerIds: [fixer.id, qa.id],
      promptCount: 1,
      verifiedCommand: 'npm test',
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
      failure,
      metadata: {
        drill: 'metaagent-autonomous-todo',
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
