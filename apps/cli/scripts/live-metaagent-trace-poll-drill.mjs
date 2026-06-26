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
const ORPHAN_RECOVERY_PHRASE = 'ORPHAN_RECOVERY_DRILL_COMPLETE'
let logPrefix = 'metaagent-trace-poll-drill'

function buildUserPrompt(options) {
  if (options.supervisionMode === 'orphan-recovery') {
    return [
      'This is a tiny metaagent task lifecycle check.',
      'In your first turn, do not call any tools, do not spawn agents, do not prompt workers, do not use workflows, and do not mark the task complete or blocked.',
      'For the first turn, send exactly this normal assistant reply and nothing else: ACK_ORPHAN_RECOVERY_READY',
      'That first reply should end the turn with the task still active.',
      'Arroba should send you a continuation prompt because the active task has no delegated work running.',
      'On that continuation, inspect the event/task state if needed, then call `arroba.meta.complete_task`.',
      `The completion summary must include the exact phrase ${ORPHAN_RECOVERY_PHRASE}.`,
    ].join('\n')
  }

  const workerPlacementInstruction = options.workerPlacement === 'new-headless-slice'
    ? 'Place the worker in a new headless slice using the agent spawn slice option.'
    : 'Keep the worker in this current session worktree; do not place it in a slice, remote kernel, separate directory, or new worktree.'
  const launchInstruction = options.workerProvider || options.workerModel || options.workerEffort
    ? [
        'Spawn one regular worker agent for a tiny supervision check.',
        `Use provider ${options.workerProvider}, model ${options.workerModel}, and variant/effort ${options.workerEffort} for the worker.`,
        'Do not use the session default launch profile for the worker.',
        workerPlacementInstruction,
      ]
    : [
        'Spawn one regular worker agent for a tiny supervision check.',
        '',
        'Use the session default model when spawning the worker; do not pass an explicit model.',
      ]

  if (options.supervisionMode === 'event-continuation') {
    return [
      ...launchInstruction,
      'Give the worker a repository inspection task that stays active for at least 15 seconds before its final response: inspect the README, git status, recent commit, and file list, then summarize what kind of fixture this repo is.',
      `The worker response must include the exact phrase ${TRACE_PHRASE}.`,
      'Do not call subscribe_trace, wait_trace, or poll_trace for this task.',
      'After you have spawned and prompted the worker, immediately stop this turn without marking the task complete.',
      'Do not wait, poll, explain, summarize, or call any more tools in that turn.',
      'Arroba will send you a continuation prompt when the worker turn completes.',
      'On that continuation, review the worker output using the event and turn/history tools available to you, then complete this metaagent task with a concise summary of the worker result and the evidence you reviewed.',
    ].join('\n')
  }

  return [
    ...launchInstruction,
    'Before prompting the worker, subscribe to that worker live trace with `arroba.meta.subscribe_trace`.',
    `Ask the worker to inspect this repo and include the exact phrase ${TRACE_PHRASE} in its response.`,
    'Call `arroba.meta.wait_trace` until you can see worker-generated output, not just a prompt echo.',
    'Then complete this metaagent task with a concise summary of the worker result and the trace evidence you observed.',
  ].join('\n')
}

function parseArgs(argv) {
  const options = {
    provider: DEFAULT_PROVIDER,
    model: DEFAULT_MODEL,
    effort: DEFAULT_EFFORT,
    workerProvider: process.env.ARROBA_METAAGENT_TRACE_POLL_WORKER_PROVIDER ?? '',
    workerModel: process.env.ARROBA_METAAGENT_TRACE_POLL_WORKER_MODEL ?? '',
    workerEffort: process.env.ARROBA_METAAGENT_TRACE_POLL_WORKER_EFFORT ?? '',
    workerPlacement: process.env.ARROBA_METAAGENT_TRACE_POLL_WORKER_PLACEMENT ?? 'current-worktree',
    supervisionMode: process.env.ARROBA_METAAGENT_TRACE_POLL_SUPERVISION_MODE ?? 'trace',
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
    else if (arg === '--worker-provider') options.workerProvider = String(argv[++index] ?? '').trim()
    else if (arg === '--worker-model') options.workerModel = String(argv[++index] ?? '').trim()
    else if (arg === '--worker-effort' || arg === '--worker-variant') options.workerEffort = String(argv[++index] ?? '').trim()
    else if (arg === '--worker-placement') options.workerPlacement = String(argv[++index] ?? '').trim()
    else if (arg === '--supervision-mode') options.supervisionMode = String(argv[++index] ?? '').trim()
    else if (arg === '--account-profile') options.accountProfile = String(argv[++index] ?? '').trim()
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++index])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++index])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--discard-artifacts-on-failure') options.keepArtifactsOnFailure = false
    else if (arg === '--preserve-on-success') options.preserveOnSuccess = true
    else if (arg === '--discard-artifacts-on-success') options.preserveOnSuccess = false
    else if (arg === '--help' || arg === '-h') {
      console.log([
        'Usage: node apps/cli/scripts/live-metaagent-trace-poll-drill.mjs [options]',
        '',
        'Runs a real-provider metaagent drill that validates worker supervision.',
        '',
        'Supervision modes:',
        '  trace               Subscribe and wait on live worker trace output.',
        '  event-continuation  Yield after delegation, then rely on kernel event continuation.',
        '  orphan-recovery     Yield with an active task and no workers, then rely on kernel orphan recovery.',
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
  const workerProfileParts = [options.workerProvider, options.workerModel, options.workerEffort].filter(Boolean).length
  if (workerProfileParts > 0 && workerProfileParts < 3) {
    throw new Error('--worker-provider, --worker-model, and --worker-effort must be provided together')
  }
  if (!['current-worktree', 'new-headless-slice'].includes(options.workerPlacement)) {
    throw new Error('--worker-placement must be current-worktree or new-headless-slice')
  }
  if (!['trace', 'event-continuation', 'orphan-recovery'].includes(options.supervisionMode)) {
    throw new Error('--supervision-mode must be trace, event-continuation, or orphan-recovery')
  }
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

function unwrap(response, key) {
  return response?.[key] ?? response
}

function unwrapVariant(response, ...keys) {
  return keys.map((key) => response?.[key]).find((value) => value != null) ?? response
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
    if (last?.state === 'Running' || last?.state === 'Active') return last
    if (last?.state === 'Ended') throw new Error(`provider run ended before active: ${JSON.stringify(last)}`)
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
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(5_000)])
  if (child.exitCode == null && child.signalCode == null) child.kill('SIGKILL')
}

function workerMatchesProfile(agent, options) {
  if (!options.workerProvider) return true
  return agent.provider === options.workerProvider
    && modelMatches(agent.model, options.workerModel, options.workerProvider)
    && agent.effort === options.workerEffort
}

function agentActivePrompt(session, agentId) {
  return session.prompt_states?.[agentId]?.active_prompt
    ?? (session.active_prompt?.target_agent_id === agentId ? session.active_prompt : null)
}

function agentIsIdle(session, agent) {
  if (!agent) return false
  return !agentActivePrompt(session, agent.id)
    && agent.state !== 'Working'
    && agent.is_processing !== true
}

function workerCompletionWasWakeup(status) {
  return status === 'submitted' || status === 'delivered'
}

function orphanRecoveryWasWakeup(status) {
  return status === 'submitted' || status === 'delivered'
}

function modelMatches(actual, expected, provider) {
  if (actual === expected) return true
  if (!actual || !expected || !provider) return false
  return actual === `${provider}/${expected}`
}

async function observe({ client, requests, sessionId, metaagentId, historyDir, beforeAgentIds, timeoutMs, pollMs, options }) {
  const deadline = Date.now() + timeoutMs
  const seenTools = new Set()
  const seenEvents = new Set()
  let sawSubscribeTrace = false
  let sawTraceWait = false
  let sawTracePhrase = false
  let sawForbiddenTraceTool = false
  let sawWorkerPrompt = false
  let sawWorkerCompletionEvent = false
  let sawInjectedContinuationPrompt = false
  let sawOrphanedTaskEvent = false
  let sawInjectedOrphanContinuationPrompt = false
  let orphanRecoveryDeliveryStatus = null
  let sawMetaagentYieldedBeforeWorkerEvent = false
  let sawMetaagentYieldedBeforeOrphanEvent = false
  let workerCompletionDeliveryStatus = null
  let sawReviewTool = false
  let sawPostEventReview = false
  let sawCompleteBeforeEvent = false
  let sawCompleteBeforeOrphanEvent = false
  let sawDelegationInOrphanMode = false
  let finalTask = null
  let workers = []
  while (Date.now() < deadline) {
    const session = await getSession(client, requests, sessionId)
    finalTask = (session.metaagent_tasks ?? []).find((entry) => entry.metaagent_id === metaagentId) ?? null
    const metaagent = (session.agents ?? []).find((agent) => agent.id === metaagentId)
    workers = (session.agents ?? []).filter((agent) => !beforeAgentIds.has(agent.id) && agent.id !== metaagentId)
    if (workers.length > 0) {
      log('workers-observed', workers.map((agent) => ({
        id: agent.id,
        alias: agent.alias ?? null,
        provider: agent.provider,
        model: agent.model,
        effort: agent.effort ?? null,
      })))
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
      if (tool.tool.includes('subscribe_trace')) {
        sawSubscribeTrace = true
        if (options.supervisionMode === 'event-continuation' || options.supervisionMode === 'orphan-recovery') sawForbiddenTraceTool = true
      }
      if (tool.tool.includes('wait_trace') || tool.tool.includes('poll_trace')) {
        if (options.supervisionMode === 'event-continuation' || options.supervisionMode === 'orphan-recovery') sawForbiddenTraceTool = true
        if (tool.tool.includes('wait_trace')) sawTraceWait = true
        if (traceItems(tool).some(traceItemContainsWorkerPhrase)) sawTracePhrase = true
      }
      if (tool.tool.includes('run_command') && tool.input?.command?.trim().startsWith('prompt ')) {
        sawWorkerPrompt = true
      }
      if (options.supervisionMode === 'orphan-recovery'
        && ((tool.tool === 'spawnAgent' || tool.tool.includes('run_command')))) {
        const command = String(tool.input?.command ?? '')
        if (tool.tool === 'spawnAgent'
          || command.trim().startsWith('agent spawn')
          || command.trim().startsWith('prompt ')
          || command.trim().startsWith('workflow ')) {
          sawDelegationInOrphanMode = true
        }
      }
      if (tool.tool.includes('complete_task') && !sawWorkerCompletionEvent) {
        sawCompleteBeforeEvent = true
      }
      if (tool.tool.includes('complete_task') && !sawOrphanedTaskEvent) sawCompleteBeforeOrphanEvent = true
      if (tool.tool.includes('turn_overview') || tool.tool.includes('turn_blob') || tool.tool.includes('read_event')) {
        sawReviewTool = true
      }
      if (sawWorkerCompletionEvent && sawReviewTool) {
        sawPostEventReview = true
      }
    }
    if (options.supervisionMode === 'event-continuation') {
      if (sawWorkerPrompt && !sawWorkerCompletionEvent && !sawMetaagentYieldedBeforeWorkerEvent && agentIsIdle(session, metaagent)) {
        sawMetaagentYieldedBeforeWorkerEvent = true
        log('metaagent-yielded-before-worker-event', {
          metaagentId,
          state: metaagent?.state ?? null,
          isProcessing: metaagent?.is_processing ?? null,
          activePrompt: agentActivePrompt(session, metaagentId)?.id ?? null,
        })
      }
      const eventsPayload = unwrap(await client.send(listMetaagentEventsRequest(sessionId, metaagentId, 100)), 'MetaagentEventsListed')
      const events = eventsPayload.events ?? []
      const workerIds = new Set(workers.map((agent) => agent.id))
      for (const event of events) {
        if (!seenEvents.has(event.event_id)) {
          seenEvents.add(event.event_id)
          log('metaagent-event-observed', {
            eventId: event.event_id,
            kind: event.kind,
            sourceAgentId: event.source_agent_id ?? null,
            injectedPromptId: event.injected_prompt_id ?? null,
            deliveryStatus: event.prompt_delivery_status ?? null,
          })
        }
        if (event.kind === 'agent.turn.completed' && workerIds.has(event.source_agent_id)) {
          sawWorkerCompletionEvent = true
          workerCompletionDeliveryStatus = event.prompt_delivery_status ?? null
          if (event.injected_prompt_id) sawInjectedContinuationPrompt = true
          if (sawReviewTool) sawPostEventReview = true
          if (workerCompletionDeliveryStatus === 'steered') {
            throw new Error('worker completion event was steered into an active metaagent turn; expected a submitted continuation after yield')
          }
        }
      }
    }
    if (options.supervisionMode === 'orphan-recovery') {
      if (!sawOrphanedTaskEvent && !sawMetaagentYieldedBeforeOrphanEvent && agentIsIdle(session, metaagent)) {
        sawMetaagentYieldedBeforeOrphanEvent = true
        log('metaagent-yielded-before-orphan-event', {
          metaagentId,
          state: metaagent?.state ?? null,
          isProcessing: metaagent?.is_processing ?? null,
          activePrompt: agentActivePrompt(session, metaagentId)?.id ?? null,
        })
      }
      const eventsPayload = unwrap(await client.send(listMetaagentEventsRequest(sessionId, metaagentId, 100)), 'MetaagentEventsListed')
      const events = eventsPayload.events ?? []
      for (const event of events) {
        if (!seenEvents.has(event.event_id)) {
          seenEvents.add(event.event_id)
          log('metaagent-event-observed', {
            eventId: event.event_id,
            kind: event.kind,
            sourceAgentId: event.source_agent_id ?? null,
            injectedPromptId: event.injected_prompt_id ?? null,
            deliveryStatus: event.prompt_delivery_status ?? null,
          })
        }
        if (event.kind === 'metaagent.task.orphaned') {
          sawOrphanedTaskEvent = true
          orphanRecoveryDeliveryStatus = event.prompt_delivery_status ?? null
          if (event.injected_prompt_id) sawInjectedOrphanContinuationPrompt = true
          if (orphanRecoveryDeliveryStatus === 'steered') {
            throw new Error('orphan recovery event was steered into an active metaagent turn; expected a submitted continuation after yield')
          }
        }
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
    const profileMatched = workers.some((agent) => workerMatchesProfile(agent, options))
    if (options.supervisionMode === 'orphan-recovery') {
      if (sawForbiddenTraceTool) throw new Error('metaagent used trace tools in orphan-recovery mode')
      if (sawDelegationInOrphanMode) throw new Error('metaagent delegated work in orphan-recovery mode')
      if (sawCompleteBeforeOrphanEvent && !sawOrphanedTaskEvent) throw new Error('metaagent completed task before kernel orphan recovery event')
      if (finalTask?.status === 'completed'
        && (sawMetaagentYieldedBeforeOrphanEvent || orphanRecoveryDeliveryStatus === 'submitted')
        && sawOrphanedTaskEvent
        && sawInjectedOrphanContinuationPrompt
        && orphanRecoveryWasWakeup(orphanRecoveryDeliveryStatus)
        && (finalTask.completion_summary ?? '').includes(ORPHAN_RECOVERY_PHRASE)) {
        return {
          task: finalTask,
          workers,
          sawMetaagentYieldedBeforeOrphanEvent,
          sawOrphanedTaskEvent,
          sawInjectedOrphanContinuationPrompt,
          orphanRecoveryDeliveryStatus,
        }
      }
      if (finalTask?.status === 'completed') {
        throw new Error(`metaagent task completed without validated orphan recovery: ${JSON.stringify({
          sawMetaagentYieldedBeforeOrphanEvent,
          sawOrphanedTaskEvent,
          sawInjectedOrphanContinuationPrompt,
          orphanRecoveryDeliveryStatus,
          summary: finalTask.completion_summary ?? null,
        }, null, 2)}`)
      }
      await sleep(pollMs)
      continue
    }
    if (options.supervisionMode === 'event-continuation') {
      if (sawForbiddenTraceTool) throw new Error('metaagent used trace tools in event-continuation mode')
      if (sawCompleteBeforeEvent) throw new Error('metaagent completed task before worker completion event')
      if (finalTask?.status === 'completed'
        && workers.length > 0
        && profileMatched
        && sawWorkerPrompt
        && sawMetaagentYieldedBeforeWorkerEvent
        && sawWorkerCompletionEvent
        && sawInjectedContinuationPrompt
        && workerCompletionWasWakeup(workerCompletionDeliveryStatus)
        && sawPostEventReview
        && (finalTask.completion_summary ?? '').includes(TRACE_PHRASE)) {
        return {
          task: finalTask,
          workers,
          sawSubscribeTrace,
          sawTraceWait,
          sawTracePhrase,
          sawWorkerPrompt,
          sawMetaagentYieldedBeforeWorkerEvent,
          sawWorkerCompletionEvent,
          sawInjectedContinuationPrompt,
          workerCompletionDeliveryStatus,
          sawPostEventReview,
        }
      }
      if (finalTask?.status === 'completed') {
        throw new Error(`metaagent task completed without validated event continuation: ${JSON.stringify({
          workers: workers.map((agent) => ({
            id: agent.id,
            alias: agent.alias,
            provider: agent.provider,
            model: agent.model,
            effort: agent.effort ?? null,
          })),
          expectedWorkerProfile: options.workerProvider ? {
            provider: options.workerProvider,
            model: options.workerModel,
            effort: options.workerEffort,
          } : null,
          profileMatched,
          sawWorkerPrompt,
          sawMetaagentYieldedBeforeWorkerEvent,
          sawWorkerCompletionEvent,
          sawInjectedContinuationPrompt,
          workerCompletionDeliveryStatus,
          sawPostEventReview,
          summary: finalTask.completion_summary ?? null,
        }, null, 2)}`)
      }
      await sleep(pollMs)
      continue
    }
    if (finalTask?.status === 'completed' && workers.length > 0 && profileMatched && sawSubscribeTrace && sawTraceWait && sawTracePhrase) {
      return { task: finalTask, workers, sawSubscribeTrace, sawTraceWait, sawTracePhrase }
    }
    if (finalTask?.status === 'completed') {
      throw new Error(`metaagent task completed without validated worker trace output: ${JSON.stringify({
        workers: workers.map((agent) => ({
          id: agent.id,
          alias: agent.alias,
          provider: agent.provider,
          model: agent.model,
          effort: agent.effort ?? null,
        })),
        expectedWorkerProfile: options.workerProvider ? {
          provider: options.workerProvider,
          model: options.workerModel,
          effort: options.workerEffort,
        } : null,
        profileMatched,
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
    workers: workers.map((agent) => ({
      id: agent.id,
      alias: agent.alias,
      provider: agent.provider,
      model: agent.model,
      effort: agent.effort ?? null,
    })),
    expectedWorkerProfile: options.workerProvider ? {
      provider: options.workerProvider,
      model: options.workerModel,
      effort: options.workerEffort,
    } : null,
    sawSubscribeTrace,
    sawTraceWait,
    sawTracePhrase,
    sawWorkerPrompt,
    sawMetaagentYieldedBeforeWorkerEvent,
    sawWorkerCompletionEvent,
    sawInjectedContinuationPrompt,
    sawMetaagentYieldedBeforeOrphanEvent,
    sawOrphanedTaskEvent,
    sawInjectedOrphanContinuationPrompt,
    orphanRecoveryDeliveryStatus,
    workerCompletionDeliveryStatus,
    sawPostEventReview,
    sawForbiddenTraceTool,
    sawDelegationInOrphanMode,
  }, null, 2)}`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  logPrefix = options.supervisionMode === 'event-continuation'
    ? 'metaagent-event-continuation-drill'
    : options.supervisionMode === 'orphan-recovery'
      ? 'metaagent-orphan-recovery-drill'
    : 'metaagent-trace-poll-drill'
  const artifactName = options.supervisionMode === 'event-continuation'
    ? 'live-metaagent-event-continuation-drill'
    : options.supervisionMode === 'orphan-recovery'
      ? 'live-metaagent-orphan-recovery-drill'
    : 'live-metaagent-trace-poll-drill'
  const rootDir = path.join(repoRoot, 'target', artifactName, `${process.pid}-${Date.now()}`)
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
      'session new $workspace as session',
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

    const userPrompt = `/meta ${buildUserPrompt(options)}`
    await client.send(requests.submitPromptRequest(sessionId, attachment.id, metaagent.id, userPrompt, []))
    log('single-prompt-submitted', { metaagentId: metaagent.id, prompt: userPrompt })
    const metaRun = await waitForAgentProviderRun(client, requests, sessionId, metaagent.id, options.timeoutMs, options.pollMs)
    assert(metaRun.adapter_key !== 'dev-stub' && metaRun.provider !== 'dev-stub', 'metaagent must run on a real provider', metaRun)
    assert(metaRun.execution_mode === 'plan', 'meta-mode provider run must be plan mode', metaRun)
    const metaSession = await getSession(client, requests, sessionId)
    const metaModeAgent = (metaSession.agents ?? []).find((agent) => agent.id === metaagent.id)
    assert(metaModeAgent?.meta_mode, 'same regular agent should enter meta mode after /meta prompt', metaModeAgent)
    log('metaagent-run-observed', { providerRunId: metaRun.id, executionMode: metaRun.execution_mode })

    const observed = await observe({
      client,
      requests,
      sessionId,
      metaagentId: metaagent.id,
      historyDir: env.ARROBA_SESSION_HISTORY_DIR,
      beforeAgentIds,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
      options,
    })
    console.log(JSON.stringify({
      status: 'ok',
      mode: options.supervisionMode === 'event-continuation'
        ? 'metaagent-event-continuation-drill'
        : 'metaagent-trace-poll-drill',
      sessionId,
      metaagentId: metaagent.id,
      provider: options.provider,
      model: options.model,
      workerProvider: options.workerProvider || null,
      workerModel: options.workerModel || null,
      workerEffort: options.workerEffort || null,
      workerPlacement: options.workerPlacement,
      supervisionMode: options.supervisionMode,
      promptCount: 1,
      workers: observed.workers.map((agent) => ({
        id: agent.id,
        alias: agent.alias ?? null,
        provider: agent.provider,
        model: agent.model,
        effort: agent.effort ?? null,
      })),
      taskStatus: observed.task.status,
      sawSubscribeTrace: observed.sawSubscribeTrace,
      sawTraceWait: observed.sawTraceWait,
      sawTracePhrase: observed.sawTracePhrase,
      sawWorkerPrompt: observed.sawWorkerPrompt ?? null,
      sawMetaagentYieldedBeforeWorkerEvent: observed.sawMetaagentYieldedBeforeWorkerEvent ?? null,
      sawWorkerCompletionEvent: observed.sawWorkerCompletionEvent ?? null,
      sawInjectedContinuationPrompt: observed.sawInjectedContinuationPrompt ?? null,
      workerCompletionDeliveryStatus: observed.workerCompletionDeliveryStatus ?? null,
      sawPostEventReview: observed.sawPostEventReview ?? null,
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
        drill: 'metaagent-trace-poll',
        supervisionMode: options.supervisionMode,
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
  log('passed')
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack ?? error.message : error)
  process.exit(1)
})
