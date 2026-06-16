#!/usr/bin/env node
import { createHash } from 'node:crypto'
import { spawn } from 'node:child_process'
import { createWriteStream } from 'node:fs'
import { mkdir, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const DEFAULT_TIMEOUT_MS = 1_800_000
const DEFAULT_POLL_MS = 1_000
const DEFAULT_PROVIDER = process.env.ARROBA_METAAGENT_WORKFLOW_WEBAPP_PROVIDER ?? 'codex'
const DEFAULT_MODEL = process.env.ARROBA_METAAGENT_WORKFLOW_WEBAPP_MODEL ?? 'gpt-5.5'
const DEFAULT_EFFORT = process.env.ARROBA_METAAGENT_WORKFLOW_WEBAPP_EFFORT ?? 'medium'
let logPrefix = 'metaagent-workflow-webapp-drill'

const WORKFLOW_USER_PROMPT = [
  'Build a small local web app for managing a personal kanban board.',
  '',
  'The app should let a user create, edit, delete, and move tasks across Todo, Doing, and Done columns. It should persist locally in the browser, work without any external services, and include enough validation or tests for a reviewer to trust the result.',
  '',
  'Use a workflow suitable for this task. Decide the workflow structure yourself, build the app by running that workflow, supervise the results, and complete the task only when the app builds and the implementation has been reviewed.',
].join('\n')

const DIRECT_USER_PROMPT = [
  'Build a small local web app for managing a personal kanban board.',
  '',
  'The app should let a user create, edit, delete, and move tasks across Todo, Doing, and Done columns. It should persist locally in the browser, work without any external services, and include enough validation or tests for a reviewer to trust the result.',
  '',
  'Decide the plan yourself, use regular agents as needed, supervise their results, and complete the task only when the app builds and the implementation has been reviewed.',
].join('\n')

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
    withoutWorkflowRequirement: false,
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
    else if (arg === '--without-workflow-requirement') options.withoutWorkflowRequirement = true
    else if (arg === '--help' || arg === '-h') {
      console.log([
        'Usage: node apps/cli/scripts/live-metaagent-workflow-webapp-drill.mjs [options]',
        '',
        'Runs an observe-only live metaagent webapp drill against a real provider:',
        '- creates a dependency-free static web app repo with local validation scripts',
        '- starts a real metaagent in plan mode',
        '- submits exactly one high-level task prompt',
        '- observes session state, metaagent events, history, and workspace diff only',
        '- passes only if the task is completed and local build/test validation passes',
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
        '  --without-workflow-requirement',
        '  --preserve-on-success',
        '  --discard-artifacts-on-success',
      ].join('\n'))
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  if (!options.provider || options.provider === 'dev-stub') {
    throw new Error('metaagent workflow webapp drill requires a real provider; dev-stub is not valid evidence')
  }
  if (!options.model) throw new Error('--model is required')
  if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) throw new Error('--timeout-ms must be positive')
  if (!Number.isFinite(options.pollMs) || options.pollMs <= 0) throw new Error('--poll-ms must be positive')
  return options
}

function makePorts() {
  const kernelPort = 62100 + Math.floor(Math.random() * 700)
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
  await runChecked('git', ['config', 'user.email', 'metaagent-workflow-webapp-drill@example.com'], { cwd: root })
  await runChecked('git', ['config', 'user.name', 'Metaagent Workflow Webapp Drill'], { cwd: root })
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
  await writeFile(path.join(workspace, 'package.json'), `${JSON.stringify({
    type: 'module',
    scripts: {
      build: 'node scripts/validate-build.mjs',
      test: 'node --test',
    },
    dependencies: {},
    devDependencies: {},
  }, null, 2)}\n`, 'utf8')
  await writeFile(path.join(workspace, 'README.md'), [
    '# Local Kanban Web App Fixture',
    '',
    'Build a dependency-free browser app in this repo. It should run by opening `index.html` or serving the folder with a static file server.',
    '',
    'Validation is local only:',
    '',
    '- `npm run build` checks app structure and offline constraints.',
    '- `npm test` checks the implemented browser app has the expected capabilities.',
    '',
  ].join('\n'), 'utf8')
  await writeFile(path.join(workspace, 'scripts', 'validate-build.mjs'), [
    "import assert from 'node:assert/strict'",
    "import { readFile, stat } from 'node:fs/promises'",
    '',
    "const required = ['index.html', 'src/app.js', 'src/styles.css']",
    'for (const file of required) {',
    '  const info = await stat(file).catch(() => null)',
    '  assert.ok(info?.isFile(), `${file} must exist`)',
    '}',
    '',
    "const html = await readFile('index.html', 'utf8')",
    "const js = await readFile('src/app.js', 'utf8')",
    "const css = await readFile('src/styles.css', 'utf8')",
    "const combined = `${html}\\n${js}\\n${css}`",
    "assert.match(html, /src\\/app\\.js/, 'index.html should load src/app.js')",
    "assert.match(html, /src\\/styles\\.css/, 'index.html should load src/styles.css')",
    "assert.match(combined, /Todo/i, 'Todo column should be present')",
    "assert.match(combined, /Doing/i, 'Doing column should be present')",
    "assert.match(combined, /Done/i, 'Done column should be present')",
    "assert.match(js, /localStorage/, 'app should persist with localStorage')",
    "assert.match(js, /addEventListener/, 'app should wire browser interactions')",
    "assert.doesNotMatch(combined, /https?:\\/\\//i, 'app must not depend on external services or CDNs')",
    '',
  ].join('\n'), 'utf8')
  await writeFile(path.join(workspace, 'test', 'kanban-app.test.mjs'), [
    "import assert from 'node:assert/strict'",
    "import { readFile } from 'node:fs/promises'",
    "import test from 'node:test'",
    '',
    "test('kanban app implements local task creation, editing, deletion, movement, and persistence', async () => {",
    "  const html = await readFile('index.html', 'utf8')",
    "  const js = await readFile('src/app.js', 'utf8')",
    "  const css = await readFile('src/styles.css', 'utf8')",
    "  const combined = `${html}\\n${js}\\n${css}`",
    "  for (const term of ['Todo', 'Doing', 'Done']) assert.match(combined, new RegExp(term, 'i'))",
    "  for (const term of ['create', 'edit', 'delete']) assert.match(combined, new RegExp(term, 'i'))",
    "  assert.match(js, /localStorage/, 'state should persist locally')",
    "  assert.match(js, /(dragstart|drop|change|click)/, 'tasks should be movable through browser interactions')",
    "  assert.doesNotMatch(combined, /https?:\\/\\//i, 'no external services or CDN URLs')",
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

async function launchMetaagent(client, requests, sessionId, metaagent, options) {
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
  const providerRun = launched.provider_run
  assert(providerRun?.id, 'metaagent launch did not return a provider run', launched)
  const active = await waitForProviderRun(client, requests, providerRun.id, options.timeoutMs, options.pollMs)
  assert(active.adapter_key !== 'dev-stub' && active.provider !== 'dev-stub', 'metaagent must run on a real provider', active)
  assert(active.execution_mode === 'plan', 'metaagent provider run must be forced to plan mode', active)
  return active
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

function commandHits(commandText, pattern) {
  return typeof commandText === 'string' && pattern.test(commandText)
}

async function observeUntilComplete({
  client,
  requests,
  sessionId,
  metaagentId,
  workspace,
  historyDir,
  beforeAgentIds,
  baselineHashes,
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
    edgeAdded: false,
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
  let lastRunKey = null
  let lastDiffKey = null
  let buildPassed = false
  let testPassed = false
  let buildResult = null
  let testResult = null
  let lastValidationDiffKey = null
  let finalSession = null
  let finalEvents = []

  while (Date.now() < deadline) {
    const session = await getSession(client, requests, sessionId)
    finalSession = session
    const metaagent = (session.agents ?? []).find((agent) => agent.id === metaagentId)
    const task = (session.metaagent_tasks ?? []).find((entry) => entry.metaagent_id === metaagentId)
    const workers = (session.agents ?? []).filter((agent) => !beforeAgentIds.has(agent.id) && agent.id !== metaagentId && agent.role !== 'meta')
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
        finalOutputValid: run.final_output_valid ?? null,
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
        workflowEvidence.edgeAdded ||= commandCompleted && commandHits(command, /^workflow\s+edge\s+add\b/)
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

    const currentHashes = {
      html: await hashFile(path.join(workspace, 'index.html')),
      js: await hashFile(path.join(workspace, 'src', 'app.js')),
      css: await hashFile(path.join(workspace, 'src', 'styles.css')),
    }
    const appChanged = currentHashes.html !== baselineHashes.html
      || currentHashes.js !== baselineHashes.js
      || currentHashes.css !== baselineHashes.css
    if (appChanged && (!buildPassed || !testPassed) && diffKey !== lastValidationDiffKey) {
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
      && workflowEvidence.nodeAddCount >= 2
      && workflowEvidence.edgeAdded
      && workflowEvidence.endpointCreated
      && workflowEvidence.run
      && workflowEvidence.runInspected
      && completedWorkflowRuns.length > 0
      && completedWorkflowRuns.some((run) => run.final_output && run.final_output_valid !== false)
    const workflowRequirementMet = options.withoutWorkflowRequirement || workflowComplete
    const requiredFiles = ['index.html', 'src/app.js', 'src/styles.css']
    const requiredFilesChanged = requiredFiles.every((file) => changedFiles.includes(file))
    const workerEventCount = finalEvents.filter((event) => event.source_agent_id && workerIds.has(event.source_agent_id)).length
    const workerEvidenceCount = workerEventCount + workerHistoryToolEvidence.size
    if (
      task?.status === 'completed'
      && metaagent?.role === 'meta'
      && workers.length > 0
      && workers.every((agent) => agent.provider !== 'dev-stub')
      && workerEvidenceCount > 0
      && task.plan_markdown?.trim()
      && workflowRequirementMet
      && requiredFilesChanged
      && buildPassed
      && testPassed
    ) {
      return {
        session,
        task,
        workers,
        events: finalEvents,
        workerEventCount,
        workerHistoryToolEvidenceCount: workerHistoryToolEvidence.size,
        workflowEvidence,
        commandDiscoveryEvidence,
        buildResult,
        testResult,
        workflowRuns,
        completedWorkflowRuns,
        changedFiles,
      }
    }

    await sleep(options.pollMs)
  }

  throw new Error(`timed out waiting for metaagent workflow webapp completion\nlast session=${JSON.stringify({
    task: finalSession?.metaagent_tasks?.find((entry) => entry.metaagent_id === metaagentId) ?? null,
    agents: finalSession?.agents?.map((agent) => ({ id: agent.id, alias: agent.alias, role: agent.role, provider: agent.provider })),
    workflowRuns: finalSession?.workflow_runs?.map((run) => ({
      id: run.id,
      workflowId: run.workflow_id,
      status: run.status,
      nodeRuns: run.node_runs?.length ?? 0,
      finalOutput: run.final_output?.message ?? null,
      finalOutputValid: run.final_output_valid ?? null,
    })),
    events: finalEvents.map(summarizeEvent),
  }, null, 2)}`)
}

async function noExternalServices(workspace) {
  const packageJson = JSON.parse(await readFile(path.join(workspace, 'package.json'), 'utf8'))
  assert(Object.keys(packageJson.dependencies ?? {}).length === 0, 'web app must not add runtime dependencies', packageJson.dependencies)
  assert(Object.keys(packageJson.devDependencies ?? {}).length === 0, 'web app must not add dev dependencies', packageJson.devDependencies)
  const files = ['index.html', 'src/app.js', 'src/styles.css']
  const combined = (await Promise.all(files.map((file) => readFile(path.join(workspace, file), 'utf8')))).join('\n')
  assert(!/https?:\/\//i.test(combined), 'web app must not reference external URLs')
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const drillSlug = options.withoutWorkflowRequirement
    ? 'live-metaagent-webapp-direct-drill'
    : 'live-metaagent-workflow-webapp-drill'
  const drillMode = options.withoutWorkflowRequirement
    ? 'metaagent-webapp-direct-drill'
    : 'metaagent-workflow-webapp-drill'
  const taskPrompt = options.withoutWorkflowRequirement ? DIRECT_USER_PROMPT : WORKFLOW_USER_PROMPT
  logPrefix = drillSlug
  const rootDir = path.join(repoRoot, 'target', drillSlug, `${process.pid}-${Date.now()}`)
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
    ARROBA_DAEMON_ID: `${drillSlug}-${process.pid}-${Date.now()}`,
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
    assert(initiallyFailingBuild.code !== 0, 'fixture should start without an implemented app', {
      stdout: initiallyFailingBuild.stdout,
      stderr: initiallyFailingBuild.stderr,
    })
    await runChecked('git', ['add', '.'], { cwd: workspace })
    await runChecked('git', ['commit', '-m', 'Add empty kanban webapp fixture'], { cwd: workspace })
    const baselineHashes = {
      html: await hashFile(path.join(workspace, 'index.html')),
      js: await hashFile(path.join(workspace, 'src', 'app.js')),
      css: await hashFile(path.join(workspace, 'src', 'styles.css')),
    }

    const kernelBinary = await buildKernel()
    const daemonStdout = createWriteStream(path.join(rootDir, 'daemon.stdout.log'), { flags: 'a' })
    const daemonStderr = createWriteStream(path.join(rootDir, 'daemon.stderr.log'), { flags: 'a' })
    daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ['ignore', 'pipe', 'pipe'] })
    daemon.stdout.pipe(daemonStdout)
    daemon.stderr.pipe(daemonStderr)
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
    requireOutput(setup.stdout, /created metaagent session /, 'metaagent session creation')
    sessionId = setup.stdout.match(/bound \$session = (\S+)/)?.[1] ?? null
    assert(sessionId, 'setup script did not bind session id', { stdout: setup.stdout })

    const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
    const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')
    client = new LocalIpcClient(kernelUrl)
    const disposeKernelEventLog = client.onKernelEvent((event) => {
      if (event.event === 'transport_closed' || event.event === 'transport_resumed') {
        log('client-transport-event', event)
      }
    })
    const attachment = unwrap(await client.send(requests.attachToSessionRequest(sessionId, `${drillSlug}-${Date.now()}`)), 'SessionAttached').attachment
    const initialSession = await getSession(client, requests, sessionId)
    const metaagent = (initialSession.agents ?? []).find((agent) => agent.role === 'meta')
    assert(metaagent, 'session should contain a metaagent', initialSession)
    const beforeAgentIds = new Set((initialSession.agents ?? []).map((agent) => agent.id))

    const metaRun = await launchMetaagent(client, requests, sessionId, metaagent, options)
    log('metaagent-run-observed', {
      providerRunId: metaRun.id,
      provider: metaRun.provider,
      adapterKey: metaRun.adapter_key,
      executionMode: metaRun.execution_mode,
      permissionLevel: metaRun.permission_level ?? null,
    })

    await client.send(requests.submitPromptRequest(sessionId, attachment.id, metaagent.id, taskPrompt, []))
    log('single-prompt-submitted', { metaagentId: metaagent.id, prompt: taskPrompt })

    const observed = await observeUntilComplete({
      client,
      requests,
      sessionId,
      metaagentId: metaagent.id,
      workspace,
      historyDir: env.ARROBA_SESSION_HISTORY_DIR,
      beforeAgentIds,
      baselineHashes,
      options,
      env,
    })

    await noExternalServices(workspace)
    const finalBuild = await runChecked('npm', ['run', 'build'], { cwd: workspace, env })
    const finalTest = await runChecked('npm', ['test'], { cwd: workspace, env })

    const result = {
      status: 'ok',
      mode: drillMode,
      rootDir,
      workspace,
      kernelUrl,
      sessionId,
      metaagentId: metaagent.id,
      provider: options.provider,
      model: options.model,
      effort: options.effort,
      workflowRequired: !options.withoutWorkflowRequirement,
      promptCount: 1,
      taskPrompt,
      harnessRuntimeMcpCallsAfterPrompt: 0,
      harnessWorkspaceWritesAfterPrompt: 0,
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
        finalOutputValid: run.final_output_valid ?? null,
      })),
      changedFiles: observed.changedFiles,
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
        drill: drillSlug,
        mode: drillMode,
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
