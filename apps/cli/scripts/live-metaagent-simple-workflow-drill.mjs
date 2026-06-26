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
const DEFAULT_PROVIDER = process.env.ARROBA_METAAGENT_SIMPLE_WORKFLOW_PROVIDER ?? 'codex'
const DEFAULT_MODEL = process.env.ARROBA_METAAGENT_SIMPLE_WORKFLOW_MODEL ?? 'gpt-5.5'
const DEFAULT_EFFORT = process.env.ARROBA_METAAGENT_SIMPLE_WORKFLOW_EFFORT ?? 'medium'
const MARKER = 'SIMPLE_WORKFLOW_DRILL_OK'
const RESULT_FILE = 'workflow-result.txt'
const logPrefix = 'metaagent-simple-workflow-drill'

const TASK_PROMPT = [
  'Use a minimal workflow suitable for this small task.',
  '',
  `The workflow should produce a file named ${RESULT_FILE} containing the exact marker ${MARKER} and one or two sentences explaining what this fixture repo is for.`,
  '',
  'A one-node workflow is enough if you think that fits. Build the result by running the workflow, then inspect the workflow/run result and complete this metaagent task only after the local validation passes.',
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
    preserveOnSuccess: true,
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
        'Usage: node apps/cli/scripts/live-metaagent-simple-workflow-drill.mjs [options]',
        '',
        'Runs an observe-only live metaagent drill for a minimal workflow task:',
        '- creates a tiny repo whose validation fails until workflow-result.txt exists',
        '- starts a real metaagent in plan mode',
        '- submits exactly one high-level task prompt',
        '- passes only if the metaagent creates and runs a workflow, reviews it, and completes after validation passes',
        '',
        `  --provider ${DEFAULT_PROVIDER}`,
        `  --model ${DEFAULT_MODEL}`,
        `  --effort ${DEFAULT_EFFORT}`,
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
    throw new Error('simple workflow metaagent drill requires a real provider; dev-stub is not valid evidence')
  }
  if (!options.model) throw new Error('--model is required')
  if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) throw new Error('--timeout-ms must be positive')
  if (!Number.isFinite(options.pollMs) || options.pollMs <= 0) throw new Error('--poll-ms must be positive')
  return options
}

function makePorts() {
  const kernelPort = 61900 + Math.floor(Math.random() * 700)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
}

function log(name, details) {
  if (details === undefined) console.log(`[${logPrefix}] ${name}`)
  else console.log(`[${logPrefix}] ${name}`, JSON.stringify(details))
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

function requireOutput(output, pattern, label) {
  if (!pattern.test(output)) throw new Error(`missing ${label}: ${pattern}\n--- output ---\n${output}`)
}

function unwrap(response, key) {
  return response?.[key] ?? response
}

function unwrapVariant(response, ...keys) {
  return keys.map((key) => response?.[key]).find((value) => value != null) ?? response
}

function fileHash(text) {
  return createHash('sha256').update(text).digest('hex')
}

async function hashFile(file) {
  return fileHash(await readFile(file, 'utf8').catch(() => ''))
}

async function initGitWorktree(root) {
  await runChecked('git', ['init', '-b', 'main'], { cwd: root })
  await runChecked('git', ['config', 'user.email', 'metaagent-simple-workflow-drill@example.com'], { cwd: root })
  await runChecked('git', ['config', 'user.name', 'Metaagent Simple Workflow Drill'], { cwd: root })
}

async function writeFixture(workspace) {
  await mkdir(path.join(workspace, 'scripts'), { recursive: true })
  await mkdir(path.join(workspace, 'test'), { recursive: true })
  await writeFile(path.join(workspace, '.gitignore'), [
    '.arroba-wait.arroba',
    '.arrobaignore',
    'node_modules/',
    '',
  ].join('\n'), 'utf8')
  await writeFile(path.join(workspace, 'README.md'), [
    '# Simple Workflow Drill Fixture',
    '',
    `A workflow worker should create \`${RESULT_FILE}\` with marker \`${MARKER}\`.`,
    '',
  ].join('\n'), 'utf8')
  await writeFile(path.join(workspace, 'package.json'), `${JSON.stringify({
    type: 'module',
    scripts: {
      build: 'node scripts/validate-result.mjs',
      test: 'node --test',
    },
    dependencies: {},
    devDependencies: {},
  }, null, 2)}\n`, 'utf8')
  await writeFile(path.join(workspace, 'scripts', 'validate-result.mjs'), [
    "import assert from 'node:assert/strict'",
    "import { readFile, stat } from 'node:fs/promises'",
    '',
    `const resultFile = '${RESULT_FILE}'`,
    `const marker = '${MARKER}'`,
    'const info = await stat(resultFile).catch(() => null)',
    'assert.ok(info?.isFile(), `${resultFile} must exist`)',
    "const text = await readFile(resultFile, 'utf8')",
    'assert.match(text, new RegExp(marker), `result file must include ${marker}`)',
    "assert.match(text, /workflow/i, 'result should mention workflow context')",
    "assert.ok(text.trim().split(/\\s+/).length >= 8, 'result should contain a short explanation')",
    '',
  ].join('\n'), 'utf8')
  await writeFile(path.join(workspace, 'test', 'workflow-result.test.mjs'), [
    "import assert from 'node:assert/strict'",
    "import { readFile } from 'node:fs/promises'",
    "import test from 'node:test'",
    '',
    "test('workflow result marker exists', async () => {",
    `  const text = await readFile('${RESULT_FILE}', 'utf8')`,
    `  assert.match(text, /${MARKER}/)`,
    "  assert.match(text, /fixture|repo|repository/i)",
    "  assert.match(text, /workflow/i)",
    '})',
    '',
  ].join('\n'), 'utf8')
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

async function gitChangedFiles(workspace) {
  const result = await runChecked('git', ['status', '--porcelain', '-uall'], { cwd: workspace })
  return result.stdout
    .split(/\r?\n/)
    .filter(Boolean)
    .map((line) => line.replace(/^.. ?/, '').trim())
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

function commandHits(commandText, pattern) {
  return typeof commandText === 'string' && pattern.test(commandText)
}

function metaagentToolIsAllowed(toolName) {
  if (typeof toolName !== 'string') return false
  return toolName.startsWith('arroba.')
    || toolName.startsWith('mcp__arroba__')
    || toolName.startsWith('mcp__arroba.')
}

function metaagentToolIsDirectExecution(toolName) {
  return /(?:^|\.|__)(write_artifact|edit_artifact|delete_artifact|exec|shell|bash|apply_patch)$/.test(toolName)
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
    if (last?.state === 'Running' || last?.state === 'Active') return last
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

function workflowRunIsCompleted(run) {
  return String(run?.status ?? '').toLowerCase() === 'completed'
}

async function observeUntilComplete({
  client,
  requests,
  sessionId,
  metaagentId,
  workspace,
  historyDir,
  beforeAgentIds,
  baselineHash,
  options,
  env,
}) {
  const deadline = Date.now() + options.timeoutMs
  const seenEvents = new Set()
  const seenHistoryTools = new Set()
  const workerHistoryToolEvidence = new Set()
  const workflowEvidence = {
    created: false,
    nodeAddCount: 0,
    endpointCreated: false,
    run: false,
    runInspected: false,
  }
  const commandDiscoveryEvidence = {
    searched: false,
    docsRead: false,
  }
  let lastTaskKey = null
  let lastWorkerKey = null
  let lastDiffKey = null
  let lastRunKey = null
  let lastValidationDiffKey = null
  let buildPassed = false
  let testPassed = false
  let buildResult = null
  let testResult = null
  let finalSession = null
  let finalEvents = []

  while (Date.now() < deadline) {
    const session = await getSession(client, requests, sessionId)
    finalSession = session
    const metaagent = (session.agents ?? []).find((agent) => agent.id === metaagentId)
    const task = (session.metaagent_tasks ?? []).find((entry) => entry.metaagent_id === metaagentId)
    const workers = (session.agents ?? []).filter((agent) => !beforeAgentIds.has(agent.id) && agent.id !== metaagentId)
    const workerIds = new Set(workers.map((agent) => agent.id))
    const workflowRuns = session.workflow_runs ?? []
    const completedWorkflowRuns = workflowRuns.filter(workflowRunIsCompleted)

    const taskKey = task
      ? `${task.status}:${task.revision}:${task.task_markdown?.length ?? 0}:${task.plan_markdown?.length ?? 0}:${task.completion_summary ?? ''}:${task.blocked_reason ?? ''}:${task.aborted_reason ?? ''}`
      : 'none'
    if (taskKey !== lastTaskKey) {
      lastTaskKey = taskKey
      log('task-observed', {
        status: task?.status ?? null,
        revision: task?.revision ?? null,
        taskLength: task?.task_markdown?.length ?? 0,
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

    const runKey = workflowRuns.map((run) => `${run.id}:${run.workflow_id}:${run.status}:${run.node_runs?.length ?? 0}:${run.final_output?.message ?? ''}`).join('|')
    if (runKey !== lastRunKey) {
      lastRunKey = runKey
      log('workflow-runs-observed', workflowRuns.map((run) => ({
        id: run.id,
        workflowId: run.workflow_id,
        status: run.status,
        nodeRuns: run.node_runs?.length ?? 0,
        finalOutput: run.final_output?.message ?? null,
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
      const command = tool.input?.command ?? null
      if (entry.agent_id === metaagentId) {
        if (!metaagentToolIsAllowed(tool.tool)) {
          throw new Error(`metaagent used disallowed provider-native tool ${tool.tool} at ${entry.file}:${entry.line}`)
        }
        if (metaagentToolIsDirectExecution(tool.tool)) {
          throw new Error(`metaagent used direct execution/file tool ${tool.tool} at ${entry.file}:${entry.line}`)
        }
        const commandCompleted = tool.status === 'completed'
        commandDiscoveryEvidence.searched ||= /search_commands/.test(tool.tool)
        commandDiscoveryEvidence.docsRead ||= /command_docs/.test(tool.tool)
        workflowEvidence.created ||= commandCompleted && commandHits(command, /^workflow\s+(new|create)\b/)
        if (commandCompleted && commandHits(command, /^workflow\s+node\s+add\b/)) workflowEvidence.nodeAddCount += 1
        workflowEvidence.endpointCreated ||= commandCompleted && commandHits(command, /^workflow\s+endpoint\s+(new|create)\b/)
        workflowEvidence.run ||= commandCompleted && commandHits(command, /^workflow\s+(run|start)\b/)
        workflowEvidence.runInspected ||= commandCompleted && commandHits(command, /^workflow\s+(runs|get-run|show)\b/)
      } else if (workerIds.has(entry.agent_id) && tool.status === 'completed') {
        workerHistoryToolEvidence.add(historyKey)
      }
      log('history-tool-observed', {
        agentId: entry.agent_id,
        role: entry.agent_id === metaagentId ? 'meta' : 'worker',
        tool: tool.tool,
        status: tool.status ?? null,
        command,
        path: tool.input?.path ?? null,
      })
    }

    const eventsPayload = unwrap(await client.send(listMetaagentEventsRequest(sessionId, metaagentId, 100)), 'MetaagentEventsListed')
    finalEvents = eventsPayload.events ?? []
    for (const event of finalEvents) {
      if (seenEvents.has(event.event_id)) continue
      seenEvents.add(event.event_id)
      log('metaagent-event', summarizeEvent(event))
    }

    const changedFiles = await gitChangedFiles(workspace)
    const diffKey = changedFiles.join('|')
    if (diffKey !== lastDiffKey) {
      lastDiffKey = diffKey
      log('workspace-diff-observed', { changedFiles })
    }

    const currentHash = await hashFile(path.join(workspace, RESULT_FILE))
    const resultChanged = currentHash !== baselineHash
    if (resultChanged && (!buildPassed || !testPassed) && diffKey !== lastValidationDiffKey) {
      lastValidationDiffKey = diffKey
      if (!buildPassed) {
        buildResult = await run('npm', ['run', 'build'], { cwd: workspace, env })
        if (buildResult.code === 0) {
          buildPassed = true
          log('build-now-passes', { command: 'npm run build' })
        } else {
          log('build-still-failing', {
            code: buildResult.code,
            stdoutTail: buildResult.stdout.slice(-500),
            stderrTail: buildResult.stderr.slice(-500),
          })
        }
      }
      if (!testPassed) {
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
    }

    if (task?.status === 'blocked' || task?.status === 'aborted') {
      throw new Error(`metaagent task ended as ${task.status}: ${task.blocked_reason ?? task.aborted_reason ?? 'no reason'}`)
    }

    const workflowComplete = workflowEvidence.created
      && workflowEvidence.nodeAddCount >= 1
      && workflowEvidence.endpointCreated
      && workflowEvidence.run
      && workflowEvidence.runInspected
      && completedWorkflowRuns.length > 0
    const workerEventCount = finalEvents.filter((event) => event.source_agent_id && workerIds.has(event.source_agent_id)).length
    const workerEvidenceCount = workerEventCount + workerHistoryToolEvidence.size
    const resultFileChanged = changedFiles.includes(RESULT_FILE)
    if (
      task?.status === 'completed'
      && workers.length > 0
      && workers.every((agent) => agent.provider !== 'dev-stub')
      && workerEvidenceCount > 0
      && task.plan_markdown?.trim()
      && workflowComplete
      && resultFileChanged
      && buildPassed
      && testPassed
    ) {
      return {
        session,
        task,
        workers,
        events: finalEvents,
        workflowRuns,
        completedWorkflowRuns,
        workerEventCount,
        workerHistoryToolEvidenceCount: workerHistoryToolEvidence.size,
        workflowEvidence,
        commandDiscoveryEvidence,
        buildResult,
        testResult,
        changedFiles,
      }
    }

    await sleep(options.pollMs)
  }

  throw new Error(`timed out waiting for simple workflow metaagent completion\nlast session=${JSON.stringify({
    task: finalSession?.metaagent_tasks?.find((entry) => entry.metaagent_id === metaagentId) ?? null,
    agents: finalSession?.agents?.map((agent) => ({ id: agent.id, alias: agent.alias, role: agent.role, provider: agent.provider })),
    workflowRuns: finalSession?.workflow_runs?.map((run) => ({
      id: run.id,
      workflowId: run.workflow_id,
      status: run.status,
      nodeRuns: run.node_runs?.length ?? 0,
      finalOutput: run.final_output?.message ?? null,
    })),
    events: finalEvents.map(summarizeEvent),
  }, null, 2)}`)
}

async function validateNoDependencies(workspace) {
  const packageJson = JSON.parse(await readFile(path.join(workspace, 'package.json'), 'utf8'))
  assert(Object.keys(packageJson.dependencies ?? {}).length === 0, 'drill must not add runtime dependencies', packageJson.dependencies)
  assert(Object.keys(packageJson.devDependencies ?? {}).length === 0, 'drill must not add dev dependencies', packageJson.devDependencies)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-metaagent-simple-workflow-drill', `${process.pid}-${Date.now()}`)
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
    ARROBA_DAEMON_ID: `metaagent-simple-workflow-drill-${process.pid}-${Date.now()}`,
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

    const initiallyFailingBuild = await run('npm', ['run', 'build'], { cwd: workspace, env })
    assert(initiallyFailingBuild.code !== 0, 'fixture should start without workflow result', {
      stdout: initiallyFailingBuild.stdout,
      stderr: initiallyFailingBuild.stderr,
    })
    await runChecked('git', ['add', '.'], { cwd: workspace })
    await runChecked('git', ['commit', '-m', 'Add simple workflow drill fixture'], { cwd: workspace })
    const baselineHash = await hashFile(path.join(workspace, RESULT_FILE))

    const kernelBinary = await buildKernel()
    daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ['ignore', 'ignore', 'inherit'] })
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
    client = new LocalIpcClient(kernelUrl, {
      kernelMaxMissedPongs: Math.max(120, Math.ceil(options.timeoutMs / 5_000)),
    })
    const attachment = unwrap(await client.send(requests.attachToSessionRequest(sessionId, `metaagent-simple-workflow-drill-${Date.now()}`)), 'SessionAttached').attachment
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

    const metaPrompt = `/meta ${TASK_PROMPT}`
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
      baselineHash,
      options,
      env,
    })

    await validateNoDependencies(workspace)
    const finalBuild = await runChecked('npm', ['run', 'build'], { cwd: workspace, env })
    const finalTest = await runChecked('npm', ['test'], { cwd: workspace, env })
    const resultText = await readFile(path.join(workspace, RESULT_FILE), 'utf8')

    const result = {
      status: 'ok',
      mode: 'metaagent-simple-workflow-drill',
      rootDir,
      workspace,
      kernelUrl,
      sessionId,
      metaagentId: metaagent.id,
      provider: options.provider,
      model: options.model,
      effort: options.effort,
      promptCount: 1,
      taskPrompt: TASK_PROMPT,
      workerIds: observed.workers.map((agent) => agent.id),
      workerAliases: observed.workers.map((agent) => agent.alias),
      workerEventCount: observed.workerEventCount,
      workerHistoryToolEvidenceCount: observed.workerHistoryToolEvidenceCount,
      taskStatus: observed.task.status,
      completionSummary: observed.task.completion_summary ?? null,
      planLength: observed.task.plan_markdown.length,
      workflowEvidence: observed.workflowEvidence,
      commandDiscoveryEvidence: observed.commandDiscoveryEvidence,
      workflowRuns: observed.workflowRuns.map((run) => ({
        id: run.id,
        workflowId: run.workflow_id,
        status: run.status,
        nodeRuns: run.node_runs?.length ?? 0,
        finalOutput: run.final_output?.message ?? null,
      })),
      changedFiles: observed.changedFiles,
      resultText,
      verifiedCommands: ['npm run build', 'npm test'],
      finalBuildTail: finalBuild.stdout.slice(-800),
      finalTestTail: finalTest.stdout.slice(-800),
      metaagentEventCount: observed.events.length,
    }
    await writeFile(path.join(rootDir, 'result.json'), `${JSON.stringify(result, null, 2)}\n`, 'utf8')
    console.log(JSON.stringify(result, null, 2))
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
        drill: 'live-metaagent-simple-workflow-drill',
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
