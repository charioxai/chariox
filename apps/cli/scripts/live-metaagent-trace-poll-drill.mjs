#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { createWriteStream } from 'node:fs'
import { mkdir, readFile, readdir, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const DEFAULT_TIMEOUT_MS = 600_000
const DEFAULT_POLL_MS = 1_000
const DEFAULT_PROVIDER = process.env.ARROBA_METAAGENT_TRACE_POLL_PROVIDER ?? 'codex'
const DEFAULT_MODEL = process.env.ARROBA_METAAGENT_TRACE_POLL_MODEL ?? 'gpt-5.5'
const DEFAULT_EFFORT = process.env.ARROBA_METAAGENT_TRACE_POLL_EFFORT ?? 'medium'
const TRACE_PHRASE = 'TRACE_POLL_DRILL_WORKER_VISIBLE'
const USER_PROMPT = [
  'Spawn one regular worker agent for a tiny supervision check.',
  '',
  'Use the session default model when spawning the worker; do not pass an explicit model.',
  'Before prompting the worker, subscribe to that worker live trace with `arroba.meta.subscribe_trace`.',
  `Ask the worker to inspect this repo and include the exact phrase ${TRACE_PHRASE} in its response.`,
  'Call `arroba.meta.wait_trace` until you can see worker-generated output, not just a prompt echo.',
  'Then complete this metaagent task with a concise summary of the worker result and the trace evidence you observed.',
].join('\n')

function parseArgs(argv) {
  const options = {
    provider: DEFAULT_PROVIDER,
    model: DEFAULT_MODEL,
    effort: DEFAULT_EFFORT,
    accountProfile: 'default',
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    keepArtifactsOnFailure: true,
    preserveOnSuccess: false,
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
    else if (arg === '--help' || arg === '-h') {
      console.log([
        'Usage: node apps/cli/scripts/live-metaagent-trace-poll-drill.mjs [options]',
        '',
        'Runs a real-provider metaagent drill that validates live worker trace polling.',
      ].join('\n'))
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  if (!options.provider || options.provider === 'dev-stub') {
    throw new Error('trace poll drill requires a real provider; dev-stub is not valid evidence')
  }
  if (!options.model) throw new Error('--model is required')
  return options
}

function makePorts() {
  const kernelPort = 62800 + Math.floor(Math.random() * 700)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
}

function log(name, details) {
  if (details === undefined) console.log(`[metaagent-trace-poll-drill] ${name}`)
  else console.log(`[metaagent-trace-poll-drill] ${name}`, JSON.stringify(details))
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

async function initGitWorktree(root) {
  await runChecked('git', ['init', '-b', 'main'], { cwd: root })
  await runChecked('git', ['config', 'user.email', 'metaagent-trace-poll-drill@example.com'], { cwd: root })
  await runChecked('git', ['config', 'user.name', 'Metaagent Trace Poll Drill'], { cwd: root })
}

async function writeFixture(workspace) {
  await writeFile(path.join(workspace, 'README.md'), [
    '# Trace Poll Drill',
    '',
    `The worker should report ${TRACE_PHRASE}.`,
    '',
  ].join('\n'), 'utf8')
  await writeFile(path.join(workspace, '.gitignore'), '.arroba-wait.arroba\n.arrobaignore\n', 'utf8')
}

async function buildKernel() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  await runChecked('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  const exists = await stat(binary).then((info) => info.isFile()).catch(() => false)
  if (!exists) throw new Error(`kernel build did not produce ${binary}`)
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

function parseToolOutput(output) {
  if (typeof output !== 'string') return null
  try {
    return JSON.parse(output)
  } catch {
    return null
  }
}

function traceItems(tool) {
  if (!tool?.tool?.match(/(?:poll_trace|wait_trace)$/) || tool.status !== 'completed') return []
  const output = parseToolOutput(tool.output)
  return output?.structuredContent?.items ?? []
}

function traceItemContainsWorkerPhrase(item) {
  if (!item || item.kind === 'prompt_echo') return false
  const text = [item.text, item.summary, item.excerpt].filter(Boolean).join('\n')
  return text.includes(TRACE_PHRASE)
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
    if (last?.state === 'Ended') throw new Error(`provider run ended before active: ${JSON.stringify(last)}`)
    await sleep(pollMs)
  }
  throw new Error(`provider run did not become active: ${JSON.stringify(last)}`)
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

async function observe({ client, requests, sessionId, metaagentId, historyDir, beforeAgentIds, timeoutMs, pollMs }) {
  const deadline = Date.now() + timeoutMs
  const seenTools = new Set()
  let sawSubscribeTrace = false
  let sawTraceWait = false
  let sawTracePhrase = false
  let finalTask = null
  let workers = []
  while (Date.now() < deadline) {
    const session = await getSession(client, requests, sessionId)
    finalTask = (session.metaagent_tasks ?? []).find((entry) => entry.metaagent_id === metaagentId) ?? null
    workers = (session.agents ?? []).filter((agent) => !beforeAgentIds.has(agent.id) && agent.id !== metaagentId && agent.role !== 'meta')
    if (workers.length > 0) {
      log('workers-observed', workers.map((agent) => ({ id: agent.id, alias: agent.alias ?? null, provider: agent.provider })))
    }
    for (const entry of await readHistoryEntries(historyDir)) {
      if (entry.kind !== 'provider_tool' || entry.agent_id !== metaagentId) continue
      const tool = parseTool(entry.text)
      if (!tool?.tool) continue
      const key = `${entry.file}:${entry.line}:${tool.tool}:${tool.status ?? ''}`
      if (seenTools.has(key)) continue
      seenTools.add(key)
      log('metaagent-tool-observed', {
        tool: tool.tool,
        status: tool.status ?? null,
        summary: JSON.stringify(tool).slice(0, 300),
      })
      if (tool.tool.includes('subscribe_trace')) sawSubscribeTrace = true
      if (tool.tool.includes('wait_trace') || tool.tool.includes('poll_trace')) {
        if (tool.tool.includes('wait_trace')) sawTraceWait = true
        if (traceItems(tool).some(traceItemContainsWorkerPhrase)) sawTracePhrase = true
      }
    }
    if (finalTask?.status) {
      log('task-observed', {
        status: finalTask.status,
        summary: finalTask.completion_summary ?? finalTask.blocked_reason ?? null,
      })
    }
    if (finalTask?.status === 'blocked' || finalTask?.status === 'aborted') {
      throw new Error(`metaagent task ended as ${finalTask.status}: ${finalTask.blocked_reason ?? finalTask.aborted_reason ?? 'no reason'}`)
    }
    if (finalTask?.status === 'completed' && workers.length > 0 && sawSubscribeTrace && sawTraceWait && sawTracePhrase) {
      return { task: finalTask, workers, sawSubscribeTrace, sawTraceWait, sawTracePhrase }
    }
    if (finalTask?.status === 'completed') {
      throw new Error(`metaagent task completed without validated worker trace output: ${JSON.stringify({
        workers: workers.map((agent) => ({ id: agent.id, alias: agent.alias })),
        sawSubscribeTrace,
        sawTraceWait,
        sawTracePhrase,
        summary: finalTask.completion_summary ?? null,
      }, null, 2)}`)
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for trace poll validation: ${JSON.stringify({
    task: finalTask,
    workers: workers.map((agent) => ({ id: agent.id, alias: agent.alias })),
    sawSubscribeTrace,
    sawTraceWait,
    sawTracePhrase,
  }, null, 2)}`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-metaagent-trace-poll-drill', `${process.pid}-${Date.now()}`)
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
    ARROBA_DAEMON_ID: `metaagent-trace-poll-drill-${process.pid}-${Date.now()}`,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    ARROBA_LOG_DIR: path.join(rootDir, 'logs'),
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
    await runChecked('git', ['add', '.'], { cwd: workspace })
    await runChecked('git', ['commit', '-m', 'Add trace poll fixture'], { cwd: workspace })

    const kernelBinary = await buildKernel()
    const kernelStdout = createWriteStream(path.join(rootDir, 'kernel.stdout.log'), { flags: 'a' })
    const kernelStderr = createWriteStream(path.join(rootDir, 'kernel.stderr.log'), { flags: 'a' })
    daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ['ignore', 'pipe', 'pipe'] })
    daemon.stdout.pipe(kernelStdout)
    daemon.stderr.pipe(kernelStderr)
    daemon.once('exit', (code, signal) => {
      log('daemon-exited', { code, signal })
    })
    await waitForDaemon(shellBin, kernelUrl, workspace, scriptsDir, env)
    log('daemon-ready', { kernelUrl, provider: options.provider, model: options.model, effort: options.effort })

    const setupScript = path.join(scriptsDir, 'setup.arroba')
    await writeFile(setupScript, [
      `set provider ${options.provider}`,
      `set model ${options.model}`,
      `set effort ${options.effort}`,
      'session new --meta $workspace as session',
      'workspace sync off',
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
    if (setup.code !== 0) throw new Error(`setup failed\nstdout:\n${setup.stdout}\nstderr:\n${setup.stderr}`)
    sessionId = setup.stdout.match(/bound \$session = (\S+)/)?.[1] ?? null
    assert(sessionId, 'setup script did not bind session id', { stdout: setup.stdout })

    const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
    const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')
    client = new LocalIpcClient(kernelUrl, {
      kernelMaxMissedPongs: Math.max(120, Math.ceil(options.timeoutMs / 5_000)),
    })
    const attachment = unwrap(await client.send(requests.attachToSessionRequest(sessionId, `metaagent-trace-poll-drill-${Date.now()}`)), 'SessionAttached').attachment
    const initialSession = await getSession(client, requests, sessionId)
    const metaagent = (initialSession.agents ?? []).find((agent) => agent.role === 'meta')
    assert(metaagent, 'session should contain a metaagent', initialSession)
    const beforeAgentIds = new Set((initialSession.agents ?? []).map((agent) => agent.id))

    const launched = unwrapVariant(
      await client.send(requests.launchProviderRunRequest(
        sessionId,
        options.provider,
        options.accountProfile,
        options.model,
        options.effort,
        metaagent.id,
      )),
      'ProviderRunLaunched',
      'ProviderRunLaunchAccepted',
    )
    const metaRun = await waitForProviderRun(client, requests, launched.provider_run.id, options.timeoutMs, options.pollMs)
    assert(metaRun.adapter_key !== 'dev-stub' && metaRun.provider !== 'dev-stub', 'metaagent must run on a real provider', metaRun)
    assert(metaRun.execution_mode === 'plan', 'metaagent provider run must be plan mode', metaRun)
    log('metaagent-run-observed', { providerRunId: metaRun.id, executionMode: metaRun.execution_mode })

    await client.send(requests.submitPromptRequest(sessionId, attachment.id, metaagent.id, USER_PROMPT, []))
    log('single-prompt-submitted', { metaagentId: metaagent.id, prompt: USER_PROMPT })

    const observed = await observe({
      client,
      requests,
      sessionId,
      metaagentId: metaagent.id,
      historyDir: env.ARROBA_SESSION_HISTORY_DIR,
      beforeAgentIds,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    console.log(JSON.stringify({
      status: 'ok',
      mode: 'metaagent-trace-poll-drill',
      sessionId,
      metaagentId: metaagent.id,
      provider: options.provider,
      model: options.model,
      promptCount: 1,
      workerIds: observed.workers.map((agent) => agent.id),
      taskStatus: observed.task.status,
      sawSubscribeTrace: observed.sawSubscribeTrace,
      sawTraceWait: observed.sawTraceWait,
      sawTracePhrase: observed.sawTracePhrase,
    }, null, 2))
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client) await client.close().catch(() => {})
    await cleanupSession(kernelUrl, sessionId)
    await terminateChild(daemon)
    if (succeeded && options.preserveOnSuccess) {
      log('preserved-successful-run', { rootDir })
    } else {
      await finalizeDrillArtifacts({
        rootDir,
        passed: succeeded,
        preserveOnFailure: options.keepArtifactsOnFailure,
        failure,
        metadata: {
          drill: 'metaagent-trace-poll',
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
  log('passed')
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack ?? error.message : error)
  process.exit(1)
})
