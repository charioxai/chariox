#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { access, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const DEFAULT_PROVIDERS = ['codex', 'opencode']
const DEFAULT_MODEL = 'gpt-5.4'
const DEFAULT_TIMEOUT_MS = 360_000
const DEFAULT_POLL_MS = 1_000
const DEFAULT_SKIPPED_ALL_MODEL_TARGETS = new Map([
  ['opencode:opencode/claude-3-5-haiku', 'OpenCode maps this catalog target to unavailable upstream model claude-3-5-haiku-20241022'],
  ['opencode:opencode/gpt-5.4-pro', 'default drill effort is low, but this model requires medium/high/xhigh'],
])

function parseArgs(argv) {
  const options = {
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
    providerModels: {},
    allModels: false,
    maxModelsPerProvider: 1,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    keepArtifactsOnFailure: false,
    continueOnFailure: false,
    outDir: null,
    listTargets: false,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--provider') options.providers = [argv[++i]]
    else if (arg === '--providers') options.providers = argv[++i].split(',').map((value) => value.trim()).filter(Boolean)
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++i].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    } else if (arg === '--all-models') options.allModels = true
    else if (arg === '--max-models-per-provider') options.maxModelsPerProvider = Number(argv[++i])
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--out-dir') options.outDir = argv[++i]
    else if (arg === '--list-targets') options.listTargets = true
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--continue-on-failure') options.continueOnFailure = true
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-tool-display-fixture-drill.mjs [options]',
    '',
    'Runs disposable provider sessions and captures raw provider_tool events for tool-display fixtures.',
    'By default it samples one model per provider. Use --all-models to expand from the provider catalog.',
    '',
    'Options:',
    '  --provider PROVIDER',
    `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
    `  --model ${DEFAULT_MODEL}`,
    '  --provider-model PROVIDER=MODEL',
    '  --all-models',
    '  --max-models-per-provider 1',
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    `  --poll-ms ${DEFAULT_POLL_MS}`,
    '  --out-dir PATH',
    '  --list-targets',
    '  --keep-artifacts-on-failure',
    '  --continue-on-failure',
  ].join('\n'))
}

async function loadCliModules(runtimeDir) {
  const [{ transformAsync }, tsPreset] = await Promise.all([
    import('@babel/core'),
    import('@babel/preset-typescript'),
  ])
  for (const rel of ['src/ipc.ts', 'src/ipc-requests.ts']) {
    const sourcePath = path.join(cliRoot, rel)
    const outPath = path.join(runtimeDir, path.basename(rel).replace(/\.tsx?$/, '.js'))
    const code = await readFile(sourcePath, 'utf8')
    const transformed = await transformAsync(code, {
      filename: sourcePath,
      presets: [[tsPreset.default ?? tsPreset]],
      sourceMaps: false,
    })
    await writeFile(outPath, transformed?.code ?? '', 'utf8')
  }
  const ipcUrl = new URL(`file://${path.join(runtimeDir, 'ipc.js')}`).href
  const requestsUrl = new URL(`file://${path.join(runtimeDir, 'ipc-requests.js')}`).href
  const { LocalIpcClient } = await import(ipcUrl)
  const requests = await import(requestsUrl)
  return { LocalIpcClient, requests }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (resp, key) => resp?.[key] ?? resp
const unwrapVariant = (resp, ...keys) => keys.map((key) => resp?.[key]).find((value) => value != null) ?? resp

function makePorts() {
  const base = 59000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort: base,
    mcpPort: base + 1000,
    opencodePort: base + 2000,
    codexPort: base + 2001,
  }
}

async function resolveBinary(binaryPath, manifestPath, binName) {
  try {
    await access(binaryPath)
    return binaryPath
  } catch {
    throw new Error(`missing built binary ${binaryPath}; run cargo build --manifest-path ${manifestPath} --bin ${binName} first`)
  }
}

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    sleep(5_000),
  ])
  if (child.exitCode == null) {
    child.kill('SIGKILL')
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      sleep(2_000),
    ])
  }
}

async function waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl)
    try {
      const session = unwrap(await probe.send(createSessionRequest(workspace, workspace)), 'SessionCreated').session
      await probe.send(endSessionRequest(session.id)).catch(() => {})
      await probe.close()
      return
    } catch {
      await probe.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error('daemon did not become ready')
}

function modelForProvider(provider, options) {
  const explicit = options.providerModels[provider]
  if (explicit) return explicit
  if (provider === 'opencode' && !options.model.includes('/')) return `opencode/${options.model}`
  if (provider === 'codex' && !options.model.includes('/')) return opencodeCodexModel(options.model)
  return options.model
}

function opencodeCodexModel(model) {
  if (model.endsWith('-codex')) return model
  if (/^gpt-5\.[23]$/.test(model)) return `${model}-codex`
  return model
}

function drillTargets(catalog, options) {
  if (!options.allModels) {
    return options.providers.map((provider) => ({ provider, model: modelForProvider(provider, options) }))
  }
  const connected = new Set(catalog.connected ?? [])
  const targets = []
  for (const provider of catalog.all ?? []) {
    if (!options.providers.includes(provider.id) || !connected.has(provider.id)) continue
    const models = Object.values(provider.models ?? {})
      .filter((model) => model.status !== 'deprecated')
      .slice(0, options.maxModelsPerProvider)
    for (const model of models) {
      const qualifiedModel = qualifiedCatalogModelId(provider.id, model.id)
      const skipReason = DEFAULT_SKIPPED_ALL_MODEL_TARGETS.get(`${provider.id}:${qualifiedModel}`)
      if (skipReason) {
        targets.push({ provider: provider.id, model: qualifiedModel, skipReason })
      } else {
        targets.push({ provider: provider.id, model: qualifiedModel })
      }
    }
  }
  return targets
}

function qualifiedCatalogModelId(provider, modelId) {
  if (provider === 'opencode' && !modelId.includes('/')) return `opencode/${modelId}`
  return modelId
}

async function waitForAgentIdle({ client, requests, sessionId, attachmentId, agentId, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send(requests.pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    const state = unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const session = state.session ?? state
    const agent = (session.agents ?? []).find((candidate) => candidate.id === agentId)
    const promptState = (session.prompt_states ?? {})[agentId] ?? {}
    if (agent && !agent.is_processing && agent.state !== 'Working' && promptState.active_prompt == null && (promptState.queued_prompts ?? []).length === 0) {
      return
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${agentId} to become idle`)
}

async function waitForHistoryMarker({ client, requests, sessionId, attachmentId, historyDir, agentId, markers, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send(requests.pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    const result = await readHistoryMarkerState(historyDir, agentId, markers)
    if (result.error) throw new Error(result.error)
    if (result.matched) {
      return
    }
    await sleep(pollMs)
  }
  await client.send(requests.pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
  const finalResult = await readHistoryMarkerState(historyDir, agentId, markers)
  if (finalResult.error) throw new Error(finalResult.error)
  if (finalResult.matched) return
  const rawDiagnostics = await collectProviderDiagnostics(historyDir, agentId)
  if (diagnosticsContainMarker(rawDiagnostics, markers)) return
  const diagnostics = summarizeDiagnostics(rawDiagnostics)
  throw new Error(`timed out waiting for assistant marker ${markers.join(' or ')}${diagnostics ? `; ${diagnostics}` : ''}`)
}

function diagnosticsContainMarker(diagnostics, markers) {
  const text = [...diagnostics.outputs, ...diagnostics.notices].join('')
  return markers.some((marker) => text.includes(marker))
}

async function readHistoryMarkerState(historyDir, agentId, markers) {
  const entries = await collectHistoryEntries(historyDir, agentId)
  const providerFailure = entries.find((entry) => entry.kind === 'provider_error')
  if (providerFailure) {
    return { matched: false, error: String(providerFailure.text ?? 'provider error') }
  }
  const combined = entries
    .filter((entry) => entry.kind === 'provider_output' || entry.kind === 'notice')
    .map((entry) => String(entry.text ?? ''))
    .join('')
  return { matched: markers.some((marker) => combined.includes(marker)), error: null }
}

async function collectHistoryEntries(historyDir, agentId) {
  const entries = []
  const files = await listJsonlFiles(historyDir)
  for (const file of files) {
    const lines = (await readFile(file, 'utf8').catch(() => '')).split(/\r?\n/)
    for (const line of lines) {
      if (!line.trim()) continue
      try {
        const entry = JSON.parse(line)
        if (!entry.agent_id || entry.agent_id === agentId) entries.push(entry)
      } catch {
        // Ignore malformed partial writes.
      }
    }
  }
  return entries
}

async function collectProviderToolEvents(historyDir, agentId) {
  const events = []
  const files = await listJsonlFiles(historyDir)
  for (const file of files) {
    const lines = (await readFile(file, 'utf8').catch(() => '')).split(/\r?\n/)
    for (const line of lines) {
      if (!line.trim()) continue
      let entry
      try {
        entry = JSON.parse(line)
      } catch {
        continue
      }
      if (entry.agent_id && entry.agent_id !== agentId) continue
      if (entry.kind === 'provider_tool' && typeof entry.text === 'string') {
        events.push(entry.text)
      }
    }
  }
  return [...new Set(events)]
}

async function collectProviderDiagnostics(historyDir, agentId) {
  const diagnostics = {
    errors: [],
    notices: [],
    outputs: [],
    reasoning: [],
  }
  const files = await listJsonlFiles(historyDir)
  for (const file of files) {
    const lines = (await readFile(file, 'utf8').catch(() => '')).split(/\r?\n/)
    for (const line of lines) {
      if (!line.trim()) continue
      let entry
      try {
        entry = JSON.parse(line)
      } catch {
        continue
      }
      if (entry.agent_id && entry.agent_id !== agentId) continue
      if (typeof entry.text !== 'string' || entry.text.trim() === '') continue
      if (entry.kind === 'provider_error') diagnostics.errors.push(entry.text.trim())
      else if (entry.kind === 'notice') diagnostics.notices.push(entry.text.trim())
      else if (entry.kind === 'provider_output') diagnostics.outputs.push(entry.text.trim())
      else if (entry.kind === 'provider_reasoning') diagnostics.reasoning.push(entry.text.trim())
    }
  }
  return diagnostics
}

function summarizeDiagnostics(diagnostics) {
  const errors = [...new Set(diagnostics.errors)].slice(-3)
  const notices = [...new Set(diagnostics.notices)].slice(-3)
  const output = diagnostics.outputs.join('').trim().slice(-600)
  const reasoning = diagnostics.reasoning.join('').trim().slice(-600)
  return [
    errors.length ? `errors=${JSON.stringify(errors)}` : null,
    notices.length ? `notices=${JSON.stringify(notices)}` : null,
    output ? `last_output=${JSON.stringify(output)}` : null,
    reasoning ? `last_reasoning=${JSON.stringify(reasoning)}` : null,
  ].filter(Boolean).join('; ')
}

async function listJsonlFiles(root) {
  const files = []
  const entries = await readdir(root, { withFileTypes: true }).catch(() => [])
  for (const entry of entries) {
    const fullPath = path.join(root, entry.name)
    if (entry.isDirectory()) {
      files.push(...await listJsonlFiles(fullPath))
    } else if (entry.isFile() && entry.name.endsWith('.jsonl')) {
      files.push(fullPath)
    }
  }
  return files
}

function fixtureFileName(provider, model) {
  return `${provider}-${model.replace(/[^a-zA-Z0-9_.-]+/g, '_')}.jsonl`
}

function fixtureSlug(value) {
  return value.replace(/[^a-zA-Z0-9_.-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 64)
}

function parsedToolEvents(events) {
  const parsed = []
  for (const event of events) {
    try {
      parsed.push(JSON.parse(event))
    } catch {
      // Ignore non-JSON provider blobs; the formatter has separate fallback tests.
    }
  }
  return parsed
}

function toolNamesFromEvents(events) {
  return new Set(parsedToolEvents(events)
    .map((event) => typeof event.tool === 'string' ? event.tool.replace(/[._-]/g, '').toLowerCase() : '')
    .filter(Boolean))
}

function assertRequiredToolFamilies(target, events) {
  const names = toolNamesFromEvents(events)
  const hasRead = [...names].some((name) => name.includes('readartifact') || name === 'read')
  const hasWrite = [...names].some((name) => name.includes('writeartifact') || name === 'write')
  const hasEdit = [...names].some((name) => name.includes('editartifact') || name === 'edit')
  const hasPatch = [...names].some((name) => name.includes('applypatch') || name.includes('patchartifact'))
  const hasMove = [...names].some((name) => name.includes('moveartifact') || name === 'move')
  const hasDelete = [...names].some((name) => name.includes('deleteartifact') || name === 'delete')
  const missing = []
  if (!hasRead) missing.push('read')
  if (!hasWrite) missing.push('write')
  if (!hasEdit) missing.push('edit')
  if (!hasPatch) missing.push('patch')
  if (!hasMove) missing.push('move')
  if (!hasDelete) missing.push('delete')
  if (missing.length > 0) {
    throw new Error(`${target.provider} ${target.model} fixture is missing required tool families: ${missing.join(', ')}`)
  }
  assertRequiredToolSuccesses(target, events)
}

function assertRequiredToolSuccesses(target, events) {
  const parsed = parsedToolEvents(events)
  const requiredMutations = [
    ['write', (name) => name.includes('writeartifact') || name === 'write'],
    ['edit', (name) => name.includes('editartifact') || name === 'edit'],
    ['delete', (name) => name.includes('deleteartifact') || name === 'delete'],
    ['patch', (name) => name.includes('applypatch') || name.includes('patchartifact')],
    ['move', (name) => name.includes('moveartifact') || name === 'move'],
  ]
  for (const [label, matches] of requiredMutations) {
    const completed = parsed.find((event) =>
      event.status === 'completed' &&
      typeof event.tool === 'string' &&
      matches(event.tool.replace(/[._-]/g, '').toLowerCase())
    )
    if (!completed) {
      throw new Error(`${target.provider} ${target.model} fixture is missing completed ${label} event`)
    }
    const payload = parseToolOutputPayload(completed.output)
    if (payload?.applied !== true) {
      throw new Error(`${target.provider} ${target.model} ${label} event was not applied: ${JSON.stringify(payload ?? completed.output)}`)
    }
  }
}

function parseToolOutputPayload(output) {
  const normalized = parseJsonLike(output)
  if (!normalized || typeof normalized !== 'object' || Array.isArray(normalized)) return normalized
  if (normalized.structuredContent) return parseToolOutputPayload(normalized.structuredContent)
  if (Array.isArray(normalized.content)) {
    const text = normalized.content
      .map((entry) => entry && typeof entry === 'object' && typeof entry.text === 'string' ? entry.text : null)
      .find((entry) => entry && entry.trim())
    if (text) return parseToolOutputPayload(text)
  }
  return normalized
}

function parseJsonLike(value) {
  if (typeof value !== 'string') return value
  const trimmed = value.trim()
  if (!trimmed || (!trimmed.startsWith('{') && !trimmed.startsWith('['))) return value
  try {
    return JSON.parse(trimmed)
  } catch {
    return value
  }
}

function runtimeToolNames(provider) {
  if (provider === 'opencode') {
    return {
      read: 'chariox_read_artifact',
      write: 'chariox_write_artifact',
      edit: 'chariox_edit_artifact',
      patch: 'chariox_patch_artifact',
      move: 'chariox_move_artifact',
      delete: 'chariox_delete_artifact',
    }
  }
  return {
    read: 'chariox.read_artifact',
    write: 'chariox.write_artifact',
    edit: 'chariox.edit_artifact',
    patch: 'mcp__chariox__patch_artifact',
    move: 'chariox.move_artifact',
    delete: 'chariox.delete_artifact',
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }

  const ports = makePorts()
  const runId = `${process.pid}-${Date.now()}`
  const runtimeDir = path.join(cliRoot, `.tmp-live-tool-display-fixture-drill-${runId}`)
  const rootDir = path.join(cliRoot, 'target', 'live-tool-display-fixture-drill', runId)
  const workspace = path.join(rootDir, 'workspace')
  const historyDir = path.join(rootDir, 'history')
  const outDir = options.outDir ? path.resolve(options.outDir) : path.join(repoRoot, 'target', 'tool-display-fixtures')
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`

  let succeeded = false
  let client = null
  let sessionId = null
  let requests = null
  let daemonChild = null
  let failure = null
  const failures = []
  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(runtimeDir, { recursive: true })
    await mkdir(workspace, { recursive: true })
    await mkdir(outDir, { recursive: true })
    await writeFile(path.join(workspace, 'seed.txt'), 'TOOL_DISPLAY_FIXTURE_SEED\n', 'utf8')
    await writeFile(path.join(workspace, 'patch-target.txt'), 'before\n', 'utf8')

    const loaded = await loadCliModules(runtimeDir)
    const LocalIpcClient = loaded.LocalIpcClient
    requests = loaded.requests
    const daemonBinary = await resolveBinary(
      path.join(repoRoot, 'apps/kernel/target/debug/chariox-kernel'),
      path.join(repoRoot, 'apps/kernel/Cargo.toml'),
      'chariox-kernel',
    )
    daemonChild = spawn(daemonBinary, [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        CHARIOX_KERNEL_PORT: String(ports.kernelPort),
        CHARIOX_MCP_PORT: String(ports.mcpPort),
        CHARIOX_OPENCODE_PORT: String(ports.opencodePort),
        CHARIOX_CODEX_PORT: String(ports.codexPort),
        CHARIOX_DAEMON_ID: `tool-display-fixtures-${runId}`,
        CHARIOX_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
        CHARIOX_SESSION_HISTORY_DIR: historyDir,
      },
      stdio: ['ignore', 'ignore', 'inherit'],
    })

    await waitForLocalDaemon(LocalIpcClient, kernelUrl, requests.createSessionRequest, requests.endSessionRequest, workspace)
    client = new LocalIpcClient(kernelUrl)
    const catalog = unwrapVariant(await client.send(requests.getProviderCatalogRequest()), 'ProviderCatalog').catalog
    const targets = drillTargets(catalog, options)
    if (targets.length === 0) throw new Error('no provider/model targets selected')
    if (options.listTargets) {
      for (const target of targets) {
        const suffix = target.skipReason ? ` SKIPPED (${target.skipReason})` : ''
        console.log(`[tool-display-fixture] target ${target.provider} ${target.model}${suffix}`)
      }
      succeeded = true
      return
    }

    const session = unwrap(await client.send(requests.createSessionRequest(workspace, workspace)), 'SessionCreated').session
    sessionId = session.id
    const attachment = unwrap(await client.send(requests.attachToSessionRequest(session.id, `tool-display-fixtures-${Date.now()}`)), 'SessionAttached').attachment

    for (const target of targets) {
      if (target.skipReason) {
        console.log(`[tool-display-fixture] ${target.provider} ${target.model}: SKIPPED ${target.skipReason}`)
        continue
      }
      try {
        await runTargetFixture({
          client,
          requests,
          session,
          attachment,
          target,
          workspace,
          historyDir,
          outDir,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
        })
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error)
        failures.push({ target, message })
        console.error(`[tool-display-fixture] ${target.provider} ${target.model}: FAILED ${message}`)
        if (!options.continueOnFailure) {
          throw error
        }
      }
    }

    if (failures.length > 0) {
      throw new Error(`${failures.length} target(s) failed: ${failures.map(({ target, message }) => `${target.provider} ${target.model}: ${message}`).join(' | ')}`)
    }

    await client.send(requests.endSessionRequest(session.id)).catch(() => {})
    sessionId = null
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client && sessionId && requests) await client.send(requests.endSessionRequest(sessionId)).catch(() => {})
    await client?.close?.().catch(() => {})
    await terminateChild(daemonChild)
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: 'tool-display-fixture',
        providers: options.providers.join(','),
        model: options.model,
        providerModels: options.providerModels,
        allModels: options.allModels,
        maxModelsPerProvider: options.maxModelsPerProvider,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        continueOnFailure: options.continueOnFailure,
        listTargets: options.listTargets,
        kernelUrl,
        runtimeDir,
        workspace,
        historyDir,
        outDir,
        failures,
      },
      log: (name, details) => console.log(`[tool-display-fixture] ${name}`, JSON.stringify(details)),
    })
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
    } else {
      console.error(`[tool-display-fixture] kept artifacts at ${rootDir}`)
      console.error(`[tool-display-fixture] kept transient CLI modules at ${runtimeDir}`)
    }
  }
}

async function runTargetFixture({
  client,
  requests,
  session,
  attachment,
  target,
  workspace,
  historyDir,
  outDir,
  timeoutMs,
  pollMs,
}) {
  const tools = runtimeToolNames(target.provider)
  await writeFile(path.join(workspace, 'seed.txt'), 'TOOL_DISPLAY_FIXTURE_SEED\n', 'utf8')
  await writeFile(path.join(workspace, 'patch-target.txt'), 'before\n', 'utf8')
  await writeFile(path.join(workspace, 'delete-target.txt'), 'delete me\n', 'utf8')
  await rm(path.join(workspace, 'write-target.txt'), { force: true }).catch(() => {})
  await rm(path.join(workspace, 'moved-target.txt'), { force: true }).catch(() => {})
  const agent = unwrapVariant(
    await client.send(requests.spawnAgentRequest(
      session.id,
      target.provider,
      `${target.provider}-tool-display-${fixtureSlug(target.model)}`,
      target.model,
      workspace,
      'low',
    )),
    'AgentSpawned',
  ).agent
  await client.send(requests.submitPromptRequest(session.id, attachment.id, agent.id, [
    'This is a Chariox tool-display fixture drill.',
    'You must actually call tools now. Do not describe, plan, or summarize the steps without calling tools.',
    'Do not print XML, JSON, markdown, or pseudo-tool-call text. Use the provider tool-call mechanism only.',
    'Keep all changes inside this disposable workspace.',
    `Step 1: call the Chariox runtime tool \`${tools.read}\` exactly once with JSON arguments {"path":"seed.txt","domain":"text"}.`,
    `Step 2: call \`${tools.write}\` exactly once with JSON arguments {"path":"write-target.txt","content_text":"write-before\\n","domain":"text"}.`,
    `Step 3: call \`${tools.read}\` exactly once with JSON arguments {"path":"write-target.txt","domain":"text"} and remember the returned snapshot_id.`,
    `Step 4: call \`${tools.edit}\` exactly once using that snapshot_id, with JSON arguments {"path":"write-target.txt","old_text":"write-before\\n","new_text":"write-after\\n","domain":"text","snapshot_id":"THE_SNAPSHOT_ID_FROM_STEP_3"}. Replace THE_SNAPSHOT_ID_FROM_STEP_3 with the exact snapshot_id from Step 3.`,
    `Step 5: call \`${tools.delete}\` exactly once with JSON arguments {"path":"delete-target.txt","domain":"text"}.`,
    'Only after the required tool calls succeed, reply with TOOL_DISPLAY_FIXTURE_PHASE_1_DONE.',
    'If you cannot call any tool, reply with TOOL_DISPLAY_FIXTURE_NO_TOOLS and include the exact reason.',
  ].join('\n'), []))
  await waitForAgentIdle({
    client,
    requests,
    sessionId: session.id,
    attachmentId: attachment.id,
    agentId: agent.id,
    timeoutMs,
    pollMs,
  })
  await waitForHistoryMarker({
    client,
    requests,
    sessionId: session.id,
    attachmentId: attachment.id,
    historyDir,
    agentId: agent.id,
    markers: ['TOOL_DISPLAY_FIXTURE_PHASE_1_DONE', 'TOOL_DISPLAY_FIXTURE_NO_TOOLS'],
    timeoutMs,
    pollMs,
  })
  await client.send(requests.submitPromptRequest(session.id, attachment.id, agent.id, [
    'This is phase 2 of the Chariox tool-display fixture drill.',
    'You must actually call the patch and move tools now. Do not describe or summarize without calling them.',
    'Do not print XML, JSON, markdown, or pseudo-tool-call text. Use the provider tool-call mechanism only.',
    `Step 1: call the Chariox runtime tool \`${tools.patch}\` exactly once with JSON arguments {"patch_text":"*** Begin Patch\\n*** Update File: patch-target.txt\\n@@\\n-before\\n+after\\n*** End Patch","domain":"text"}.`,
    `Step 2: call \`${tools.move}\` exactly once with JSON arguments {"from_path":"patch-target.txt","to_path":"moved-target.txt","old_text":"after\\n","new_text":"moved-after\\n","domain":"text"}.`,
    'Only after both tools succeed, reply with TOOL_DISPLAY_FIXTURE_DONE.',
    'If you cannot call the patch or move tool, reply with TOOL_DISPLAY_FIXTURE_NO_PATCH and include the exact reason.',
  ].join('\n'), []))
  await waitForAgentIdle({
    client,
    requests,
    sessionId: session.id,
    attachmentId: attachment.id,
    agentId: agent.id,
    timeoutMs,
    pollMs,
  })
  await waitForHistoryMarker({
    client,
    requests,
    sessionId: session.id,
    attachmentId: attachment.id,
    historyDir,
    agentId: agent.id,
    markers: ['TOOL_DISPLAY_FIXTURE_DONE', 'TOOL_DISPLAY_FIXTURE_NO_PATCH'],
    timeoutMs,
    pollMs,
  })
  const events = await collectProviderToolEvents(historyDir, agent.id)
  if (events.length === 0) {
    const diagnostics = summarizeDiagnostics(await collectProviderDiagnostics(historyDir, agent.id))
    throw new Error(`${target.provider} ${target.model} produced no provider_tool events${diagnostics ? `; ${diagnostics}` : ''}`)
  }
  assertRequiredToolFamilies(target, events)
  const targetPath = path.join(outDir, fixtureFileName(target.provider, target.model))
  await writeFile(targetPath, `${events.join('\n')}\n`, 'utf8')
  await writeRenderedFixture(targetPath, events)
  console.log(`[tool-display-fixture] ${target.provider} ${target.model}: ${events.length} tool events -> ${targetPath}`)
}

async function writeRenderedFixture(targetPath, events) {
  try {
    const { formatToolDisplay } = await import('@chariox/tool-display')
    const rendered = parsedToolEvents(events)
      .map((event) => JSON.stringify(formatToolDisplay(event)))
    if (rendered.length > 0) {
      await writeFile(`${targetPath}.display.jsonl`, `${rendered.join('\n')}\n`, 'utf8')
    }
  } catch {
    // The drill can still capture raw fixtures before the workspace packages are built.
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack || error.message : String(error))
  process.exitCode = 1
})
