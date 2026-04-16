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
const DEFAULT_MODEL = 'gpt-5.4'
const DEFAULT_PROVIDERS = ['opencode', 'codex']
const DEFAULT_TIMEOUT_MS = 360_000
const DEFAULT_POLL_MS = 1_000

function parseArgs(argv) {
  const options = {
    kernel: DEFAULT_KERNEL,
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
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
    'Usage: node apps/cli/scripts/live-managed-io-drill.mjs [options]',
    '',
    'Runs a live managed-I/O provider drill with isolated daemon/session/workspace lifecycle:',
    '- positive: agents read seed.txt and exercise Arroba write/edit/apply_patch/move/delete tools',
    '- negative: agents are asked to write directly without Arroba; direct output files must not appear',
    '- collision: two agents attempt the same text edit area; exactly one write may land',
    '- external changes: non-overlap stale edits rebase, overlapping stale edits are rejected',
    '',
    'Options:',
    `  --kernel ${DEFAULT_KERNEL}`,
    '  --provider PROVIDER',
    `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
    `  --model ${DEFAULT_MODEL}`,
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    `  --poll-ms ${DEFAULT_POLL_MS}`,
    '  --no-spawn-daemon',
    '  --machine-ref MACHINE_ID_OR_ALIAS (spawn agents on a remote worker machine)',
    '  --history-dir PATH (session history dir when using --no-spawn-daemon)',
    '  --keep-artifacts-on-failure',
    '  --positive-only (stop after the managed read/write/edit/apply_patch/move/delete smoke)',
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

function managedIoSpawnAgentRequest(spawnAgentRequest, sessionId, provider, alias, model, worktreeId, effort, machineRef) {
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

async function spawnManagedIoPhaseAgents({
  client,
  sessionId,
  providers,
  model,
  workspace,
  machineRef,
  spawnAgentRequest,
  aliasSuffix,
}) {
  const agents = []
  for (let index = 0; index < providers.length; index += 1) {
    const provider = providers[index]
    const agent = unwrapVariant(
      await client.send(managedIoSpawnAgentRequest(
        spawnAgentRequest,
        sessionId,
        provider,
        `${provider}-managed-io-${aliasSuffix}-${index + 1}`,
        model,
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

async function waitForCompletionsAndFiles({ client, sessionId, attachmentId, events, expectedCompletionCount, requiredFiles, forbiddenFiles, timeoutMs, pollMs, debugSnapshot }) {
  const started = Date.now()
  let lastRequiredCount = 0
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
    for (const requiredFile of requiredFiles) {
      if (await fileExists(requiredFile)) requiredExisting.push(requiredFile)
    }
    lastRequiredCount = requiredExisting.length
    const completed = events.filter((event) => event.event === 'assistant_message_completed')
    if (requiredExisting.length === requiredFiles.length && completed.length >= expectedCompletionCount) {
      return completed
    }
    await sleep(pollMs)
  }
  const debug = debugSnapshot ? `; debug=${JSON.stringify(await debugSnapshot())}` : ''
  throw new Error(`timed out waiting for ${expectedCompletionCount} completions and ${requiredFiles.length} required files; required files present=${lastRequiredCount}${debug}`)
}

async function waitForCompletionCount({ client, sessionId, attachmentId, events, expectedCompletionCount, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    const completed = events.filter((event) => event.event === 'assistant_message_completed')
    if (completed.length >= expectedCompletionCount) return completed
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${expectedCompletionCount} completions`)
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
  model,
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
    const overlapPath = path.join(outputsDir, `${provider}-overlap.txt`)
    await writeFile(overlapPath, 'one\nTARGET\nthree\n', 'utf8')
    const collider = unwrapVariant(
      await client.send(managedIoSpawnAgentRequest(
        spawnAgentRequest,
        session.id,
        provider,
        `${provider}-managed-io-collider`,
        model,
        workspace,
        'low',
        machineRef,
      )),
      'AgentSpawned',
      'AgentSpawned',
    ).agent
    const firstNewText = `FROM_${provider.toUpperCase()}_A`
    const secondNewText = `FROM_${provider.toUpperCase()}_B`
    const overlapSameAreaEditStartedAt = Date.now()
    for (const [editAgent, label, newText] of [[agent, 'A', firstNewText], [collider, 'B', secondNewText]]) {
      const prompt = [
        'This is a live Arroba managed I/O overlapping-writer drill.',
        'Use only Arroba managed I/O. Do not use shell commands or native filesystem writes.',
        'Do not reread or retry if the managed edit is rejected.',
        `Call \`arroba.edit_artifact\` exactly once with JSON arguments {"path":"outputs/${provider}-overlap.txt","old_text":"TARGET","new_text":${JSON.stringify(newText)},"domain":"text"}.`,
        `Then reply exactly ${provider.toUpperCase()}_OVERLAP_${label}_DONE if applied, or ${provider.toUpperCase()}_OVERLAP_${label}_BLOCKED if rejected.`,
      ].join('\n')
      await client.send(submitPromptRequest(session.id, attachment.id, editAgent.id, prompt, []))
    }
    await waitForManagedEditResults({
      historyDir,
      artifactPath: `outputs/${provider}-overlap.txt`,
      sinceMs: overlapSameAreaEditStartedAt,
      count: 2,
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

    const nonOverlapPath = path.join(outputsDir, `${provider}-external-nonoverlap.txt`)
    const nonOverlapBase = 'header\nalpha\nTARGET\nomega\nfooter\n'
    const nonOverlapExternallyChanged = 'intro\nheader\nalpha\nTARGET\nomega\nfooter\noutro\n'
    const nonOverlapExpected = 'intro\nheader\nalpha\nREPLACED\nomega\nfooter\noutro\n'
    await writeFile(nonOverlapPath, nonOverlapBase, 'utf8')
    await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
      'This is a live Arroba managed I/O external non-overlap drill.',
      'Use only Arroba managed I/O.',
      `Call \`arroba.read_artifact\` exactly once with JSON arguments {"path":"outputs/${provider}-external-nonoverlap.txt","domain":"text"}.`,
      `Remember the returned snapshot_id for the next turn. Then reply exactly ${provider.toUpperCase()}_EXTERNAL_NONOVERLAP_READ_DONE.`,
    ].join('\n'), []))
    const nonOverlapRead = await waitForManagedReadSnapshot({
      historyDir,
      artifactPath: `outputs/${provider}-external-nonoverlap.txt`,
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
        agentIds: [agent.id],
        getSessionStateRequest,
        timeoutMs,
        pollMs,
      })
    }
    await writeFile(nonOverlapPath, nonOverlapExternallyChanged, 'utf8')
    const nonOverlapEditStartedAt = Date.now()
    await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
      'Continue the external non-overlap drill.',
      'Use only Arroba managed I/O. Do not reread the artifact.',
      `Use this exact snapshot_id: ${nonOverlapRead.snapshot_id}`,
      `Call \`arroba.edit_artifact\` exactly once with JSON arguments {"path":"outputs/${provider}-external-nonoverlap.txt","old_text":"TARGET","new_text":"REPLACED","domain":"text","snapshot_id":${JSON.stringify(nonOverlapRead.snapshot_id)}}.`,
      `Then reply exactly ${provider.toUpperCase()}_EXTERNAL_NONOVERLAP_EDIT_DONE.`,
    ].join('\n'), []))
    const nonOverlapEdit = await waitForManagedEditResult({
      historyDir,
      artifactPath: `outputs/${provider}-external-nonoverlap.txt`,
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
    await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
      'This is a live Arroba managed I/O external overlap drill.',
      'Use only Arroba managed I/O.',
      `Call \`arroba.read_artifact\` exactly once with JSON arguments {"path":"outputs/${provider}-external-overlap.txt","domain":"text"}.`,
      `Remember the returned snapshot_id for the next turn. Then reply exactly ${provider.toUpperCase()}_EXTERNAL_OVERLAP_READ_DONE.`,
    ].join('\n'), []))
    const overlapRead = await waitForManagedReadSnapshot({
      historyDir,
      artifactPath: `outputs/${provider}-external-overlap.txt`,
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
        agentIds: [agent.id],
        getSessionStateRequest,
        timeoutMs,
        pollMs,
      })
    }
    await writeFile(overlapExternalPath, externalOverlapExpected, 'utf8')
    const overlapEditStartedAt = Date.now()
    await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
      'Continue the external overlap drill.',
      'Use only Arroba managed I/O. Do not reread the artifact and do not retry if rejected.',
      `Use this exact snapshot_id: ${overlapRead.snapshot_id}`,
      `Call \`arroba.edit_artifact\` exactly once with JSON arguments {"path":"outputs/${provider}-external-overlap.txt","old_text":"TARGET","new_text":"AGENT","domain":"text","snapshot_id":${JSON.stringify(overlapRead.snapshot_id)}}.`,
      `Then reply exactly ${provider.toUpperCase()}_EXTERNAL_OVERLAP_BLOCKED if rejected, or ${provider.toUpperCase()}_EXTERNAL_OVERLAP_UNEXPECTED_APPLIED if applied.`,
    ].join('\n'), []))
    const overlapEdit = await waitForManagedEditResult({
      historyDir,
      artifactPath: `outputs/${provider}-external-overlap.txt`,
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

  const runtimeDir = path.join(cliRoot, '.tmp-live-managed-io-drill')
  // Keep the live workspace out of OS temp directories: Codex read-only mode may
  // allow TMPDIR writes, which would make the negative direct-write probe invalid.
  const rootDir = path.join(cliRoot, 'target', 'live-managed-io-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const outputsDir = path.join(workspace, 'outputs')
  await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(runtimeDir, { recursive: true })
  await mkdir(outputsDir, { recursive: true })
  await writeFile(path.join(workspace, 'seed.txt'), 'seed-value-42\n', 'utf8')
  for (const provider of options.providers) {
    await writeFile(path.join(outputsDir, `${provider}-delete-me.txt`), 'delete-me\n', 'utf8')
  }

  const { LocalIpcClient, requests } = await loadCliModules(runtimeDir)
  const {
    attachToSessionRequest,
    createSessionRequest,
    endSessionRequest,
    getSessionStateRequest,
    listProviderProcessesRequest,
    spawnAgentRequest,
    submitPromptRequest,
  } = requests

  let daemonChild = null
  let kernelUrl = options.kernel
  const startedAt = Date.now()
  let succeeded = false
  if (options.spawnDaemon) {
    const ports = makePorts()
    kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
    const daemonBinary = await resolveBinary(
      path.join(repoRoot, 'apps/daemon/target/debug/arroba-daemon'),
      path.join(repoRoot, 'apps/daemon/Cargo.toml'),
      'arroba-daemon',
    )
    daemonChild = spawn(daemonBinary, [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        ARROBA_KERNEL_PORT: String(ports.kernelPort),
        ARROBA_MCP_PORT: String(ports.mcpPort),
        ARROBA_OPENCODE_PORT: String(ports.opencodePort),
        ARROBA_CODEX_PORT: String(ports.codexPort),
        ARROBA_DAEMON_ID: `managed-io-drill-${process.pid}-${Date.now()}`,
        ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
        ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
      },
      stdio: ['ignore', 'ignore', 'inherit'],
    })
    await waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace, workspace)
  }

  const client = new LocalIpcClient(kernelUrl)
  const events = []
  let sessionId = null
  try {
    const session = unwrap(await client.send(createSessionRequest(workspace, workspace)), 'SessionCreated').session
    sessionId = session.id
    const attachment = unwrap(
      await client.send(attachToSessionRequest(session.id, `managed-io-drill-${Date.now()}`)),
      'SessionAttached',
    ).attachment
    client.onKernelEvent((event) => events.push({ ...event, observed_at_ms: Date.now() }))
    await client.subscribeToKernelEvents(session.id, attachment.id)

    const agents = await spawnManagedIoPhaseAgents({
      client,
      sessionId: session.id,
      providers: options.providers,
      model: options.model,
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

    const beforePositiveCompletionCount = events.filter((event) => event.event === 'assistant_message_completed').length
    for (const { provider, agent } of agents) {
      const written = `${provider}-managed-io-write-ok: seed-value-42\n`
      const edited = `${provider}-managed-io-edit-ok: seed-value-42\n`
      const patchInitial = `patch-start-${provider}\n`
      const patchMoved = `patch-moved-${provider}\n`
      const patchText = [
        '*** Begin Patch',
        `*** Add File: outputs/${provider}-patch.txt`,
        `+${patchInitial.trimEnd()}`,
        '*** End Patch',
      ].join('\n')
      const prompt = [
        'This is a live Arroba managed I/O positive smoke test.',
        'Do not use shell commands, direct filesystem writes, native patch/edit tools, or any non-Arroba file write path.',
        'Use only the Arroba MCP/runtime tools for file I/O.',
        'Step 1: call `arroba.read_artifact` with JSON arguments {"path":"seed.txt","domain":"text"}.',
        `Step 2: call \`arroba.write_artifact\` with JSON arguments {"path":"outputs/${provider}.txt","content_text":${JSON.stringify(written)},"domain":"text"}.`,
        `Step 3: call \`arroba.edit_artifact\` with JSON arguments {"path":"outputs/${provider}.txt","old_text":${JSON.stringify(written)},"new_text":${JSON.stringify(edited)},"domain":"text"}.`,
        `Step 4: call \`arroba.apply_patch\` with JSON arguments {"patch_text":${JSON.stringify(patchText)},"domain":"text"}.`,
        `Step 5: call \`arroba.move_artifact\` with JSON arguments {"from_path":"outputs/${provider}-patch.txt","to_path":"outputs/${provider}-moved.txt","old_text":${JSON.stringify(patchInitial)},"new_text":${JSON.stringify(patchMoved)},"domain":"text"}.`,
        `After the tool succeeds, reply exactly ${provider.toUpperCase()}_MANAGED_IO_DONE and nothing else.`,
      ].join('\n')
      await client.send(submitPromptRequest(session.id, attachment.id, agent.id, prompt, []))
    }

    const positiveFiles = options.providers.map((provider) => path.join(outputsDir, `${provider}.txt`))
    const movedFiles = options.providers.map((provider) => path.join(outputsDir, `${provider}-moved.txt`))
    const directFiles = options.providers.map((provider) => path.join(outputsDir, `${provider}-direct.txt`))
    await waitForCompletionsAndFiles({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      events,
      expectedCompletionCount: beforePositiveCompletionCount + agents.length,
      requiredFiles: [...positiveFiles, ...movedFiles],
      forbiddenFiles: directFiles,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
      debugSnapshot: debugSessionSnapshot,
    })
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
        `${provider}-managed-io-edit-ok: seed-value-42\n`,
      )
      await assertFileContent(path.join(outputsDir, `${provider}-moved.txt`), `patch-moved-${provider}\n`)
      if (await fileExists(path.join(outputsDir, `${provider}-patch.txt`))) {
        throw new Error(`managed move left source file behind: outputs/${provider}-patch.txt`)
      }
    }

    const deleteAgents = []
    if (options.machineRef) {
      for (const { provider } of agents) {
        const deleteAgent = unwrapVariant(
          await client.send(managedIoSpawnAgentRequest(
            spawnAgentRequest,
            session.id,
            provider,
            `${provider}-managed-io-delete`,
            options.model,
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

    const beforeDeleteCompletionCount = events.filter((event) => event.event === 'assistant_message_completed').length
    for (const { provider, agent } of deleteAgents) {
      const prompt = [
        'This is a live Arroba managed I/O delete smoke test.',
        'Do not use shell commands, direct filesystem writes, native patch/edit tools, or any non-Arroba file write path.',
        `Call \`arroba.delete_artifact\` with JSON arguments {"path":"outputs/${provider}-delete-me.txt","domain":"text"} to delete the pre-existing delete-me file.`,
        `After the tool succeeds, reply exactly ${provider.toUpperCase()}_MANAGED_IO_DELETE_DONE and nothing else.`,
      ].join('\n')
      await client.send(submitPromptRequest(session.id, attachment.id, agent.id, prompt, []))
    }
    await waitForCompletionCount({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      events,
      expectedCompletionCount: beforeDeleteCompletionCount + deleteAgents.length,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
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
          patchSourceFileExists: await fileExists(path.join(outputsDir, `${provider}-patch.txt`)),
          deletedFileExists: await fileExists(path.join(outputsDir, `${provider}-delete-me.txt`)),
          directWriteFileExists: await fileExists(path.join(outputsDir, `${provider}-direct.txt`)),
        })
      }
      const finalState = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
      const processes = unwrapVariant(await client.send(listProviderProcessesRequest()), 'ProviderProcessesListed').processes || []
      console.log(JSON.stringify({
        status: 'ok',
        mode: 'managed-io-live-drill',
        kernelUrl,
        machineRef: options.machineRef,
        workspace,
        providers: options.providers,
        model: options.model,
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

    const negativeAgents = options.machineRef ? await spawnManagedIoPhaseAgents({
      client,
      sessionId: session.id,
      providers: options.providers,
      model: options.model,
      workspace,
      machineRef: options.machineRef,
      spawnAgentRequest,
      aliasSuffix: 'negative',
    }) : agents

    const beforeNegativeCompletionCount = events.filter((event) => event.event === 'assistant_message_completed').length
    for (const { provider, agent } of negativeAgents) {
      const prompt = [
        'This is a live Arroba managed I/O negative smoke test.',
        `Try to create outputs/${provider}-direct.txt containing exactly DIRECT_WRITE_SHOULD_NOT_EXIST using any direct/native filesystem write mechanism available to you.`,
        'Do not use `arroba.write_artifact`, `arroba.edit_artifact`, or any other Arroba managed I/O tool for this negative test.',
        `If direct/native writes are blocked or unavailable, reply exactly ${provider.toUpperCase()}_DIRECT_WRITE_BLOCKED and nothing else.`,
      ].join('\n')
      await client.send(submitPromptRequest(session.id, attachment.id, agent.id, prompt, []))
    }

    await waitForCompletionsAndFiles({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      events,
      expectedCompletionCount: beforeNegativeCompletionCount + negativeAgents.length,
      requiredFiles: positiveFiles,
      forbiddenFiles: directFiles,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
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

    const collisionAgents = options.machineRef ? await spawnManagedIoPhaseAgents({
      client,
      sessionId: session.id,
      providers: options.providers,
      model: options.model,
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
      model: options.model,
      machineRef: options.machineRef,
      workspace,
      outputsDir,
      historyDir: options.historyDir ?? path.join(rootDir, 'history'),
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
      getSessionStateRequest,
      spawnAgentRequest,
      submitPromptRequest,
    })

    const files = []
    for (const provider of options.providers) {
      const filePath = path.join(outputsDir, `${provider}.txt`)
      files.push({
        provider,
        relativePath: `outputs/${provider}.txt`,
        content: await readFile(filePath, 'utf8'),
        movedRelativePath: `outputs/${provider}-moved.txt`,
        movedContent: await readFile(path.join(outputsDir, `${provider}-moved.txt`), 'utf8'),
        patchSourceFileExists: await fileExists(path.join(outputsDir, `${provider}-patch.txt`)),
        deletedFileExists: await fileExists(path.join(outputsDir, `${provider}-delete-me.txt`)),
        directWriteFileExists: await fileExists(path.join(outputsDir, `${provider}-direct.txt`)),
      })
    }
    const finalState = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
    const processes = unwrapVariant(await client.send(listProviderProcessesRequest()), 'ProviderProcessesListed').processes || []

    console.log(JSON.stringify({
      status: 'ok',
      mode: 'managed-io-live-drill',
      kernelUrl,
      machineRef: options.machineRef,
      workspace,
      providers: options.providers,
      model: options.model,
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
      console.error(`managed I/O drill artifacts kept at ${rootDir}`)
      console.error(`managed I/O drill transient CLI modules kept at ${runtimeDir}`)
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
