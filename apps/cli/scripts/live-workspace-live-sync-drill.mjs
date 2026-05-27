import { spawn } from 'node:child_process'
import { access, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

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

const DEFAULT_KERNEL = 'ws://127.0.0.1:43284'
const DEFAULT_MODEL = 'gpt-5.2'
const DEFAULT_PROVIDERS = ['opencode', 'codex']
const DEFAULT_TIMEOUT_MS = 360_000
const DEFAULT_POLL_MS = 1_000

function parseArgs(argv) {
  const options = {
    kernel: DEFAULT_KERNEL,
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
    providerModels: {},
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    spawnDaemon: true,
    machineRef: null,
    historyDir: null,
    keepArtifactsOnFailure: false,
    positiveOnly: false,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--kernel') options.kernel = argv[++i]
    else if (arg === '--provider') options.providers = [argv[++i]]
    else if (arg === '--providers') options.providers = argv[++i].split(',').map((value) => value.trim()).filter(Boolean)
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++i].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    }
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--no-spawn-daemon') options.spawnDaemon = false
    else if (arg === '--machine-ref') options.machineRef = argv[++i]
    else if (arg === '--history-dir') options.historyDir = argv[++i]
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--positive-only') options.positiveOnly = true
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-workspace-live-sync-drill.mjs [options]',
    '',
    'Runs a live workspace live sync provider drill with isolated daemon/session/workspace lifecycle:',
    '- positive: agents read seed.txt and exercise Arroba write/edit/patch/move/delete tools',
    '- negative: agents are asked to write directly without Arroba; direct output files must not appear',
    '- collision: two agents attempt the same text edit area; exactly one write may land',
    '- external changes: non-overlap stale edits rebase, overlapping stale edits are rejected',
    '',
    'Options:',
    `  --kernel ${DEFAULT_KERNEL}`,
    '  --provider PROVIDER',
    `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
    `  --model ${DEFAULT_MODEL}`,
    '  --provider-model PROVIDER=MODEL (for example opencode=openai/gpt-5.2-codex)',
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    `  --poll-ms ${DEFAULT_POLL_MS}`,
    '  --no-spawn-daemon',
    '  --machine-ref MACHINE_ID_OR_ALIAS (spawn agents on a remote worker machine)',
    '  --history-dir PATH (session history dir when using --no-spawn-daemon)',
    '  --keep-artifacts-on-failure',
    '  --positive-only (stop after the managed read/write/edit/patch/move/delete smoke)',
  ].join('\n'))
}

function makePorts() {
  const base = 57000 + Math.floor(Math.random() * 1000)
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

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (resp, key) => resp?.[key] ?? resp
const unwrapVariant = (resp, ...keys) => keys.map((key) => resp?.[key]).find((value) => value != null) ?? resp

function workspaceLiveSyncSpawnAgentRequest(spawnAgentRequest, sessionId, provider, alias, model, worktreeId, effort, machineRef) {
  if (!machineRef) return spawnAgentRequest(sessionId, provider, alias, model, worktreeId, effort)
  return {
    SpawnAgent: {
      session_id: sessionId,
      provider,
      alias,
      model,
      effort,
      worktree_id: worktreeId,
      machine_ref: machineRef,
    },
  }
}

function modelForProvider(provider, options) {
  const explicit = options.providerModels[provider]
  if (explicit) return explicit
  if (provider === 'opencode' && !options.model.includes('/')) return `openai/${opencodeCodexModel(options.model)}`
  return options.model
}

function opencodeCodexModel(model) {
  if (model.endsWith('-codex')) return model
  if (/^gpt-5\.[23]$/.test(model)) return `${model}-codex`
  return model
}

function workspaceLiveSyncToolNames(provider) {
  if (provider === 'opencode') {
    return {
      read: 'arroba_read_artifact',
      write: 'arroba_write_artifact',
      edit: 'arroba_edit_artifact',
      applyPatch: 'arroba_patch_artifact',
      delete: 'arroba_delete_artifact',
      move: 'arroba_move_artifact',
    }
  }
  return {
    read: 'arroba.read_artifact',
    write: 'arroba.write_artifact',
    edit: 'arroba.edit_artifact',
    applyPatch: 'mcp__arroba__patch_artifact',
    delete: 'arroba.delete_artifact',
    move: 'arroba.move_artifact',
  }
}

function workspaceLiveSyncMoveSourceName(provider) {
  return provider === 'opencode' ? `${provider}-source.txt` : `${provider}-patch.txt`
}

async function spawnWorkspaceLiveSyncPhaseAgents({
  client,
  sessionId,
  providers,
  modelForProvider,
  workspace,
  machineRef,
  spawnAgentRequest,
  aliasSuffix,
}) {
  const agents = []
  for (let index = 0; index < providers.length; index += 1) {
    const provider = providers[index]
    const agent = unwrapVariant(
      await client.send(workspaceLiveSyncSpawnAgentRequest(
        spawnAgentRequest,
        sessionId,
        provider,
        `${provider}-workspace-live-sync-${aliasSuffix}-${index + 1}`,
        modelForProvider(provider),
        workspace,
        'low',
        machineRef,
      )),
      'AgentSpawned',
    ).agent
    agents.push({ provider, agent })
  }
  return agents
}

async function waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace, worktree) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl)
    try {
      const session = unwrap(await probe.send(createSessionRequest(workspace, worktree)), 'SessionCreated').session
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

async function fileExists(filePath) {
  try {
    await access(filePath)
    return true
  } catch {
    return false
  }
}

async function assertFileContent(filePath, expected) {
  const actual = await readFile(filePath, 'utf8')
  if (actual !== expected) {
    throw new Error(`unexpected content for ${filePath}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
  }
  return actual
}

async function assertFileBytes(filePath, expected) {
  const actual = await readFile(filePath)
  const expectedBytes = Buffer.from(expected)
  if (!actual.equals(expectedBytes)) {
    throw new Error(`unexpected bytes for ${filePath}: expected ${expectedBytes.toString('hex')}, got ${actual.toString('hex')}`)
  }
  return actual
}

async function waitForCompletionsAndFiles({ client, sessionId, attachmentId, events, expectedCompletionCount, completionSinceMs = 0, requiredFiles, forbiddenFiles, timeoutMs, pollMs, debugSnapshot }) {
  const started = Date.now()
  let lastRequiredCount = 0
  let lastMissingRequired = requiredFiles
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    const forbiddenExisting = []
    for (const forbiddenFile of forbiddenFiles) {
      if (await fileExists(forbiddenFile)) forbiddenExisting.push(forbiddenFile)
    }
    if (forbiddenExisting.length > 0) {
      throw new Error(`direct write unexpectedly created forbidden files: ${forbiddenExisting.join(', ')}`)
    }

    const requiredExisting = []
    const missingRequired = []
    for (const requiredFile of requiredFiles) {
      if (await fileExists(requiredFile)) requiredExisting.push(requiredFile)
      else missingRequired.push(requiredFile)
    }
    lastRequiredCount = requiredExisting.length
    lastMissingRequired = missingRequired
    const completed = events.filter((event) =>
      event.event === 'assistant_message_completed' &&
      ((event.observed_at_ms ?? 0) >= completionSinceMs)
    )
    if (requiredExisting.length === requiredFiles.length && completed.length >= expectedCompletionCount) {
      return completed
    }
    await sleep(pollMs)
  }
  const debug = debugSnapshot ? `; debug=${JSON.stringify(await debugSnapshot())}` : ''
  throw new Error(`timed out waiting for ${expectedCompletionCount} completions and ${requiredFiles.length} required files; required files present=${lastRequiredCount}; missing=${lastMissingRequired.join(', ')}${debug}`)
}

async function assertFilesAbsent(filePaths, label) {
  const existing = []
  for (const filePath of filePaths) {
    if (await fileExists(filePath)) existing.push(filePath)
  }
  if (existing.length > 0) {
    throw new Error(`${label}: forbidden files exist: ${existing.join(', ')}`)
  }
}

async function waitForFilesAbsent({ filePaths, timeoutMs, pollMs }) {
  const started = Date.now()
  let existing = filePaths
  while (Date.now() - started < timeoutMs) {
    existing = []
    for (const filePath of filePaths) {
      if (await fileExists(filePath)) existing.push(filePath)
    }
    if (existing.length === 0) return
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for workspace live sync files to be absent; still present=${existing.join(', ')}`)
}

async function waitForCompletionCount({ client, sessionId, attachmentId, events, expectedCompletionCount, completionSinceMs = 0, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    const completed = events.filter((event) =>
      event.event === 'assistant_message_completed' &&
      ((event.observed_at_ms ?? 0) >= completionSinceMs)
    )
    if (completed.length >= expectedCompletionCount) return completed
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${expectedCompletionCount} completions`)
}

async function waitForPromptPhase({
  client,
  sessionId,
  attachmentId,
  events,
  expectedCompletionCount,
  completionSinceMs,
  requiredFiles,
  forbiddenFiles,
  timeoutMs,
  pollMs,
  debugSnapshot,
}) {
  await waitForCompletionsAndFiles({
    client,
    sessionId,
    attachmentId,
    events,
    expectedCompletionCount,
    completionSinceMs,
    requiredFiles,
    forbiddenFiles,
    timeoutMs,
    pollMs,
    debugSnapshot,
  })
}

async function historyProviderOutputMarkerGroups({ historyDir, markerGroups, sinceMs }) {
  const remaining = markerGroups.map((markers) => [...markers])
  const outputByKey = new Map()
  const files = (await readdir(historyDir).catch(() => []))
    .filter((file) => file.endsWith('.jsonl'))
    .map((file) => path.join(historyDir, file))
    .sort()

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
      if ((entry.timestamp_ms ?? 0) < sinceMs) continue
      if (entry.kind !== 'provider_output' || typeof entry.text !== 'string') continue
      const key = `${file}:${entry.merge_key ?? entry.timestamp_ms ?? outputByKey.size}`
      outputByKey.set(key, `${outputByKey.get(key) ?? ''}${entry.text}`)
    }
  }

  const outputs = Array.from(outputByKey.values())
  return remaining.filter((markers) => !outputs.some((output) => markers.some((marker) => output.includes(marker))))
}

async function waitForHistoryOutputMarkers({ historyDir, markerGroups, sinceMs, timeoutMs, pollMs }) {
  const started = Date.now()
  let missing = markerGroups
  while (Date.now() - started < timeoutMs) {
    missing = await historyProviderOutputMarkerGroups({ historyDir, markerGroups, sinceMs })
    if (missing.length === 0) return
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for provider output markers: ${missing.map((markers) => markers.join(' or ')).join(', ')}`)
}

async function providerToolUpdatesSince({ historyDir, sinceMs }) {
  const updates = []
  const files = (await readdir(historyDir).catch(() => []))
    .filter((file) => file.endsWith('.jsonl'))
    .map((file) => path.join(historyDir, file))
    .sort()

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
      if ((entry.timestamp_ms ?? 0) < sinceMs) continue
      if (entry.kind !== 'provider_tool' || typeof entry.text !== 'string') continue
      try {
        updates.push(JSON.parse(entry.text))
      } catch {
        continue
      }
    }
  }
  return updates
}

function providerToolUpdateMatches(update, expectation) {
  const tool = String(update.tool ?? '')
  if (!tool.endsWith(expectation.toolSuffix)) return false
  if (update.status !== 'completed') return false
  if (expectation.path != null && update.input?.path !== expectation.path) return false
  if (expectation.fromPath != null && update.input?.from_path !== expectation.fromPath) return false
  if (expectation.toPath != null && update.input?.to_path !== expectation.toPath) return false
  if (expectation.requireApplied === false) return true
  try {
    return parseManagedToolOutput(update.output)?.applied === true
  } catch {
    return false
  }
}

async function waitForManagedToolExpectationsAndFiles({
  client,
  sessionId,
  attachmentId,
  historyDir,
  sinceMs,
  expectations,
  requiredFiles,
  forbiddenFiles,
  timeoutMs,
  pollMs,
  debugSnapshot,
}) {
  const started = Date.now()
  let missingExpectations = expectations
  let lastRequiredCount = 0
  let lastMissingRequired = requiredFiles
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    const forbiddenExisting = []
    for (const forbiddenFile of forbiddenFiles) {
      if (await fileExists(forbiddenFile)) forbiddenExisting.push(forbiddenFile)
    }
    if (forbiddenExisting.length > 0) {
      throw new Error(`direct write unexpectedly created forbidden files: ${forbiddenExisting.join(', ')}`)
    }

    const requiredExisting = []
    const missingRequired = []
    for (const requiredFile of requiredFiles) {
      if (await fileExists(requiredFile)) requiredExisting.push(requiredFile)
      else missingRequired.push(requiredFile)
    }
    lastRequiredCount = requiredExisting.length
    lastMissingRequired = missingRequired

    const updates = await providerToolUpdatesSince({ historyDir, sinceMs })
    missingExpectations = expectations.filter((expectation) =>
      !updates.some((update) => providerToolUpdateMatches(update, expectation))
    )
    if (missingExpectations.length === 0 && requiredExisting.length === requiredFiles.length) return updates
    await sleep(pollMs)
  }
  const missingTools = missingExpectations
    .map((expectation) => `${expectation.toolSuffix}:${expectation.path ?? `${expectation.fromPath ?? ''}->${expectation.toPath ?? ''}`}`)
    .join(', ')
  const debug = debugSnapshot ? `; debug=${JSON.stringify(await debugSnapshot())}` : ''
  throw new Error(`timed out waiting for managed tool results and files; missing tools=${missingTools}; required files present=${lastRequiredCount}; missing=${lastMissingRequired.join(', ')}${debug}`)
}

async function waitForAgentsIdle({ client, sessionId, attachmentId, agentIds, getSessionStateRequest, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    const state = unwrapVariant(await client.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const session = state.session ?? state
    const promptStates = session.prompt_states ?? {}
    const agents = session.agents ?? []
    const allIdle = agentIds.every((agentId) => {
      const agent = agents.find((candidate) => candidate.id === agentId)
      const promptState = promptStates[agentId] ?? {}
      const noPrompt =
        (promptState.active_prompt == null) &&
        ((promptState.queued_prompts ?? []).length === 0)
      return agent && !agent.is_processing && agent.state !== 'Working' && noPrompt
    })
    if (allIdle) return
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for agents to become idle: ${agentIds.join(', ')}`)
}

function parseManagedToolOutput(rawOutput) {
  if (typeof rawOutput !== 'string') return null
  const parsed = JSON.parse(rawOutput)
  if (parsed?.structuredContent) return parsed.structuredContent
  const text = parsed?.content?.find?.((entry) => entry?.type === 'text' && typeof entry.text === 'string')?.text
  if (text) return JSON.parse(text)
  return parsed
}

async function waitForManagedReadSnapshot({ historyDir, artifactPath, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const files = (await readdir(historyDir).catch(() => []))
      .filter((file) => file.endsWith('.jsonl'))
      .map((file) => path.join(historyDir, file))
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
        if (entry.kind !== 'provider_tool' || typeof entry.text !== 'string') continue
        let update
        try {
          update = JSON.parse(entry.text)
        } catch {
          continue
        }
        const tool = String(update.tool ?? '')
        if (!tool.endsWith('read_artifact') || update.status !== 'completed') continue
        if (update.input?.path !== artifactPath) continue
        try {
          const output = parseManagedToolOutput(update.output)
          if (typeof output?.snapshot_id === 'string') return output
        } catch {
          continue
        }
      }
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for managed read snapshot for ${artifactPath}`)
}

async function waitForManagedEditResult({ historyDir, artifactPath, sinceMs, timeoutMs, pollMs }) {
  const results = await waitForManagedEditResults({
    historyDir,
    artifactPath,
    sinceMs,
    count: 1,
    timeoutMs,
    pollMs,
  })
  return results[0]
}

async function waitForManagedEditResults({ historyDir, artifactPath, sinceMs, count, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const results = []
    const files = (await readdir(historyDir).catch(() => []))
      .filter((file) => file.endsWith('.jsonl'))
      .map((file) => path.join(historyDir, file))
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
        if ((entry.timestamp_ms ?? 0) < sinceMs) continue
        if (entry.kind !== 'provider_tool' || typeof entry.text !== 'string') continue
        let update
        try {
          update = JSON.parse(entry.text)
        } catch {
          continue
        }
        const tool = String(update.tool ?? '')
        if (!tool.endsWith('edit_artifact') || !['completed', 'error'].includes(update.status)) continue
        if (update.input?.path !== artifactPath) continue
        try {
          results.push(parseManagedToolOutput(update.output))
        } catch {
          continue
        }
      }
    }
    if (results.length >= count) return results.slice(0, count)
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${count} managed edit results for ${artifactPath}`)
}

async function runLiveCollisionAndExternalChecks({
  client,
  session,
  attachment,
  events,
  agents,
  modelForProvider,
  machineRef,
  workspace,
  outputsDir,
  historyDir,
  timeoutMs,
  pollMs,
  getSessionStateRequest,
  spawnAgentRequest,
  submitPromptRequest,
}) {
  const checks = []

  for (const { provider, agent } of agents) {
    const colliderProvider = agents.find((candidate) => candidate.provider !== provider)?.provider ?? provider
    const overlapPath = path.join(outputsDir, `${provider}-overlap.txt`)
    await writeFile(overlapPath, 'one\nTARGET\nthree\n', 'utf8')
    const collider = unwrapVariant(
      await client.send(workspaceLiveSyncSpawnAgentRequest(
        spawnAgentRequest,
        session.id,
        colliderProvider,
        `${provider}-workspace-live-sync-collider-${colliderProvider}`,
        modelForProvider(colliderProvider),
        workspace,
        'low',
        machineRef,
      )),
      'AgentSpawned',
      'AgentSpawned',
    ).agent
    const firstNewText = `FROM_${provider.toUpperCase()}_A`
    const secondNewText = `FROM_${provider.toUpperCase()}_B`
    const tools = workspaceLiveSyncToolNames(provider)
    const overlapSameAreaEditStartedAt = Date.now()
    for (const [editAgent, label, newText] of [[agent, 'A', firstNewText], [collider, 'B', secondNewText]]) {
      const prompt = [
        'This is a live Arroba workspace live sync overlapping-writer drill.',
        'Use only Arroba workspace live sync. Do not use shell commands or native filesystem writes.',
        `First call \`${tools.read}\` exactly once with JSON arguments {"path":"outputs/${provider}-overlap.txt","domain":"text"}.`,
        `Then call \`${tools.edit}\` exactly once using the \`snapshot_id\` from that read, with old_text "TARGET" and the requested new_text.`,
        'Do not reread or retry if the managed edit is rejected.',
        `The edit arguments must target {"path":"outputs/${provider}-overlap.txt","old_text":"TARGET","new_text":${JSON.stringify(newText)},"domain":"text"} plus the read snapshot_id.`,
        `Then reply exactly ${provider.toUpperCase()}_OVERLAP_${label}_DONE if applied, or ${provider.toUpperCase()}_OVERLAP_${label}_BLOCKED if rejected.`,
      ].join('\n')
      await client.send(submitPromptRequest(session.id, attachment.id, editAgent.id, prompt, []))
    }
    await waitForManagedEditResults({
      historyDir,
      artifactPath: `outputs/${provider}-overlap.txt`,
      sinceMs: overlapSameAreaEditStartedAt,
      // Remote lease histories can mirror one side of a cross-provider collision
      // without the matching provider_tool record even though the file mutation
      // has landed. The final content assertion below still verifies that one
      // write won and the losing edit did not corrupt the file.
      count: machineRef ? 1 : 2,
      timeoutMs,
      pollMs,
    })
    await waitForHistoryOutputMarkers({
      historyDir,
      markerGroups: [
        [`${provider.toUpperCase()}_OVERLAP_A_DONE`, `${provider.toUpperCase()}_OVERLAP_A_BLOCKED`],
        [`${provider.toUpperCase()}_OVERLAP_B_DONE`, `${provider.toUpperCase()}_OVERLAP_B_BLOCKED`],
      ],
      sinceMs: overlapSameAreaEditStartedAt,
      timeoutMs,
      pollMs,
    })
    if (!machineRef) {
      await waitForAgentsIdle({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentIds: [agent.id, collider.id],
        getSessionStateRequest,
        timeoutMs,
        pollMs,
      })
    }
    const overlapContent = await readFile(overlapPath, 'utf8')
    const allowedOverlapContents = new Set([
      `one\n${firstNewText}\nthree\n`,
      `one\n${secondNewText}\nthree\n`,
    ])
    if (!allowedOverlapContents.has(overlapContent)) {
      throw new Error(`overlap drill produced unexpected content for ${provider}: ${JSON.stringify(overlapContent)}`)
    }
    checks.push({
      provider,
      scenario: 'overlap_same_area',
      relativePath: `outputs/${provider}-overlap.txt`,
      finalContent: overlapContent,
      expectedOneOf: Array.from(allowedOverlapContents),
    })

    const spawnCheckAgent = async (suffix) => {
      if (!machineRef) return agent
      return unwrapVariant(
        await client.send(workspaceLiveSyncSpawnAgentRequest(
          spawnAgentRequest,
          session.id,
          provider,
          `${provider}-workspace-live-sync-${suffix}`,
          modelForProvider(provider),
          workspace,
          'low',
          machineRef,
        )),
        'AgentSpawned',
      ).agent
    }

    const nonOverlapPath = path.join(outputsDir, `${provider}-external-nonoverlap.txt`)
    const nonOverlapBase = 'header\nalpha\nTARGET\nomega\nfooter\n'
    const nonOverlapExternallyChanged = 'intro\nheader\nalpha\nTARGET\nomega\nfooter\noutro\n'
    const nonOverlapExpected = 'intro\nheader\nalpha\nREPLACED\nomega\nfooter\noutro\n'
    await writeFile(nonOverlapPath, nonOverlapBase, 'utf8')
    const nonOverlapReadAgent = await spawnCheckAgent('external-nonoverlap-read')
    const nonOverlapReadStartedAt = Date.now()
    await client.send(submitPromptRequest(session.id, attachment.id, nonOverlapReadAgent.id, [
      'This is a live Arroba workspace live sync external non-overlap drill.',
      'Use only Arroba workspace live sync.',
      `Call \`${tools.read}\` exactly once with JSON arguments {"path":"outputs/${provider}-external-nonoverlap.txt","domain":"text"}.`,
      `Remember the returned snapshot_id for the next turn. Then reply exactly ${provider.toUpperCase()}_EXTERNAL_NONOVERLAP_READ_DONE.`,
    ].join('\n'), []))
    const nonOverlapRead = await waitForManagedReadSnapshot({
      historyDir,
      artifactPath: `outputs/${provider}-external-nonoverlap.txt`,
      timeoutMs,
      pollMs,
    })
    await waitForHistoryOutputMarkers({
      historyDir,
      markerGroups: [[`${provider.toUpperCase()}_EXTERNAL_NONOVERLAP_READ_DONE`]],
      sinceMs: nonOverlapReadStartedAt,
      timeoutMs,
      pollMs,
    })
    if (nonOverlapRead.content_text !== nonOverlapBase) {
      throw new Error(`external non-overlap read happened after external write for ${provider}: ${JSON.stringify(nonOverlapRead.content_text)}`)
    }
    if (!machineRef) {
      await waitForAgentsIdle({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentIds: [nonOverlapReadAgent.id],
        getSessionStateRequest,
        timeoutMs,
        pollMs,
      })
    }
    await writeFile(nonOverlapPath, nonOverlapExternallyChanged, 'utf8')
    const nonOverlapEditStartedAt = Date.now()
    const nonOverlapEditAgent = await spawnCheckAgent('external-nonoverlap-edit')
    await client.send(submitPromptRequest(session.id, attachment.id, nonOverlapEditAgent.id, [
      'Continue the external non-overlap drill.',
      'Use only Arroba workspace live sync. Do not reread the artifact.',
      `Use this exact snapshot_id: ${nonOverlapRead.snapshot_id}`,
      `Call \`${tools.edit}\` exactly once with JSON arguments {"path":"outputs/${provider}-external-nonoverlap.txt","old_text":"TARGET","new_text":"REPLACED","domain":"text","snapshot_id":${JSON.stringify(nonOverlapRead.snapshot_id)}}.`,
      `Then reply exactly ${provider.toUpperCase()}_EXTERNAL_NONOVERLAP_EDIT_DONE.`,
    ].join('\n'), []))
    const nonOverlapEdit = await waitForManagedEditResult({
      historyDir,
      artifactPath: `outputs/${provider}-external-nonoverlap.txt`,
      sinceMs: nonOverlapEditStartedAt,
      timeoutMs,
      pollMs,
    })
    await waitForHistoryOutputMarkers({
      historyDir,
      markerGroups: [[`${provider.toUpperCase()}_EXTERNAL_NONOVERLAP_EDIT_DONE`]],
      sinceMs: nonOverlapEditStartedAt,
      timeoutMs,
      pollMs,
    })
    if (nonOverlapEdit?.applied !== true) {
      throw new Error(`external non-overlap edit was not applied for ${provider}: ${JSON.stringify(nonOverlapEdit)}`)
    }
    await assertFileContent(nonOverlapPath, nonOverlapExpected)
    checks.push({
      provider,
      scenario: 'external_non_overlap_rebase',
      relativePath: `outputs/${provider}-external-nonoverlap.txt`,
      finalContent: nonOverlapExpected,
    })

    const overlapExternalPath = path.join(outputsDir, `${provider}-external-overlap.txt`)
    const externalOverlapBase = 'one\nTARGET\nthree\n'
    const externalOverlapExpected = 'one\nEXTERNAL\nthree\n'
    await writeFile(overlapExternalPath, externalOverlapBase, 'utf8')
    const overlapReadAgent = await spawnCheckAgent('external-overlap-read')
    const overlapReadStartedAt = Date.now()
    await client.send(submitPromptRequest(session.id, attachment.id, overlapReadAgent.id, [
      'This is a live Arroba workspace live sync external overlap drill.',
      'Use only Arroba workspace live sync.',
      `Call \`${tools.read}\` exactly once with JSON arguments {"path":"outputs/${provider}-external-overlap.txt","domain":"text"}.`,
      `Remember the returned snapshot_id for the next turn. Then reply exactly ${provider.toUpperCase()}_EXTERNAL_OVERLAP_READ_DONE.`,
    ].join('\n'), []))
    const overlapRead = await waitForManagedReadSnapshot({
      historyDir,
      artifactPath: `outputs/${provider}-external-overlap.txt`,
      timeoutMs,
      pollMs,
    })
    await waitForHistoryOutputMarkers({
      historyDir,
      markerGroups: [[`${provider.toUpperCase()}_EXTERNAL_OVERLAP_READ_DONE`]],
      sinceMs: overlapReadStartedAt,
      timeoutMs,
      pollMs,
    })
    if (overlapRead.content_text !== externalOverlapBase) {
      throw new Error(`external overlap read happened after external write for ${provider}: ${JSON.stringify(overlapRead.content_text)}`)
    }
    if (!machineRef) {
      await waitForAgentsIdle({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentIds: [overlapReadAgent.id],
        getSessionStateRequest,
        timeoutMs,
        pollMs,
      })
    }
    await writeFile(overlapExternalPath, externalOverlapExpected, 'utf8')
    const overlapEditStartedAt = Date.now()
    const overlapEditAgent = await spawnCheckAgent('external-overlap-edit')
    await client.send(submitPromptRequest(session.id, attachment.id, overlapEditAgent.id, [
      'Continue the external overlap drill.',
      'Use only Arroba workspace live sync. Do not reread the artifact and do not retry if rejected.',
      `Use this exact snapshot_id: ${overlapRead.snapshot_id}`,
      `Call \`${tools.edit}\` exactly once with JSON arguments {"path":"outputs/${provider}-external-overlap.txt","old_text":"TARGET","new_text":"AGENT","domain":"text","snapshot_id":${JSON.stringify(overlapRead.snapshot_id)}}.`,
      `Then reply exactly ${provider.toUpperCase()}_EXTERNAL_OVERLAP_BLOCKED if rejected, or ${provider.toUpperCase()}_EXTERNAL_OVERLAP_UNEXPECTED_APPLIED if applied.`,
    ].join('\n'), []))
    const overlapEdit = await waitForManagedEditResult({
      historyDir,
      artifactPath: `outputs/${provider}-external-overlap.txt`,
      sinceMs: overlapEditStartedAt,
      timeoutMs,
      pollMs,
    })
    await waitForHistoryOutputMarkers({
      historyDir,
      markerGroups: [[
        `${provider.toUpperCase()}_EXTERNAL_OVERLAP_BLOCKED`,
        `${provider.toUpperCase()}_EXTERNAL_OVERLAP_UNEXPECTED_APPLIED`,
      ]],
      sinceMs: overlapEditStartedAt,
      timeoutMs,
      pollMs,
    })
    if (overlapEdit?.applied !== false || overlapEdit?.reason?.kind !== 'conflict') {
      throw new Error(`external overlap edit was not rejected as a conflict for ${provider}: ${JSON.stringify(overlapEdit)}`)
    }
    await assertFileContent(overlapExternalPath, externalOverlapExpected)
    checks.push({
      provider,
      scenario: 'external_overlap_rejected',
      relativePath: `outputs/${provider}-external-overlap.txt`,
      finalContent: externalOverlapExpected,
    })
  }

  return checks
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  if (options.providers.length === 0) {
    throw new Error('at least one provider is required')
  }

  const runtimeDir = path.join(cliRoot, '.tmp-live-workspace-live-sync-drill')
  // Keep the live workspace out of OS temp directories: Codex read-only mode may
  // allow TMPDIR writes, which would make the negative direct-write probe invalid.
  const rootDir = path.join(cliRoot, 'target', 'live-workspace-live-sync-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const outputsDir = path.join(workspace, 'outputs')
  await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(runtimeDir, { recursive: true })
  await mkdir(outputsDir, { recursive: true })
  await writeFile(path.join(workspace, 'seed.txt'), 'seed-value-42\n', 'utf8')
  for (const provider of options.providers) {
    await writeFile(path.join(outputsDir, `${provider}-delete-me.txt`), 'delete-me\n', 'utf8')
    await writeFile(path.join(outputsDir, `${provider}-opaque-delete-me.bin`), Buffer.from([9, 8, 7]))
  }

  const { LocalIpcClient, requests } = await loadCliModules(runtimeDir)
  const {
    attachToSessionRequest,
    createSessionRequest,
    endSessionRequest,
    getSessionStateRequest,
    listProviderProcessesRequest,
    setWorkspaceLiveSyncModeRequest,
    spawnAgentRequest,
    submitPromptRequest,
  } = requests

  let daemonChild = null
  let kernelUrl = options.kernel
  const startedAt = Date.now()
  const historyDir = options.historyDir ?? path.join(rootDir, 'history')
  const xdgConfigHome = path.join(rootDir, 'xdg-config')
  const xdgStateHome = path.join(rootDir, 'xdg-state')
  let succeeded = false
  if (options.spawnDaemon) {
    const ports = makePorts()
    kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
    const daemonBinary = await resolveBinary(
      path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel'),
      path.join(repoRoot, 'apps/kernel/Cargo.toml'),
      'arroba-kernel',
    )
    daemonChild = spawn(daemonBinary, [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        ARROBA_KERNEL_PORT: String(ports.kernelPort),
        ARROBA_MCP_PORT: String(ports.mcpPort),
        ARROBA_OPENCODE_PORT: String(ports.opencodePort),
        ARROBA_CODEX_PORT: String(ports.codexPort),
        ARROBA_DAEMON_ID: `workspace-live-sync-drill-${process.pid}-${Date.now()}`,
        ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
        ARROBA_SESSION_HISTORY_DIR: historyDir,
        XDG_CONFIG_HOME: xdgConfigHome,
        XDG_STATE_HOME: xdgStateHome,
      },
      stdio: ['ignore', 'ignore', 'inherit'],
    })
    await waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace, workspace)
  }

  const client = new LocalIpcClient(kernelUrl)
  const events = []
  let sessionId = null
  try {
    if (setWorkspaceLiveSyncModeRequest) {
      await client.send(setWorkspaceLiveSyncModeRequest('managed'))
    }
    const session = unwrap(await client.send(createSessionRequest(workspace, workspace)), 'SessionCreated').session
    sessionId = session.id
    const attachment = unwrap(
      await client.send(attachToSessionRequest(session.id, `workspace-live-sync-drill-${Date.now()}`)),
      'SessionAttached',
    ).attachment
    client.onKernelEvent((event) => events.push({ ...event, observed_at_ms: Date.now() }))
    await client.subscribeToKernelEvents(session.id, attachment.id)

    const agents = await spawnWorkspaceLiveSyncPhaseAgents({
      client,
      sessionId: session.id,
      providers: options.providers,
      modelForProvider: (provider) => modelForProvider(provider, options),
      workspace,
      machineRef: options.machineRef,
      spawnAgentRequest,
      aliasSuffix: 'positive',
    })
    const debugSessionSnapshot = async () => {
      const state = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
      const currentSession = state.session ?? state
      const promptStates = currentSession.prompt_states ?? {}
      return {
        events: events.reduce((counts, event) => {
          counts[event.event] = (counts[event.event] ?? 0) + 1
          return counts
        }, {}),
        lastTerminalOutput: events
          .filter((event) => event.event === 'terminal_output')
          .slice(-3)
          .map((event) => String(event.text ?? event.data ?? event.output ?? '').slice(0, 500)),
        agents: (currentSession.agents ?? []).map((agent) => ({
          id: agent.id,
          alias: agent.alias,
          state: agent.state,
          is_processing: agent.is_processing,
          provider_run_id: agent.provider_run_id ?? null,
          prompt: {
            active: promptStates[agent.id]?.active_prompt != null,
            queued: (promptStates[agent.id]?.queued_prompts ?? []).length,
          },
        })),
      }
    }

    const positiveFiles = options.providers.map((provider) => path.join(outputsDir, `${provider}.txt`))
    const movedFiles = options.providers.map((provider) => path.join(outputsDir, `${provider}-moved.txt`))
    const opaqueMovedFiles = options.providers.map((provider) => path.join(outputsDir, `${provider}-opaque-moved.bin`))
    const directFiles = options.providers.map((provider) => path.join(outputsDir, `${provider}-direct.txt`))
    const runPositivePromptPhase = async ({ provider, agent, prompt, requiredFiles, label, marker, managedToolExpectations = [] }) => {
      const completionSinceMs = Date.now()
      await client.send(submitPromptRequest(session.id, attachment.id, agent.id, prompt, []))
      if (managedToolExpectations.length > 0) {
        await waitForManagedToolExpectationsAndFiles({
          client,
          sessionId: session.id,
          attachmentId: attachment.id,
          historyDir,
          sinceMs: completionSinceMs,
          expectations: managedToolExpectations,
          requiredFiles,
          forbiddenFiles: directFiles,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
          debugSnapshot: debugSessionSnapshot,
        })
      } else {
        await waitForPromptPhase({
          client,
          sessionId: session.id,
          attachmentId: attachment.id,
          events,
          expectedCompletionCount: 1,
          completionSinceMs,
          requiredFiles,
          forbiddenFiles: directFiles,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
          debugSnapshot: debugSessionSnapshot,
        })
        await waitForHistoryOutputMarkers({
          historyDir,
          markerGroups: [[marker]],
          sinceMs: completionSinceMs,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
        })
      }
      if (!options.machineRef) {
        await waitForAgentsIdle({
          client,
          sessionId: session.id,
          attachmentId: attachment.id,
          agentIds: [agent.id],
          getSessionStateRequest,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
        })
        await sleep(4_000)
      }
      await assertFilesAbsent(directFiles, `${provider} ${label} direct-write check`)
    }

    for (const { provider, agent } of agents) {
      const written = `${provider}-workspace-live-sync-write-ok: seed-value-42\n`
      const edited = `${provider}-workspace-live-sync-edit-ok: seed-value-42\n`
      const sourceName = workspaceLiveSyncMoveSourceName(provider)
      const patchInitial = provider === 'opencode' ? `source-start-${provider}\n` : `patch-start-${provider}\n`
      const patchMoved = provider === 'opencode' ? patchInitial : `patch-moved-${provider}\n`
      const tools = workspaceLiveSyncToolNames(provider)
      const patchText = [
        '*** Begin Patch',
        `*** Add File: outputs/${sourceName}`,
        `+${patchInitial.trimEnd()}`,
        '*** End Patch',
      ].join('\n')
      const opaqueBytes = Buffer.from([0, provider.length, 255, 10])
      const opaqueBase64 = opaqueBytes.toString('base64')
      await runPositivePromptPhase({
        provider,
        agent,
        label: 'text read/write',
        marker: `${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_TEXT_WRITE_DONE`,
        managedToolExpectations: [
          { toolSuffix: 'write_artifact', path: `outputs/${provider}.txt` },
        ],
        requiredFiles: [path.join(outputsDir, `${provider}.txt`)],
        prompt: [
          'This is a live Arroba workspace live sync positive text read/write smoke test.',
          'Do not use shell commands, direct filesystem writes, native patch/edit tools, or any non-Arroba file write path.',
          'Use only the Arroba MCP/runtime tools for file I/O.',
          `Step 1: call \`${tools.read}\` exactly once with JSON arguments {"path":"seed.txt","domain":"text"}.`,
          `Step 2: call \`${tools.write}\` exactly once with JSON arguments {"path":"outputs/${provider}.txt","content_text":${JSON.stringify(written)},"domain":"text"}.`,
          `Only after both steps succeed and outputs/${provider}.txt exists, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_TEXT_WRITE_DONE and nothing else.`,
          `If any workspace live sync tool reports applied:false or an error, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_FAILED and stop.`,
        ].join('\n'),
      })
      await assertFileContent(path.join(outputsDir, `${provider}.txt`), written)

      await runPositivePromptPhase({
        provider,
        agent,
        label: 'text read/edit',
        marker: `${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_TEXT_EDIT_DONE`,
        managedToolExpectations: [
          { toolSuffix: 'edit_artifact', path: `outputs/${provider}.txt` },
        ],
        requiredFiles: [path.join(outputsDir, `${provider}.txt`)],
        prompt: [
          'This is a live Arroba workspace live sync positive text edit smoke test.',
          'Do not use shell commands, direct filesystem writes, native patch/edit tools, or any non-Arroba file write path.',
          'Use only the Arroba MCP/runtime tools for file I/O.',
          `Step 1: call \`${tools.read}\` exactly once with JSON arguments {"path":"outputs/${provider}.txt","domain":"text"} and remember the returned snapshot_id.`,
          `Step 2: call \`${tools.edit}\` exactly once with JSON arguments {"path":"outputs/${provider}.txt","old_text":${JSON.stringify(written)},"new_text":${JSON.stringify(edited)},"domain":"text","snapshot_id":"THE_OUTPUT_SNAPSHOT_ID_FROM_STEP_1"}. Replace THE_OUTPUT_SNAPSHOT_ID_FROM_STEP_1 with the exact snapshot_id from step 1.`,
          `Only after the edit succeeds and outputs/${provider}.txt contains the new text, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_TEXT_EDIT_DONE and nothing else.`,
          `If any workspace live sync tool reports applied:false or an error, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_FAILED and stop.`,
        ].join('\n'),
      })
      await assertFileContent(path.join(outputsDir, `${provider}.txt`), edited)

      const patchPrompt = [
          'This is a live Arroba workspace live sync positive text patch smoke test.',
          'Do not use shell commands, direct filesystem writes, native patch/edit tools, or any non-Arroba file write path.',
          'Use only the Arroba MCP/runtime tools for file I/O.',
          `Call \`${tools.applyPatch}\` exactly once with JSON arguments {"patch_text":${JSON.stringify(patchText)},"domain":"text"}.`,
          `Only after outputs/${sourceName} exists, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_PATCH_DONE and nothing else.`,
          `If the workspace live sync tool reports applied:false or an error, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_FAILED and stop.`,
        ].join('\n')
      if (provider === 'opencode') {
        await writeFile(path.join(outputsDir, sourceName), patchInitial, 'utf8')
      } else {
        await runPositivePromptPhase({
          provider,
          agent,
          label: 'text patch',
          marker: `${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_PATCH_DONE`,
          managedToolExpectations: [
            { toolSuffix: 'patch_artifact' },
          ],
          requiredFiles: [path.join(outputsDir, sourceName)],
          prompt: patchPrompt,
        })
      }
      await assertFileContent(path.join(outputsDir, sourceName), patchInitial)

      await runPositivePromptPhase({
        provider,
        agent,
        label: 'text move',
        marker: `${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_MOVE_DONE`,
        managedToolExpectations: [
          {
            toolSuffix: 'move_artifact',
            fromPath: `outputs/${sourceName}`,
            toPath: `outputs/${provider}-moved.txt`,
          },
        ],
        requiredFiles: [path.join(outputsDir, `${provider}-moved.txt`)],
        prompt: [
          'This is a live Arroba workspace live sync positive move smoke test.',
          'Do not use shell commands, direct filesystem writes, native patch/edit tools, or any non-Arroba file write path.',
          'Use only the Arroba MCP/runtime tools for file I/O.',
          provider === 'opencode'
            ? `Call \`${tools.move}\` exactly once with JSON arguments {"from_path":"outputs/${sourceName}","to_path":"outputs/${provider}-moved.txt","domain":"text"}.`
            : `Call \`${tools.move}\` exactly once with JSON arguments {"from_path":"outputs/${sourceName}","to_path":"outputs/${provider}-moved.txt","old_text":${JSON.stringify(patchInitial)},"new_text":${JSON.stringify(patchMoved)},"domain":"text"}.`,
          `Only after outputs/${provider}-moved.txt exists and outputs/${sourceName} is gone, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_MOVE_DONE and nothing else.`,
          `If the workspace live sync tool reports applied:false or an error, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_FAILED and stop.`,
        ].join('\n'),
      })
      await assertFileContent(path.join(outputsDir, `${provider}-moved.txt`), patchMoved)
      if (await fileExists(path.join(outputsDir, sourceName))) {
        throw new Error(`managed move left source file behind: outputs/${sourceName}`)
      }

      await runPositivePromptPhase({
        provider,
        agent,
        label: 'opaque write',
        marker: `${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_OPAQUE_WRITE_DONE`,
        managedToolExpectations: [
          { toolSuffix: 'write_artifact', path: `outputs/${provider}-opaque.bin` },
        ],
        requiredFiles: [path.join(outputsDir, `${provider}-opaque.bin`)],
        prompt: [
          'This is a live Arroba workspace live sync positive opaque write smoke test.',
          'Do not use shell commands, direct filesystem writes, native patch/edit tools, or any non-Arroba file write path.',
          'Use only the Arroba MCP/runtime tools for file I/O.',
          'Every tool call in this turn must use `"domain":"opaque"`.',
          `Call \`${tools.write}\` exactly once with JSON arguments {"path":"outputs/${provider}-opaque.bin","content_base64":${JSON.stringify(opaqueBase64)},"domain":"opaque"}.`,
          `Only after outputs/${provider}-opaque.bin exists, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_OPAQUE_WRITE_DONE and nothing else.`,
          `If the workspace live sync tool reports applied:false or an error, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_FAILED and stop.`,
        ].join('\n'),
      })
      await assertFileBytes(path.join(outputsDir, `${provider}-opaque.bin`), opaqueBytes)

      await runPositivePromptPhase({
        provider,
        agent,
        label: 'opaque read/move',
        marker: `${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_OPAQUE_MOVE_DONE`,
        managedToolExpectations: [
          {
            toolSuffix: 'move_artifact',
            fromPath: `outputs/${provider}-opaque.bin`,
            toPath: `outputs/${provider}-opaque-moved.bin`,
          },
        ],
        requiredFiles: [path.join(outputsDir, `${provider}-opaque-moved.bin`)],
        prompt: [
          'This is a live Arroba workspace live sync positive opaque read/move smoke test.',
          'Do not use shell commands, direct filesystem writes, native patch/edit tools, or any non-Arroba file write path.',
          'Use only the Arroba MCP/runtime tools for file I/O.',
          'Every tool call in this turn must use `"domain":"opaque"`.',
          'Do not include old_text, new_text, content_text, or patch_text in the opaque move call.',
          `Step 1: call \`${tools.read}\` exactly once with JSON arguments {"path":"outputs/${provider}-opaque.bin","domain":"opaque"} and verify the returned content_base64 is ${JSON.stringify(opaqueBase64)}.`,
          `Step 2: call \`${tools.move}\` exactly once with JSON arguments {"from_path":"outputs/${provider}-opaque.bin","to_path":"outputs/${provider}-opaque-moved.bin","domain":"opaque"}.`,
          `Only after outputs/${provider}-opaque-moved.bin exists and outputs/${provider}-opaque.bin is gone, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_OPAQUE_MOVE_DONE and nothing else.`,
          `If any workspace live sync tool reports applied:false or an error, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_FAILED and stop.`,
        ].join('\n'),
      })
      await assertFileBytes(path.join(outputsDir, `${provider}-opaque-moved.bin`), opaqueBytes)
      if (await fileExists(path.join(outputsDir, `${provider}-opaque.bin`))) {
        throw new Error(`managed opaque move left source file behind: outputs/${provider}-opaque.bin`)
      }
    }
    if (!options.machineRef) {
      await waitForAgentsIdle({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentIds: agents.map(({ agent }) => agent.id),
        getSessionStateRequest,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
    }
    for (const provider of options.providers) {
      await assertFileContent(
        path.join(outputsDir, `${provider}.txt`),
        `${provider}-workspace-live-sync-edit-ok: seed-value-42\n`,
      )
      await assertFileContent(
        path.join(outputsDir, `${provider}-moved.txt`),
        provider === 'opencode' ? `source-start-${provider}\n` : `patch-moved-${provider}\n`,
      )
      await assertFileBytes(path.join(outputsDir, `${provider}-opaque-moved.bin`), [0, provider.length, 255, 10])
      const sourceName = workspaceLiveSyncMoveSourceName(provider)
      if (await fileExists(path.join(outputsDir, sourceName))) {
        throw new Error(`managed move left source file behind: outputs/${sourceName}`)
      }
      if (await fileExists(path.join(outputsDir, `${provider}-opaque.bin`))) {
        throw new Error(`managed opaque move left source file behind: outputs/${provider}-opaque.bin`)
      }
    }

    if (options.positiveOnly) {
      const files = []
      for (const provider of options.providers) {
        const filePath = path.join(outputsDir, `${provider}.txt`)
        files.push({
          provider,
          relativePath: `outputs/${provider}.txt`,
          content: await readFile(filePath, 'utf8'),
          movedRelativePath: `outputs/${provider}-moved.txt`,
          movedContent: await readFile(path.join(outputsDir, `${provider}-moved.txt`), 'utf8'),
          opaqueMovedRelativePath: `outputs/${provider}-opaque-moved.bin`,
          opaqueMovedHex: (await readFile(path.join(outputsDir, `${provider}-opaque-moved.bin`))).toString('hex'),
          patchSourceFileExists: await fileExists(path.join(outputsDir, workspaceLiveSyncMoveSourceName(provider))),
          opaqueMoveSourceFileExists: await fileExists(path.join(outputsDir, `${provider}-opaque.bin`)),
          deletedFileExists: await fileExists(path.join(outputsDir, `${provider}-delete-me.txt`)),
          opaqueDeletedFileExists: await fileExists(path.join(outputsDir, `${provider}-opaque-delete-me.bin`)),
          directWriteFileExists: await fileExists(path.join(outputsDir, `${provider}-direct.txt`)),
        })
      }
      const finalState = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
      const processes = unwrapVariant(await client.send(listProviderProcessesRequest()), 'ProviderProcessesListed').processes || []
      console.log(JSON.stringify({
        status: 'ok',
        mode: 'workspace-live-sync-live-drill',
        kernelUrl,
        machineRef: options.machineRef,
        workspace,
        providers: options.providers,
        model: options.model,
        providerModels: Object.fromEntries(options.providers.map((provider) => [
          provider,
          modelForProvider(provider, options),
        ])),
        durationMs: Date.now() - startedAt,
        agents: agents.map(({ provider, agent }) => ({
          id: agent.id,
          alias: agent.alias,
          provider,
        })),
        completionCount: events.filter((event) => event.event === 'assistant_message_completed').length,
        terminalEventCount: events.filter((event) => event.event === 'terminal_output').length,
        files,
        collisionAndExternalChecks: [],
        providerProcesses: processes.map((process) => ({
          processId: process.process_id,
          provider: process.provider,
          pid: process.pid ?? null,
          ownerRunIds: process.owner_provider_run_ids || [],
        })),
        focusedAgentId: finalState.session?.focused_agent_id ?? finalState.focused_agent_id ?? null,
      }, null, 2))
      succeeded = true
      return
    }

    const deleteAgents = []
    if (options.machineRef) {
      for (const { provider } of agents) {
        const deleteAgent = unwrapVariant(
          await client.send(workspaceLiveSyncSpawnAgentRequest(
            spawnAgentRequest,
            session.id,
            provider,
            `${provider}-workspace-live-sync-delete`,
            modelForProvider(provider, options),
            workspace,
            'low',
            options.machineRef,
          )),
          'AgentSpawned',
        ).agent
        deleteAgents.push({ provider, agent: deleteAgent })
      }
    } else {
      deleteAgents.push(...agents)
    }

    const deletePrompts = []
    for (const { provider, agent } of deleteAgents) {
      const tools = workspaceLiveSyncToolNames(provider)
      const prompt = [
        'This is a live Arroba workspace live sync delete smoke test.',
        'Do not use shell commands, direct filesystem writes, native patch/edit tools, or any non-Arroba file write path.',
        `Call \`${tools.delete}\` with JSON arguments {"path":"outputs/${provider}-delete-me.txt","domain":"text"} to delete the pre-existing delete-me file.`,
        `Then call \`${tools.delete}\` with JSON arguments {"path":"outputs/${provider}-opaque-delete-me.bin","domain":"opaque"} to delete the pre-existing opaque delete-me file.`,
        `After the tool succeeds, reply exactly ${provider.toUpperCase()}_WORKSPACE_LIVE_SYNC_DELETE_DONE and nothing else.`,
      ].join('\n')
      deletePrompts.push({ provider, agent, prompt })
    }
    if (options.machineRef) {
      for (const { provider, agent, prompt } of deletePrompts) {
        const completionSinceMs = Date.now()
        await client.send(submitPromptRequest(session.id, attachment.id, agent.id, prompt, []))
        await waitForManagedToolExpectationsAndFiles({
          client,
          sessionId: session.id,
          attachmentId: attachment.id,
          historyDir,
          sinceMs: completionSinceMs,
          expectations: [
            { toolSuffix: 'delete_artifact', path: `outputs/${provider}-delete-me.txt` },
            { toolSuffix: 'delete_artifact', path: `outputs/${provider}-opaque-delete-me.bin` },
          ],
          requiredFiles: [],
          forbiddenFiles: directFiles,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
          debugSnapshot: debugSessionSnapshot,
        })
        await waitForFilesAbsent({
          filePaths: [
            path.join(outputsDir, `${provider}-delete-me.txt`),
            path.join(outputsDir, `${provider}-opaque-delete-me.bin`),
          ],
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
        })
      }
    } else {
      const completionSinceMs = Date.now()
      for (const { agent, prompt } of deletePrompts) {
        await client.send(submitPromptRequest(session.id, attachment.id, agent.id, prompt, []))
      }
      for (const { provider } of deletePrompts) {
        await waitForManagedToolExpectationsAndFiles({
          client,
          sessionId: session.id,
          attachmentId: attachment.id,
          historyDir,
          sinceMs: completionSinceMs,
          expectations: [
            { toolSuffix: 'delete_artifact', path: `outputs/${provider}-delete-me.txt` },
            { toolSuffix: 'delete_artifact', path: `outputs/${provider}-opaque-delete-me.bin` },
          ],
          requiredFiles: [],
          forbiddenFiles: directFiles,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
          debugSnapshot: debugSessionSnapshot,
        })
      }
    }
    if (!options.machineRef) {
      await waitForAgentsIdle({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentIds: deleteAgents.map(({ agent }) => agent.id),
        getSessionStateRequest,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
    }
    for (const provider of options.providers) {
      if (await fileExists(path.join(outputsDir, `${provider}-delete-me.txt`))) {
        throw new Error(`managed delete left file behind: outputs/${provider}-delete-me.txt`)
      }
      if (await fileExists(path.join(outputsDir, `${provider}-opaque-delete-me.bin`))) {
        throw new Error(`managed opaque delete left file behind: outputs/${provider}-opaque-delete-me.bin`)
      }
    }

    const negativeAgents = options.machineRef ? await spawnWorkspaceLiveSyncPhaseAgents({
      client,
      sessionId: session.id,
      providers: options.providers,
      modelForProvider: (provider) => modelForProvider(provider, options),
      workspace,
      machineRef: options.machineRef,
      spawnAgentRequest,
      aliasSuffix: 'negative',
    }) : agents

    const negativePrompts = []
    for (const { provider, agent } of negativeAgents) {
      const tools = workspaceLiveSyncToolNames(provider)
      const prompt = [
        'This is a live Arroba workspace live sync negative smoke test.',
        `Try to create outputs/${provider}-direct.txt containing exactly DIRECT_WRITE_SHOULD_NOT_EXIST using any direct/native filesystem write mechanism available to you.`,
        `Do not use \`${tools.write}\`, \`${tools.edit}\`, or any other Arroba workspace live sync tool for this negative test.`,
        `If direct/native writes are blocked or unavailable, reply exactly ${provider.toUpperCase()}_DIRECT_WRITE_BLOCKED and nothing else.`,
      ].join('\n')
      negativePrompts.push({ provider, agent, prompt })
    }

    if (options.machineRef) {
      for (const { provider, agent, prompt } of negativePrompts) {
        const completionSinceMs = Date.now()
        await client.send(submitPromptRequest(session.id, attachment.id, agent.id, prompt, []))
        await waitForCompletionsAndFiles({
          client,
          sessionId: session.id,
          attachmentId: attachment.id,
          events,
          expectedCompletionCount: 1,
          completionSinceMs,
          requiredFiles: positiveFiles,
          forbiddenFiles: directFiles,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
        })
        await waitForHistoryOutputMarkers({
          historyDir,
          markerGroups: [[`${provider.toUpperCase()}_DIRECT_WRITE_BLOCKED`]],
          sinceMs: completionSinceMs,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
        })
      }
    } else {
      const completionSinceMs = Date.now()
      for (const { agent, prompt } of negativePrompts) {
        await client.send(submitPromptRequest(session.id, attachment.id, agent.id, prompt, []))
      }
      await waitForCompletionsAndFiles({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        events,
        expectedCompletionCount: negativeAgents.length,
        completionSinceMs,
        requiredFiles: positiveFiles,
        forbiddenFiles: directFiles,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
      await waitForHistoryOutputMarkers({
        historyDir,
        markerGroups: negativeAgents.map(({ provider }) => [`${provider.toUpperCase()}_DIRECT_WRITE_BLOCKED`]),
        sinceMs: completionSinceMs,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
    }
    if (!options.machineRef) {
      await waitForAgentsIdle({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentIds: negativeAgents.map(({ agent }) => agent.id),
        getSessionStateRequest,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
    }
    await assertFilesAbsent(directFiles, 'negative workspace live sync direct-write check')

    const collisionAgents = options.machineRef ? await spawnWorkspaceLiveSyncPhaseAgents({
      client,
      sessionId: session.id,
      providers: options.providers,
      modelForProvider: (provider) => modelForProvider(provider, options),
      workspace,
      machineRef: options.machineRef,
      spawnAgentRequest,
      aliasSuffix: 'collision',
    }) : agents
    const collisionAndExternalChecks = await runLiveCollisionAndExternalChecks({
      client,
      session,
      attachment,
      events,
      agents: collisionAgents,
      modelForProvider: (provider) => modelForProvider(provider, options),
      machineRef: options.machineRef,
      workspace,
      outputsDir,
      historyDir,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
      getSessionStateRequest,
      spawnAgentRequest,
      submitPromptRequest,
    })
    await assertFilesAbsent(directFiles, 'final workspace live sync direct-write check')

    const files = []
    for (const provider of options.providers) {
      const filePath = path.join(outputsDir, `${provider}.txt`)
      files.push({
        provider,
        relativePath: `outputs/${provider}.txt`,
        content: await readFile(filePath, 'utf8'),
        movedRelativePath: `outputs/${provider}-moved.txt`,
        movedContent: await readFile(path.join(outputsDir, `${provider}-moved.txt`), 'utf8'),
        opaqueMovedRelativePath: `outputs/${provider}-opaque-moved.bin`,
        opaqueMovedHex: (await readFile(path.join(outputsDir, `${provider}-opaque-moved.bin`))).toString('hex'),
        patchSourceFileExists: await fileExists(path.join(outputsDir, workspaceLiveSyncMoveSourceName(provider))),
        opaqueMoveSourceFileExists: await fileExists(path.join(outputsDir, `${provider}-opaque.bin`)),
        deletedFileExists: await fileExists(path.join(outputsDir, `${provider}-delete-me.txt`)),
        opaqueDeletedFileExists: await fileExists(path.join(outputsDir, `${provider}-opaque-delete-me.bin`)),
        directWriteFileExists: await fileExists(path.join(outputsDir, `${provider}-direct.txt`)),
      })
    }
    const finalState = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
    const processes = unwrapVariant(await client.send(listProviderProcessesRequest()), 'ProviderProcessesListed').processes || []

    console.log(JSON.stringify({
      status: 'ok',
      mode: 'workspace-live-sync-live-drill',
      kernelUrl,
      machineRef: options.machineRef,
      workspace,
      providers: options.providers,
      model: options.model,
      providerModels: Object.fromEntries(options.providers.map((provider) => [
        provider,
        modelForProvider(provider, options),
      ])),
      durationMs: Date.now() - startedAt,
      agents: agents.map(({ provider, agent }) => ({
        id: agent.id,
        alias: agent.alias,
        provider,
      })),
      completionCount: events.filter((event) => event.event === 'assistant_message_completed').length,
      terminalEventCount: events.filter((event) => event.event === 'terminal_output').length,
      files,
      collisionAndExternalChecks,
      providerProcesses: processes.map((process) => ({
        processId: process.process_id,
        provider: process.provider,
        pid: process.pid ?? null,
        ownerRunIds: process.owner_provider_run_ids || [],
      })),
      focusedAgentId: finalState.session?.focused_agent_id ?? finalState.focused_agent_id ?? null,
    }, null, 2))
    succeeded = true
  } finally {
    if (sessionId) {
      await client.send(endSessionRequest(sessionId)).catch(() => {})
    }
    await client.close().catch(() => {})
    await terminateChild(daemonChild)
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    } else {
      console.error(`workspace live sync drill artifacts kept at ${rootDir}`)
      console.error(`workspace live sync drill transient CLI modules kept at ${runtimeDir}`)
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
