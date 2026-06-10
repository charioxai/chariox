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
    providerHistoryDirs: [],
    keepArtifactsOnFailure: false,
    positiveOnly: false,
    mode: 'managed',
    managedTargetCount: 0,
    targetBranch: 'main',
    trackedTargetCount: 1,
    trackedBidirectional: false,
    remoteSourceSideEffects: false,
    rootDir: null,
    afterFixtureCommand: null,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--') continue
    else if (arg === '--kernel') options.kernel = argv[++i]
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
    else if (arg === '--provider-history-dir') options.providerHistoryDirs.push(argv[++i])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--positive-only') options.positiveOnly = true
    else if (arg === '--mode') {
      options.mode = argv[++i]
      if (!['off', 'managed', 'tracked'].includes(options.mode)) throw new Error('--mode must be off, managed, or tracked')
    }
    else if (arg === '--managed-target-count') options.managedTargetCount = Number(argv[++i])
    else if (arg === '--target-branch') options.targetBranch = argv[++i]
    else if (arg === '--tracked-target-count') options.trackedTargetCount = Number(argv[++i])
    else if (arg === '--tracked-bidirectional') options.trackedBidirectional = true
    else if (arg === '--remote-source-side-effects') options.remoteSourceSideEffects = true
    else if (arg === '--root-dir') options.rootDir = argv[++i]
    else if (arg === '--after-fixture-command') options.afterFixtureCommand = argv[++i]
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  if (!Number.isInteger(options.trackedTargetCount) || options.trackedTargetCount < 1) {
    throw new Error('--tracked-target-count must be a positive integer')
  }
  if (!Number.isInteger(options.managedTargetCount) || options.managedTargetCount < 0) {
    throw new Error('--managed-target-count must be a non-negative integer')
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-workspace-live-sync-drill.mjs [options]',
    '',
    'Runs a live workspace live sync provider drill with isolated daemon/session/workspace lifecycle:',
    '- off: agents write directly in the selected repo and a sibling repo; both writes must land',
    '- positive: agents read seed.txt and exercise Arroba write/edit/patch/move/delete tools',
    '- negative: agents are asked to write directly without Arroba; direct output files must not appear',
    '- collision: two agents attempt the same text edit area; exactly one write may land',
    '- external changes: non-overlap stale edits rebase, overlapping stale edits are rejected',
    '- tracked mode: direct writes inside the selected worktree sync at turn end, while direct writes to the sibling repo must remain allowed and unsynced',
    '',
    'Options:',
    `  --kernel ${DEFAULT_KERNEL}`,
    '  --provider PROVIDER',
    `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
    `  --model ${DEFAULT_MODEL}`,
    '  --provider-model PROVIDER=MODEL (for example codex=gpt-5.2 or opencode=opencode/gpt-5.2)',
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    `  --poll-ms ${DEFAULT_POLL_MS}`,
    '  --no-spawn-daemon',
    '  --machine-ref MACHINE_ID_OR_ALIAS (spawn agents on a remote worker machine)',
    '  --history-dir PATH (session history dir when using --no-spawn-daemon)',
    '  --provider-history-dir PATH (additional provider-event history dir; repeatable for remote workers)',
    '  --keep-artifacts-on-failure',
    '  --positive-only (stop after the managed read/write/edit/patch/move/delete smoke)',
    '  --mode off|managed|tracked',
    '  --managed-target-count COUNT (managed mode only; attach and validate target workspaces)',
    '  --target-branch BRANCH (tracked mode target branch; use a non-main value to drill explicit cross-branch links)',
    '  --tracked-target-count COUNT (tracked mode only; attach and validate multiple target workspaces)',
    '  --tracked-bidirectional (tracked mode only; validate target-origin fanout back to source/sibling targets)',
    '  --root-dir PATH (override isolated drill root)',
    '  --after-fixture-command CMD (run after local fixtures are initialized, before agents spawn)',
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

async function runCommand(command, args, cwd) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd, stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('exit', (code) => {
      if (code === 0) resolve({ stdout, stderr })
      else reject(new Error(`${command} ${args.join(' ')} failed with code ${code}: ${stderr || stdout}`))
    })
    child.on('error', reject)
  })
}

async function initGitWorktree(workspace, branch = 'main') {
  await runCommand('git', ['init'], workspace)
  await runCommand('git', ['config', 'user.email', 'workspace-live-sync-drill@example.com'], workspace)
  await runCommand('git', ['config', 'user.name', 'Workspace Live Sync Drill'], workspace)
  await runCommand('git', ['add', '.'], workspace)
  await runCommand('git', ['commit', '-m', 'seed workspace live sync fixture'], workspace)
  if (branch && branch !== 'main') {
    await runCommand('git', ['checkout', '-b', branch], workspace)
  }
}

async function initTrackedWorkspace(workspace, provider, branch = 'main') {
  const outputsDir = path.join(workspace, 'outputs')
  await mkdir(path.join(workspace, 'ignored'), { recursive: true })
  await mkdir(outputsDir, { recursive: true })
  await writeFile(path.join(workspace, '.gitignore'), 'ignored/\n*.secret\n', 'utf8')
  await writeFile(path.join(workspace, 'tracked.txt'), 'line-a\nline-b\n', 'utf8')
  await writeFile(path.join(workspace, 'target-origin.txt'), 'target-origin-a\ntarget-origin-b\n', 'utf8')
  await writeFile(path.join(outputsDir, `${provider}-tracked-delete.txt`), 'delete me\n', 'utf8')
  await writeFile(path.join(outputsDir, `${provider}-tracked-rename-source.txt`), 'rename me\n', 'utf8')
  await writeFile(path.join(outputsDir, `${provider}-tracked-rebase.txt`), 'alpha\nbeta\nomega\n', 'utf8')
  await writeFile(path.join(outputsDir, `${provider}-tracked-conflict.txt`), 'one\ntwo\nthree\n', 'utf8')
  await runCommand('git', ['init'], workspace)
  await runCommand('git', ['config', 'user.email', 'tracked-drill@example.com'], workspace)
  await runCommand('git', ['config', 'user.name', 'Tracked Drill'], workspace)
  await runCommand('git', ['add', '.'], workspace)
  await runCommand('git', ['commit', '-m', 'seed tracked workspace'], workspace)
  if (branch && branch !== 'main') {
    await runCommand('git', ['checkout', '-b', branch], workspace)
  }
}

async function gitHead(workspace) {
  const { stdout } = await runCommand('git', ['rev-parse', 'HEAD'], workspace)
  return stdout.trim()
}

async function resetTrackedWorkspace(workspace) {
  await runCommand('git', ['reset', '--hard', 'HEAD'], workspace)
  await runCommand('git', ['clean', '-fdx'], workspace)
}

async function runAfterFixtureCommand(command, context) {
  const env = {
    ...process.env,
    ARROBA_WORKSPACE_LIVE_SYNC_ROOT: context.rootDir,
    ARROBA_WORKSPACE_LIVE_SYNC_WORKSPACE: context.workspace,
    ARROBA_WORKSPACE_LIVE_SYNC_SIBLING_WORKSPACE: context.siblingWorkspace,
    ARROBA_WORKSPACE_LIVE_SYNC_TARGET_WORKSPACE: context.targetWorkspace,
    ARROBA_WORKSPACE_LIVE_SYNC_TARGET_WORKSPACES: JSON.stringify(context.targetWorkspaces),
    ARROBA_WORKSPACE_LIVE_SYNC_MODE: context.mode,
  }
  await new Promise((resolve, reject) => {
    const child = spawn('bash', ['-lc', command], { env, stdio: ['ignore', 'inherit', 'pipe'] })
    let stderr = ''
    child.stderr.on('data', (chunk) => {
      process.stderr.write(chunk)
      stderr += chunk.toString()
    })
    child.on('exit', (code) => {
      if (code === 0) resolve()
      else reject(new Error(`after-fixture command exited with code ${code}: ${stderr.trim()}`))
    })
    child.on('error', reject)
  })
}

async function initManagedTargetWorkspace(workspace, providers) {
  const outputsDir = path.join(workspace, 'outputs')
  await mkdir(outputsDir, { recursive: true })
  await writeFile(path.join(workspace, 'seed.txt'), 'seed-value-42\n', 'utf8')
  for (const provider of providers) {
    await writeFile(path.join(outputsDir, `${provider}-delete-me.txt`), 'delete-me\n', 'utf8')
    await writeFile(path.join(outputsDir, `${provider}-opaque-delete-me.bin`), Buffer.from([9, 8, 7]))
  }
  await initGitWorktree(workspace)
}

function workspaceLiveSyncSpawnAgentRequest(spawnAgentRequest, sessionId, provider, alias, model, worktreeId, effort, kernelRef) {
  return spawnAgentRequest(
    sessionId,
    provider,
    alias,
    model,
    worktreeId,
    effort,
    undefined,
    undefined,
    kernelRef ?? undefined,
  )
}

function modelForProvider(provider, options) {
  const explicit = options.providerModels[provider]
  if (explicit) return explicit
  if (provider === 'opencode' && !options.model.includes('/')) return `opencode/${options.model}`
  return options.model
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
  kernelRef,
  spawnAgentRequest,
  aliasSuffix,
}) {
  const agents = []
  for (let index = 0; index < providers.length; index += 1) {
    const provider = providers[index]
    const spawned = unwrapVariant(
      await client.send(workspaceLiveSyncSpawnAgentRequest(
        spawnAgentRequest,
        sessionId,
        provider,
        `${provider}-workspace-live-sync-${aliasSuffix}-${index + 1}`,
        modelForProvider(provider),
        workspace,
        'low',
        kernelRef,
      )),
      'AgentSpawned',
    )
    if (kernelRef && !spawned.agent.remote_execution?.leased_agent_id) {
      throw new Error(`agent ${spawned.agent.id} for provider ${provider} did not receive a remote lease`)
    }
    agents.push({ provider, agent: spawned.agent, spawnedSessionId: spawned.session?.id ?? null })
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

async function resolveRemoteWorkerKernelRef(client, requests, machineRef, providers, timeoutMs, pollMs) {
  if (!machineRef) return null
  const deadline = Date.now() + timeoutMs
  let lastKernels = []
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const response = unwrapVariant(
        await client.send(requests.listRemoteMachineKernelsRequest(machineRef)),
        'RemoteMachineKernelsListed',
      )
      lastKernels = response.kernels || []
      const kernel = lastKernels.find((candidate) => {
        const available = candidate.available_providers || []
        return candidate.accepting_remote_leases && providers.every((provider) => available.includes(provider))
      })
      if (kernel) return kernel.kernel_id || kernel.daemon_id || kernel.kernel_alias || machineRef
    } catch (error) {
      lastError = error
    }
    await sleep(pollMs)
  }
  throw new Error(`remote machine ${machineRef} did not advertise a worker kernel for providers ${providers.join(',')}; last=${JSON.stringify(lastKernels)} error=${lastError?.message ?? lastError}`)
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

async function providerErrorsSince({ historyDir, sinceMs }) {
  if (!historyDir) return []
  const errors = []
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
      if (entry.kind !== 'provider_error') continue
      errors.push({
        file,
        agentId: entry.agent_id ?? null,
        providerRunId: entry.provider_run_id ?? null,
        text: String(entry.text ?? '').trim(),
      })
    }
  }
  return errors
}

async function throwIfProviderError({ historyDir, sinceMs }) {
  const errors = await providerErrorsSince({ historyDir, sinceMs })
  if (errors.length === 0) return
  const summary = errors.map((error) => {
    const owner = [error.agentId, error.providerRunId].filter(Boolean).join('/')
    return `${owner ? `${owner}: ` : ''}${error.text}`
  }).join(' | ')
  throw new Error(`provider error while waiting for workspace live sync drill progress: ${summary}`)
}

async function waitForCompletionsAndFiles({ client, sessionId, attachmentId, events, expectedCompletionCount, completionSinceMs = 0, requiredFiles, forbiddenFiles, timeoutMs, pollMs, debugSnapshot, historyDir, providerErrorSinceMs = completionSinceMs }) {
  const started = Date.now()
  let lastRequiredCount = 0
  let lastMissingRequired = requiredFiles
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    await throwIfProviderError({ historyDir, sinceMs: providerErrorSinceMs })
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

async function managedTargetFanoutSnapshot(targetWorkspaces, providers) {
  return Promise.all(targetWorkspaces.map(async (targetWorkspace) => {
    const outputsDir = path.join(targetWorkspace, 'outputs')
    return {
      targetWorkspace,
      providers: await Promise.all(providers.map(async (provider) => ({
        provider,
        content: await readFile(path.join(outputsDir, `${provider}.txt`), 'utf8'),
        movedContent: await readFile(path.join(outputsDir, `${provider}-moved.txt`), 'utf8'),
        opaqueMovedHex: (await readFile(path.join(outputsDir, `${provider}-opaque-moved.bin`))).toString('hex'),
        patchSourceFileExists: await fileExists(path.join(outputsDir, workspaceLiveSyncMoveSourceName(provider))),
        opaqueMoveSourceFileExists: await fileExists(path.join(outputsDir, `${provider}-opaque.bin`)),
        deletedFileExists: await fileExists(path.join(outputsDir, `${provider}-delete-me.txt`)),
        opaqueDeletedFileExists: await fileExists(path.join(outputsDir, `${provider}-opaque-delete-me.bin`)),
        directWriteFileExists: await fileExists(path.join(outputsDir, `${provider}-direct.txt`)),
      }))),
    }
  }))
}

async function assertManagedTargetFanout(targetWorkspaces, providers, { deletesApplied }) {
  for (const targetWorkspace of targetWorkspaces) {
    const outputsDir = path.join(targetWorkspace, 'outputs')
    for (const provider of providers) {
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
        throw new Error(`managed target fanout left patch source behind for ${provider} in ${targetWorkspace}`)
      }
      if (await fileExists(path.join(outputsDir, `${provider}-opaque.bin`))) {
        throw new Error(`managed target fanout left opaque move source behind for ${provider} in ${targetWorkspace}`)
      }
      if (deletesApplied) {
        if (await fileExists(path.join(outputsDir, `${provider}-delete-me.txt`))) {
          throw new Error(`managed target fanout left deleted text file behind for ${provider} in ${targetWorkspace}`)
        }
        if (await fileExists(path.join(outputsDir, `${provider}-opaque-delete-me.bin`))) {
          throw new Error(`managed target fanout left deleted opaque file behind for ${provider} in ${targetWorkspace}`)
        }
      } else {
        await assertFileContent(path.join(outputsDir, `${provider}-delete-me.txt`), 'delete-me\n')
        await assertFileBytes(path.join(outputsDir, `${provider}-opaque-delete-me.bin`), [9, 8, 7])
      }
      if (await fileExists(path.join(outputsDir, `${provider}-direct.txt`))) {
        throw new Error(`managed target fanout created forbidden direct-write file for ${provider} in ${targetWorkspace}`)
      }
    }
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

async function waitForCompletionCount({ client, sessionId, attachmentId, events, expectedCompletionCount, completionSinceMs = 0, timeoutMs, pollMs, historyDir, providerErrorSinceMs = completionSinceMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    await throwIfProviderError({ historyDir, sinceMs: providerErrorSinceMs })
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
  historyDir,
  providerErrorSinceMs = completionSinceMs,
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
    historyDir,
    providerErrorSinceMs,
  })
}

async function historyProviderOutputMarkerGroups({ historyDir, providerHistoryDirs, markerGroups, sinceMs }) {
  const remaining = markerGroups.map((markers) => [...markers])
  const outputByKey = new Map()
  const files = (await managedHistoryFiles(managedHistoryDirs(historyDir, providerHistoryDirs))).sort()

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

async function pumpTerminalOutputIfAvailable({ client, sessionId, attachmentId }) {
  if (!client || !sessionId || !attachmentId) return
  await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
}

async function waitForHistoryOutputMarkers({ historyDir, providerHistoryDirs, markerGroups, sinceMs, timeoutMs, pollMs, client, sessionId, attachmentId }) {
  const started = Date.now()
  let missing = markerGroups
  while (Date.now() - started < timeoutMs) {
    await pumpTerminalOutputIfAvailable({ client, sessionId, attachmentId })
    await throwIfProviderError({ historyDir, sinceMs })
    missing = await historyProviderOutputMarkerGroups({ historyDir, providerHistoryDirs, markerGroups, sinceMs })
    if (missing.length === 0) return
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for provider output markers: ${missing.map((markers) => markers.join(' or ')).join(', ')}`)
}

async function historyNoticeMessagesSince({ historyDir, sinceMs }) {
  const messages = []
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
      if (entry.kind !== 'notice' || typeof entry.text !== 'string') continue
      messages.push(entry.text)
    }
  }
  return messages
}

async function waitForHistoryNotices({ historyDir, requirements, sinceMs, timeoutMs, pollMs, client, sessionId, attachmentId }) {
  const started = Date.now()
  let messages = []
  let missing = requirements
  while (Date.now() - started < timeoutMs) {
    await pumpTerminalOutputIfAvailable({ client, sessionId, attachmentId })
    await throwIfProviderError({ historyDir, sinceMs })
    messages = await historyNoticeMessagesSince({ historyDir, sinceMs })
    missing = requirements.filter((requirement) =>
      !messages.some((message) => requirement.includes.every((needle) => message.includes(needle)))
    )
    if (missing.length === 0) return messages
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for workspace live sync runtime notices: missing=${missing.map((requirement) => requirement.label).join(', ')}; notices=${JSON.stringify(messages)}`)
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
    await throwIfProviderError({ historyDir, sinceMs })
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

function managedHistoryDirs(historyDir, providerHistoryDirs = []) {
  return [...new Set([historyDir, ...providerHistoryDirs].filter(Boolean))]
}

async function managedHistoryFiles(historyDirs) {
  const nested = await Promise.all(historyDirs.map(async (dir) => {
    const files = await readdir(dir).catch(() => [])
    return files
      .filter((file) => file.endsWith('.jsonl'))
      .map((file) => path.join(dir, file))
  }))
  return nested.flat()
}

async function waitForManagedReadSnapshot({ historyDir, providerHistoryDirs, artifactPath, timeoutMs, pollMs, client, sessionId, attachmentId }) {
  const snapshots = await waitForManagedReadSnapshots({
    historyDir,
    providerHistoryDirs,
    artifactPath,
    count: 1,
    sinceMs: 0,
    timeoutMs,
    pollMs,
    client,
    sessionId,
    attachmentId,
  })
  return snapshots[0]
}

async function waitForManagedReadSnapshots({ historyDir, providerHistoryDirs, artifactPath, count, sinceMs, timeoutMs, pollMs, client, sessionId, attachmentId }) {
  const started = Date.now()
  const historyDirs = managedHistoryDirs(historyDir, providerHistoryDirs)
  while (Date.now() - started < timeoutMs) {
    await pumpTerminalOutputIfAvailable({ client, sessionId, attachmentId })
    const snapshots = []
    const files = await managedHistoryFiles(historyDirs)
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
        if (!tool.endsWith('read_artifact') || update.status !== 'completed') continue
        if (update.input?.path !== artifactPath) continue
        try {
          const output = parseManagedToolOutput(update.output)
          if (typeof output?.snapshot_id === 'string') {
            snapshots.push({
              ...output,
              agent_id: entry.agent_id ?? null,
              provider_run_id: entry.provider_run_id ?? null,
            })
          }
        } catch {
          continue
        }
      }
    }
    if (snapshots.length >= count) return snapshots.slice(0, count)
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${count} managed read snapshots for ${artifactPath}`)
}

async function waitForManagedEditResult({ historyDir, providerHistoryDirs, artifactPath, sinceMs, timeoutMs, pollMs, client, sessionId, attachmentId }) {
  const results = await waitForManagedEditResults({
    historyDir,
    providerHistoryDirs,
    artifactPath,
    sinceMs,
    count: 1,
    timeoutMs,
    pollMs,
    client,
    sessionId,
    attachmentId,
  })
  return results[0]
}

async function waitForManagedEditResults({ historyDir, providerHistoryDirs, artifactPath, sinceMs, count, timeoutMs, pollMs, client, sessionId, attachmentId }) {
  const started = Date.now()
  const historyDirs = managedHistoryDirs(historyDir, providerHistoryDirs)
  while (Date.now() - started < timeoutMs) {
    await pumpTerminalOutputIfAvailable({ client, sessionId, attachmentId })
    const results = []
    const files = await managedHistoryFiles(historyDirs)
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
  kernelRef,
  workspace,
  outputsDir,
  historyDir,
  providerHistoryDirs,
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
        kernelRef,
      )),
      'AgentSpawned',
      'AgentSpawned',
    ).agent
    const firstNewText = `FROM_${provider.toUpperCase()}_A`
    const secondNewText = `FROM_${provider.toUpperCase()}_B`
    const tools = workspaceLiveSyncToolNames(provider)
    const overlapSameAreaReadStartedAt = Date.now()
    for (const [editAgent, label] of [[agent, 'A'], [collider, 'B']]) {
      await client.send(submitPromptRequest(session.id, attachment.id, editAgent.id, [
        'This is a live Arroba workspace live sync overlapping-writer drill.',
        'Use only Arroba workspace live sync. Do not use shell commands or native filesystem writes.',
        `First call \`${tools.read}\` exactly once with JSON arguments {"path":"outputs/${provider}-overlap.txt","domain":"text"}.`,
        `Then reply exactly ${provider.toUpperCase()}_OVERLAP_${label}_READ_DONE.`,
      ].join('\n'), []))
    }
    const overlapReadSnapshots = await waitForManagedReadSnapshots({
      historyDir,
      providerHistoryDirs,
      artifactPath: `outputs/${provider}-overlap.txt`,
      sinceMs: overlapSameAreaReadStartedAt,
      count: machineRef ? 1 : 2,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
    })
    for (const snapshot of overlapReadSnapshots) {
      if (snapshot.content_text !== 'one\nTARGET\nthree\n') {
        throw new Error(`overlap drill read after a write for ${provider}: ${JSON.stringify(overlapReadSnapshots)}`)
      }
    }
    const snapshotByAgentId = new Map(overlapReadSnapshots.map((snapshot) => [snapshot.agent_id, snapshot.snapshot_id]))
    const overlapSameAreaEditStartedAt = Date.now()
    for (const [editAgent, label, newText] of [[agent, 'A', firstNewText], [collider, 'B', secondNewText]]) {
      const snapshotId = snapshotByAgentId.get(editAgent.id) ?? overlapReadSnapshots[0]?.snapshot_id
      const prompt = [
        'Continue the live Arroba workspace live sync overlapping-writer drill.',
        'Use only Arroba workspace live sync. Do not use shell commands or native filesystem writes.',
        'Do not read, reread, or retry.',
        'Do not include a range field.',
        `Call \`${tools.edit}\` exactly once with JSON arguments {"path":"outputs/${provider}-overlap.txt","old_text":"TARGET","new_text":${JSON.stringify(newText)},"domain":"text","snapshot_id":${JSON.stringify(snapshotId)}}.`,
        `Then reply exactly ${provider.toUpperCase()}_OVERLAP_${label}_DONE if applied, or ${provider.toUpperCase()}_OVERLAP_${label}_BLOCKED if rejected.`,
      ].join('\n')
      await client.send(submitPromptRequest(session.id, attachment.id, editAgent.id, prompt, []))
    }
    const overlapEditResults = await waitForManagedEditResults({
      historyDir,
      providerHistoryDirs,
      artifactPath: `outputs/${provider}-overlap.txt`,
      sinceMs: overlapSameAreaEditStartedAt,
      // Remote lease histories can mirror one side of a cross-provider collision
      // without the matching provider_tool record even though the file mutation
      // has landed. The final content assertion below still verifies that one
      // write won and the losing edit did not corrupt the file.
      count: machineRef ? 1 : 2,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
    })
    if (!machineRef) {
      const appliedCount = overlapEditResults.filter((result) => result?.applied === true).length
      const conflictCount = overlapEditResults.filter((result) => result?.applied === false && result?.reason?.kind === 'conflict').length
      if (appliedCount !== 1 || conflictCount !== 1) {
        throw new Error(`overlap drill expected one applied edit and one conflict for ${provider}: ${JSON.stringify(overlapEditResults)}`)
      }
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
      return unwrapVariant(
        await client.send(workspaceLiveSyncSpawnAgentRequest(
          spawnAgentRequest,
          session.id,
          provider,
          `${provider}-workspace-live-sync-${suffix}`,
          modelForProvider(provider),
          workspace,
          'low',
          kernelRef,
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
      providerHistoryDirs,
      artifactPath: `outputs/${provider}-external-nonoverlap.txt`,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
    })
    await waitForHistoryOutputMarkers({
      historyDir,
      providerHistoryDirs,
      markerGroups: [[`${provider.toUpperCase()}_EXTERNAL_NONOVERLAP_READ_DONE`]],
      sinceMs: nonOverlapReadStartedAt,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
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
      providerHistoryDirs,
      artifactPath: `outputs/${provider}-external-nonoverlap.txt`,
      sinceMs: nonOverlapEditStartedAt,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
    })
    await waitForHistoryOutputMarkers({
      historyDir,
      providerHistoryDirs,
      markerGroups: [[`${provider.toUpperCase()}_EXTERNAL_NONOVERLAP_EDIT_DONE`]],
      sinceMs: nonOverlapEditStartedAt,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
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
      providerHistoryDirs,
      artifactPath: `outputs/${provider}-external-overlap.txt`,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
    })
    await waitForHistoryOutputMarkers({
      historyDir,
      providerHistoryDirs,
      markerGroups: [[`${provider.toUpperCase()}_EXTERNAL_OVERLAP_READ_DONE`]],
      sinceMs: overlapReadStartedAt,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
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
      providerHistoryDirs,
      artifactPath: `outputs/${provider}-external-overlap.txt`,
      sinceMs: overlapEditStartedAt,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
    })
    await waitForHistoryOutputMarkers({
      historyDir,
      providerHistoryDirs,
      markerGroups: [[
        `${provider.toUpperCase()}_EXTERNAL_OVERLAP_BLOCKED`,
        `${provider.toUpperCase()}_EXTERNAL_OVERLAP_UNEXPECTED_APPLIED`,
      ]],
      sinceMs: overlapEditStartedAt,
      timeoutMs,
      pollMs,
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
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

async function waitForTrackedFanout({
  client,
  sessionId,
  attachmentId,
  getWorkspaceLiveSyncStatusRequest,
  sourceWorkspace,
  targetWorkspaces,
  provider,
  remoteSourceSideEffects,
  timeoutMs,
  pollMs,
}) {
  const sourceOutputs = path.join(sourceWorkspace, 'outputs')
  const expectedTracked = `line-a\n${provider}-tracked-modified\n`
  const expectedAdded = `${provider}-tracked-added\n`
  const expectedRenamed = `${provider}-tracked-renamed\n`
  const expectedSourceRebase = `alpha\nbeta\n${provider}-tracked-source\nomega\n`
  const expectedTargetRebase = `alpha\n${provider}-tracked-target-local\nbeta\n${provider}-tracked-source\nomega\n`
  const expectedSourceConflict = `one\n${provider}-tracked-source-conflict\nthree\n`
  const expectedTargetConflict = `one\n${provider}-tracked-target-conflict\nthree\n`
  let lastStatus = null
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    lastStatus = unwrapVariant(
      await client.send(getWorkspaceLiveSyncStatusRequest(sessionId)),
      'WorkspaceLiveSyncStatus',
    ).status
    const checks = [
      [path.join(sourceWorkspace, 'tracked.txt'), expectedTracked],
      [path.join(sourceOutputs, `${provider}-tracked-added.txt`), expectedAdded],
      [path.join(sourceOutputs, `${provider}-tracked-renamed.txt`), expectedRenamed],
      [path.join(sourceOutputs, `${provider}-tracked-rebase.txt`), expectedSourceRebase],
      [path.join(sourceOutputs, `${provider}-tracked-conflict.txt`), expectedSourceConflict],
      [path.join(sourceWorkspace, '.arrobaignore'), 'ignored/\n*.secret\n'],
    ]
    for (const targetWorkspace of targetWorkspaces) {
      const targetOutputs = path.join(targetWorkspace, 'outputs')
      checks.push(
        [path.join(targetWorkspace, 'tracked.txt'), expectedTracked],
        [path.join(targetOutputs, `${provider}-tracked-added.txt`), expectedAdded],
        [path.join(targetOutputs, `${provider}-tracked-renamed.txt`), expectedRenamed],
        [path.join(targetOutputs, `${provider}-tracked-rebase.txt`), expectedTargetRebase],
        [path.join(targetOutputs, `${provider}-tracked-conflict.txt`), expectedTargetConflict],
        [path.join(targetWorkspace, '.arrobaignore'), 'ignored/\n*.secret\n'],
      )
    }
    let contentOk = true
    for (const [filePath, expected] of checks) {
      if (!(await fileExists(filePath)) || (await readFile(filePath, 'utf8')) !== expected) {
        contentOk = false
        break
      }
    }
    if (contentOk) {
      const sourceBinaryPath = path.join(sourceOutputs, `${provider}-tracked-binary.bin`)
      if (!(await fileExists(sourceBinaryPath))) {
        contentOk = false
      } else {
        const sourceBinary = await readFile(sourceBinaryPath)
        contentOk = sourceBinary.length > 0
        for (const targetWorkspace of targetWorkspaces) {
          const targetBinaryPath = path.join(targetWorkspace, 'outputs', `${provider}-tracked-binary.bin`)
          if (!(await fileExists(targetBinaryPath)) || !(await readFile(targetBinaryPath)).equals(sourceBinary)) {
            contentOk = false
            break
          }
        }
      }
    }
    let deletedOk = !(await fileExists(path.join(sourceOutputs, `${provider}-tracked-delete.txt`))) &&
      !(await fileExists(path.join(sourceOutputs, `${provider}-tracked-rename-source.txt`)))
    let ignoredOk = remoteSourceSideEffects
      ? !(await fileExists(path.join(sourceWorkspace, 'ignored', `${provider}-ignored.txt`)))
      : await fileExists(path.join(sourceWorkspace, 'ignored', `${provider}-ignored.txt`))
    let hasTargets = true
    let hasExpectedConflicts = true
    for (const targetWorkspace of targetWorkspaces) {
      const targetOutputs = path.join(targetWorkspace, 'outputs')
      deletedOk = deletedOk &&
        !(await fileExists(path.join(targetOutputs, `${provider}-tracked-delete.txt`))) &&
        !(await fileExists(path.join(targetOutputs, `${provider}-tracked-rename-source.txt`)))
      ignoredOk = ignoredOk && !(await fileExists(path.join(targetWorkspace, 'ignored', `${provider}-ignored.txt`)))
      hasTargets = hasTargets && (lastStatus.targets ?? []).some((target) => target.repo_root === targetWorkspace)
      hasExpectedConflicts = hasExpectedConflicts && (lastStatus.conflicts ?? []).some((conflict) => (
        conflict.target_repo_root === targetWorkspace &&
        conflict.path === `outputs/${provider}-tracked-conflict.txt` &&
        conflict.source_agent_id
      ))
    }
    if (contentOk && deletedOk && ignoredOk && hasTargets && hasExpectedConflicts) return lastStatus
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for tracked workspace live sync fanout; lastStatus=${JSON.stringify(lastStatus)}`)
}

async function waitForTrackedTargetOriginFanout({
  client,
  sessionId,
  attachmentId,
  getWorkspaceLiveSyncStatusRequest,
  allWorkspaces,
  statusTargetWorkspaces,
  provider,
  timeoutMs,
  pollMs,
}) {
  const expectedText = `${provider}-target-origin-modified\n`
  const expectedAdded = `${provider}-target-origin-added\n`
  let lastStatus = null
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    lastStatus = unwrapVariant(
      await client.send(getWorkspaceLiveSyncStatusRequest(sessionId)),
      'WorkspaceLiveSyncStatus',
    ).status
    let contentOk = true
    for (const workspace of allWorkspaces) {
      const textPath = path.join(workspace, 'target-origin.txt')
      const addedPath = path.join(workspace, 'outputs', `${provider}-target-origin-added.txt`)
      if (!(await fileExists(textPath)) || (await readFile(textPath, 'utf8')) !== expectedText) {
        contentOk = false
        break
      }
      if (!(await fileExists(addedPath)) || (await readFile(addedPath, 'utf8')) !== expectedAdded) {
        contentOk = false
        break
      }
    }
    const hasTargets = statusTargetWorkspaces.every((workspace) =>
      (lastStatus.targets ?? []).some((target) => target.repo_root === workspace)
    )
    if (contentOk && hasTargets) return lastStatus
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for tracked target-origin fanout; lastStatus=${JSON.stringify(lastStatus)}`)
}

async function waitForTrackedConflictFileFanout({
  client,
  sessionId,
  attachmentId,
  getWorkspaceLiveSyncStatusRequest,
  workspaces,
  targetWorkspaces,
  provider,
  expectedContent,
  expectConflictsCleared,
  timeoutMs,
  pollMs,
}) {
  let lastStatus = null
  const relativePath = `outputs/${provider}-tracked-conflict.txt`
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    lastStatus = unwrapVariant(
      await client.send(getWorkspaceLiveSyncStatusRequest(sessionId)),
      'WorkspaceLiveSyncStatus',
    ).status
    let contentOk = true
    for (const workspace of workspaces) {
      const filePath = path.join(workspace, relativePath)
      if (!(await fileExists(filePath)) || (await readFile(filePath, 'utf8')) !== expectedContent) {
        contentOk = false
        break
      }
    }
    const conflicts = lastStatus.conflicts ?? []
    const conflictsCleared = targetWorkspaces.every((targetWorkspace) => !conflicts.some((conflict) => (
      conflict.target_repo_root === targetWorkspace &&
      conflict.path === relativePath
    )))
    if (contentOk && (!expectConflictsCleared || conflictsCleared)) return lastStatus
    await sleep(pollMs)
  }
  const contents = {}
  for (const workspace of workspaces) {
    const filePath = path.join(workspace, relativePath)
    contents[workspace] = await fileExists(filePath) ? await readFile(filePath, 'utf8') : null
  }
  throw new Error(`timed out waiting for tracked conflict file fanout; contents=${JSON.stringify(contents)}; lastStatus=${JSON.stringify(lastStatus)}`)
}

async function runTrackedTargetOriginPhase({
  client,
  session,
  attachment,
  events,
  provider,
  agent,
  workspace,
  targetWorkspaces,
  historyDir,
  timeoutMs,
  pollMs,
  getSessionStateRequest,
  getWorkspaceLiveSyncStatusRequest,
  submitPromptRequest,
}) {
  const allWorkspaces = [workspace, ...targetWorkspaces]
  const headsBefore = Object.fromEntries(await Promise.all(allWorkspaces.map(async (worktree) => [worktree, await gitHead(worktree)])))
  const completionSinceMs = Date.now()
  const marker = `${provider.toUpperCase()}_TRACKED_WORKSPACE_LIVE_SYNC_TARGET_ORIGIN_DONE`
  await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
    'This is a live Arroba workspace live sync tracked-mode target-origin drill.',
    'Use direct filesystem writes through shell/native file tools. Do not use any Arroba workspace live sync MCP/runtime tools.',
    `Run direct writes in the current workspace so that target-origin.txt becomes exactly "${provider}-target-origin-modified\\n".`,
    `Create outputs/${provider}-target-origin-added.txt containing exactly "${provider}-target-origin-added\\n".`,
    `After those direct filesystem writes complete, reply exactly ${marker} and nothing else.`,
  ].join('\n'), []))

  await waitForCompletionsAndFiles({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    events,
    expectedCompletionCount: events.filter((event) => event.event === 'assistant_message_completed').length + 1,
    completionSinceMs,
    requiredFiles: [
      path.join(targetWorkspaces[0], 'outputs', `${provider}-target-origin-added.txt`),
    ],
    forbiddenFiles: [],
    timeoutMs,
    pollMs,
    historyDir,
    providerErrorSinceMs: completionSinceMs,
  })
  await waitForHistoryOutputMarkers({
    historyDir,
    markerGroups: [[marker]],
    sinceMs: completionSinceMs,
    timeoutMs,
    pollMs,
  })
  await waitForAgentsIdle({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    agentIds: [agent.id],
    getSessionStateRequest,
    timeoutMs,
    pollMs,
  })
  const status = await waitForTrackedTargetOriginFanout({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    getWorkspaceLiveSyncStatusRequest,
    allWorkspaces,
    statusTargetWorkspaces: targetWorkspaces,
    provider,
    timeoutMs,
    pollMs,
  })
  const headsAfter = Object.fromEntries(await Promise.all(allWorkspaces.map(async (worktree) => [worktree, await gitHead(worktree)])))
  const changedHead = allWorkspaces.find((worktree) => headsAfter[worktree] !== headsBefore[worktree])
  if (changedHead) {
    const headSummary = allWorkspaces.map((worktree) => `${worktree}: ${headsBefore[worktree]} -> ${headsAfter[worktree]}`).join('; ')
    throw new Error(`tracked target-origin fanout unexpectedly created commits; ${headSummary}`)
  }
  return {
    sourceWorkspace: targetWorkspaces[0],
    targetWorkspaces: allWorkspaces.filter((worktree) => worktree !== targetWorkspaces[0]),
    headsBefore,
    headsAfter,
    status,
  }
}

async function runTrackedConflictResolutionPhase({
  client,
  session,
  attachment,
  events,
  provider,
  sourceAgent,
  resolverAgent,
  workspace,
  targetWorkspaces,
  historyDir,
  timeoutMs,
  pollMs,
  getSessionStateRequest,
  getWorkspaceLiveSyncStatusRequest,
  submitPromptRequest,
}) {
  const allWorkspaces = [workspace, ...targetWorkspaces]
  const relativePath = `outputs/${provider}-tracked-conflict.txt`
  const sourceSideContent = `one\n${provider}-tracked-source-conflict\nthree\n`
  const resolvedContent = `one\n${provider}-tracked-resolved\nthree\n`
  const targetConflictPaths = targetWorkspaces.map((targetWorkspace) =>
    path.join(targetWorkspace, relativePath)
  )

  const alignSinceMs = Date.now()
  const alignMarker = `${provider.toUpperCase()}_TRACKED_WORKSPACE_LIVE_SYNC_CONFLICT_ALIGNED`
  await client.send(submitPromptRequest(session.id, attachment.id, resolverAgent.id, [
    'This is a live Arroba workspace live sync tracked-mode conflict alignment drill.',
    'Use direct filesystem writes through shell/native file tools. Do not use any Arroba workspace live sync MCP/runtime tools.',
    `Run a direct write in the current workspace so that ${relativePath} becomes exactly "one\\n${provider}-tracked-source-conflict\\nthree\\n".`,
    ...targetConflictPaths.slice(1).map((targetPath) =>
      `Also run a direct write so that ${targetPath} becomes exactly "one\\n${provider}-tracked-source-conflict\\nthree\\n".`
    ),
    `After that direct filesystem write completes, reply exactly ${alignMarker} and nothing else.`,
  ].join('\n'), []))

  await waitForCompletionsAndFiles({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    events,
    expectedCompletionCount: 1,
    completionSinceMs: alignSinceMs,
    requiredFiles: targetConflictPaths,
    forbiddenFiles: [],
    timeoutMs,
    pollMs,
    historyDir,
    providerErrorSinceMs: alignSinceMs,
  })
  await waitForHistoryOutputMarkers({
    historyDir,
    markerGroups: [[alignMarker]],
    sinceMs: alignSinceMs,
    timeoutMs,
    pollMs,
  })
  await waitForAgentsIdle({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    agentIds: [resolverAgent.id],
    getSessionStateRequest,
    timeoutMs,
    pollMs,
  })
  const alignedStatus = await waitForTrackedConflictFileFanout({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    getWorkspaceLiveSyncStatusRequest,
    workspaces: allWorkspaces,
    targetWorkspaces,
    provider,
    expectedContent: sourceSideContent,
    expectConflictsCleared: false,
    timeoutMs,
    pollMs,
  })

  const resolveSinceMs = Date.now()
  const resolveMarker = `${provider.toUpperCase()}_TRACKED_WORKSPACE_LIVE_SYNC_CONFLICT_RESOLVED`
  await client.send(submitPromptRequest(session.id, attachment.id, sourceAgent.id, [
    'This is a live Arroba workspace live sync tracked-mode conflict resolution drill.',
    'Use direct filesystem writes through shell/native file tools. Do not use any Arroba workspace live sync MCP/runtime tools.',
    `Run a direct write in the current workspace so that ${relativePath} becomes exactly "one\\n${provider}-tracked-resolved\\nthree\\n".`,
    `After that direct filesystem write completes, reply exactly ${resolveMarker} and nothing else.`,
  ].join('\n'), []))

  await waitForCompletionsAndFiles({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    events,
    expectedCompletionCount: 1,
    completionSinceMs: resolveSinceMs,
    requiredFiles: [path.join(workspace, relativePath)],
    forbiddenFiles: [],
    timeoutMs,
    pollMs,
    historyDir,
    providerErrorSinceMs: resolveSinceMs,
  })
  await waitForHistoryOutputMarkers({
    historyDir,
    markerGroups: [[resolveMarker]],
    sinceMs: resolveSinceMs,
    timeoutMs,
    pollMs,
  })
  await waitForAgentsIdle({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    agentIds: [sourceAgent.id],
    getSessionStateRequest,
    timeoutMs,
    pollMs,
  })
  const resolvedStatus = await waitForTrackedConflictFileFanout({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    getWorkspaceLiveSyncStatusRequest,
    workspaces: allWorkspaces,
    targetWorkspaces,
    provider,
    expectedContent: resolvedContent,
    expectConflictsCleared: true,
    timeoutMs,
    pollMs,
  })

  return {
    alignedStatus,
    resolvedStatus,
    resolvedContent,
  }
}

async function runTrackedWorkspaceLiveSyncDrill({
  client,
  session,
  attachment,
  events,
  provider,
  agent,
  targetOriginAgent,
  spawnedSessionId,
  workspace,
  siblingWorkspace,
  targetWorkspace,
  targetWorkspaces,
  historyDir,
  timeoutMs,
  pollMs,
  machineRef,
  getSessionStateRequest,
  getWorkspaceLiveSyncStatusRequest,
  listProviderProcessesRequest,
  submitPromptRequest,
  startedAt,
  kernelUrl,
  options,
}) {
  const trackedTargetWorkspaces = targetWorkspaces ?? [targetWorkspace]
  let bidirectional = null
  if (options.trackedBidirectional) {
    if (!targetOriginAgent) {
      throw new Error('tracked bidirectional drill requires a target-origin agent')
    }
    bidirectional = await runTrackedTargetOriginPhase({
      client,
      session,
      attachment,
      events,
      provider,
      agent: targetOriginAgent.agent,
      workspace,
      targetWorkspaces: trackedTargetWorkspaces,
      historyDir,
      timeoutMs,
      pollMs,
      getSessionStateRequest,
      getWorkspaceLiveSyncStatusRequest,
      submitPromptRequest,
    })
    for (const worktree of [workspace, ...trackedTargetWorkspaces]) {
      await resetTrackedWorkspace(worktree)
    }
  }
  const sourceHeadBefore = await gitHead(workspace)
  const targetHeadsBefore = Object.fromEntries(await Promise.all(trackedTargetWorkspaces.map(async (target) => [target, await gitHead(target)])))
  for (const target of trackedTargetWorkspaces) {
    await writeFile(
      path.join(target, 'outputs', `${provider}-tracked-rebase.txt`),
      `alpha\n${provider}-tracked-target-local\nbeta\nomega\n`,
      'utf8',
    )
    await writeFile(
      path.join(target, 'outputs', `${provider}-tracked-conflict.txt`),
      `one\n${provider}-tracked-target-conflict\nthree\n`,
      'utf8',
    )
  }
  const linkedState = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
  const linkedSession = linkedState.session ?? linkedState
  const linkedAgents = linkedSession.agents ?? []
  const sessionAgent = linkedAgents.find((candidate) => candidate.id === agent.id)
  if (!sessionAgent) {
    throw new Error(`tracked drill spawned agent ${agent.id} session=${agent.session_id ?? agent.sessionId ?? 'unknown'} but current session ${session.id} has agents=${linkedAgents.map((candidate) => candidate.id).join(',')}; spawnedSessionId=${spawnedSessionId ?? 'unknown'}`)
  }

  const completionSinceMs = Date.now()
  const marker = `${provider.toUpperCase()}_TRACKED_WORKSPACE_LIVE_SYNC_DONE`
  const siblingWritePath = path.join(siblingWorkspace, `${provider}-tracked-sibling.txt`)
  await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
    'This is a live Arroba workspace live sync tracked-mode drill.',
    'Use direct filesystem writes through shell/native file tools. Do not use any Arroba workspace live sync MCP/runtime tools.',
    `Run direct writes in the current workspace so that tracked.txt becomes exactly "line-a\\n${provider}-tracked-modified\\n".`,
    `Create outputs/${provider}-tracked-added.txt containing exactly "${provider}-tracked-added\\n".`,
    `Create outputs/${provider}-tracked-binary.bin containing exactly the four bytes with hex 0005ff0a.`,
    `Delete outputs/${provider}-tracked-delete.txt.`,
    `Rename outputs/${provider}-tracked-rename-source.txt to outputs/${provider}-tracked-renamed.txt and make the renamed file contain exactly "${provider}-tracked-renamed\\n".`,
    `Modify outputs/${provider}-tracked-rebase.txt so it becomes exactly "alpha\\nbeta\\n${provider}-tracked-source\\nomega\\n".`,
    `Modify outputs/${provider}-tracked-conflict.txt so it becomes exactly "one\\n${provider}-tracked-source-conflict\\nthree\\n".`,
    `Create ignored/${provider}-ignored.txt containing exactly "${provider}-ignored\\n".`,
    `Create ../sibling-repo/${provider}-tracked-sibling.txt containing exactly "${provider}-tracked-sibling\\n"; this sibling repo is outside the synced workspace and must remain writable.`,
    `After those direct filesystem writes complete, reply exactly ${marker} and nothing else.`,
  ].join('\n'), []))

  await waitForCompletionsAndFiles({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    events,
    expectedCompletionCount: 1,
    completionSinceMs,
    requiredFiles: [
      path.join(workspace, 'outputs', `${provider}-tracked-added.txt`),
      path.join(workspace, 'outputs', `${provider}-tracked-renamed.txt`),
      ...(options.remoteSourceSideEffects ? [] : [
        path.join(workspace, 'ignored', `${provider}-ignored.txt`),
        siblingWritePath,
      ]),
    ],
    forbiddenFiles: [],
    timeoutMs,
    pollMs,
    historyDir,
    providerErrorSinceMs: completionSinceMs,
  })
  await waitForHistoryOutputMarkers({
    historyDir,
    markerGroups: [[marker]],
    sinceMs: completionSinceMs,
    timeoutMs,
    pollMs,
  })
  if (!options.remoteSourceSideEffects) {
    await assertFileContent(siblingWritePath, `${provider}-tracked-sibling\n`)
  }
  await waitForAgentsIdle({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    agentIds: [agent.id],
    getSessionStateRequest,
    timeoutMs,
    pollMs,
  })

  const conflictStatus = await waitForTrackedFanout({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    getWorkspaceLiveSyncStatusRequest,
    sourceWorkspace: workspace,
    targetWorkspaces: trackedTargetWorkspaces,
    provider,
    remoteSourceSideEffects: options.remoteSourceSideEffects,
    timeoutMs,
    pollMs,
  })
  await waitForHistoryNotices({
    historyDir,
    sinceMs: completionSinceMs,
    timeoutMs,
    pollMs,
    requirements: [
      {
        label: 'tracked turn summary',
        includes: [
          'Workspace live sync tracked turn summary',
          `source agent \`${agent.id}\``,
          `outputs/${provider}-tracked-added.txt`,
          'Next action:',
        ],
      },
      {
        label: 'tracked conflict notice',
        includes: [
          'Workspace live sync conflict',
          `outputs/${provider}-tracked-conflict.txt`,
          'Next action: assign a resolver agent',
        ],
      },
    ],
  })
  let resolution = null
  if (options.trackedBidirectional) {
    resolution = await runTrackedConflictResolutionPhase({
      client,
      session,
      attachment,
      events,
      provider,
      sourceAgent: agent,
      resolverAgent: targetOriginAgent.agent,
      workspace,
      targetWorkspaces: trackedTargetWorkspaces,
      historyDir,
      timeoutMs,
      pollMs,
      getSessionStateRequest,
      getWorkspaceLiveSyncStatusRequest,
      submitPromptRequest,
    })
  }
  await waitForAgentsIdle({
    client,
    sessionId: session.id,
    attachmentId: attachment.id,
    agentIds: [agent.id],
    getSessionStateRequest,
    timeoutMs,
    pollMs,
  })
  const status = resolution?.resolvedStatus ?? conflictStatus
  const sourceHeadAfter = await gitHead(workspace)
  const targetHeadsAfter = Object.fromEntries(await Promise.all(trackedTargetWorkspaces.map(async (target) => [target, await gitHead(target)])))
  const changedTargetHead = trackedTargetWorkspaces.find((target) => targetHeadsAfter[target] !== targetHeadsBefore[target])
  if (sourceHeadAfter !== sourceHeadBefore || changedTargetHead) {
    const targetHeadSummary = trackedTargetWorkspaces.map((target) => `${target}: ${targetHeadsBefore[target]} -> ${targetHeadsAfter[target]}`).join('; ')
    throw new Error(`tracked workspace live sync unexpectedly created commits; source ${sourceHeadBefore} -> ${sourceHeadAfter}; targets ${targetHeadSummary}`)
  }
  const outsideTurnPath = path.join(workspace, 'outputs', `${provider}-outside-turn.txt`)
  const outsideTurnTargetPaths = trackedTargetWorkspaces.map((target) => path.join(target, 'outputs', `${provider}-outside-turn.txt`))
  await writeFile(outsideTurnPath, `${provider}-outside-turn-change\n`, 'utf8')
  const outsideTurnStarted = Date.now()
  while (Date.now() - outsideTurnStarted < Math.min(5_000, timeoutMs)) {
    await client.send({ PumpTerminalOutput: { session_id: session.id, attachment_id: attachment.id } }).catch(() => {})
    await client.send(getWorkspaceLiveSyncStatusRequest(session.id)).catch(() => {})
    for (const outsideTurnTargetPath of outsideTurnTargetPaths) {
      if (await fileExists(outsideTurnTargetPath)) {
        throw new Error(`outside-turn tracked workspace change unexpectedly synced to target: ${outsideTurnTargetPath}`)
      }
    }
    await sleep(Math.min(500, pollMs))
  }
  const finalState = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
  const processes = unwrapVariant(await client.send(listProviderProcessesRequest()), 'ProviderProcessesListed').processes || []
  console.log(JSON.stringify({
    status: 'ok',
    mode: 'tracked-workspace-live-sync-live-drill',
    kernelUrl,
    machineRef,
    workspace,
    targetWorkspace,
    targetWorkspaces: trackedTargetWorkspaces,
    targetBranch: options.targetBranch,
    providers: [provider],
    model: options.model,
    providerModels: { [provider]: modelForProvider(provider, options) },
    durationMs: Date.now() - startedAt,
    agent: {
      id: agent.id,
      alias: agent.alias,
      provider,
    },
    completionCount: events.filter((event) => event.event === 'assistant_message_completed').length,
    terminalEventCount: events.filter((event) => event.event === 'terminal_output').length,
    tracked: {
      sourceTrackedContent: await readFile(path.join(workspace, 'tracked.txt'), 'utf8'),
      targetTrackedContent: await readFile(path.join(targetWorkspace, 'tracked.txt'), 'utf8'),
      targetAddedContent: await readFile(path.join(targetWorkspace, 'outputs', `${provider}-tracked-added.txt`), 'utf8'),
      targetRenamedContent: await readFile(path.join(targetWorkspace, 'outputs', `${provider}-tracked-renamed.txt`), 'utf8'),
      targetBinaryHex: (await readFile(path.join(targetWorkspace, 'outputs', `${provider}-tracked-binary.bin`))).toString('hex'),
      sourceRebaseContent: await readFile(path.join(workspace, 'outputs', `${provider}-tracked-rebase.txt`), 'utf8'),
      targetRebaseContent: await readFile(path.join(targetWorkspace, 'outputs', `${provider}-tracked-rebase.txt`), 'utf8'),
      sourceConflictContent: await readFile(path.join(workspace, 'outputs', `${provider}-tracked-conflict.txt`), 'utf8'),
      targetConflictContent: await readFile(path.join(targetWorkspace, 'outputs', `${provider}-tracked-conflict.txt`), 'utf8'),
      targetDeleteFileExists: await fileExists(path.join(targetWorkspace, 'outputs', `${provider}-tracked-delete.txt`)),
      targetRenameSourceFileExists: await fileExists(path.join(targetWorkspace, 'outputs', `${provider}-tracked-rename-source.txt`)),
      sourceIgnoredFileExists: await fileExists(path.join(workspace, 'ignored', `${provider}-ignored.txt`)),
      targetIgnoredFileExists: await fileExists(path.join(targetWorkspace, 'ignored', `${provider}-ignored.txt`)),
      outsideTurnSourceFileExists: await fileExists(outsideTurnPath),
      outsideTurnTargetFileExists: await fileExists(outsideTurnTargetPaths[0]),
      outsideTurnTargetFileExistsByTarget: Object.fromEntries(await Promise.all(outsideTurnTargetPaths.map(async (targetPath) => [targetPath, await fileExists(targetPath)]))),
      siblingRepoWriteContent: options.remoteSourceSideEffects ? null : await readFile(siblingWritePath, 'utf8'),
      sourceSideEffectsLocation: options.remoteSourceSideEffects ? 'remote-worker' : 'local-mirror',
      sourceHeadBefore,
      sourceHeadAfter,
      targetHeadBefore: targetHeadsBefore[targetWorkspace],
      targetHeadAfter: targetHeadsAfter[targetWorkspace],
      targetHeadsBefore,
      targetHeadsAfter,
      sourceArrobaignore: await readFile(path.join(workspace, '.arrobaignore'), 'utf8'),
      targetArrobaignore: await readFile(path.join(targetWorkspace, '.arrobaignore'), 'utf8'),
      targetArrobaignores: Object.fromEntries(await Promise.all(trackedTargetWorkspaces.map(async (target) => [target, await readFile(path.join(target, '.arrobaignore'), 'utf8')]))),
    },
    bidirectional,
    resolution,
    conflictStatus,
    workspaceLiveSyncStatus: status,
    providerProcesses: processes.map((process) => ({
      processId: process.process_id,
      provider: process.provider,
      pid: process.pid ?? null,
      ownerRunIds: process.owner_provider_run_ids || [],
    })),
    focusedAgentId: finalState.session?.focused_agent_id ?? finalState.focused_agent_id ?? null,
  }, null, 2))
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
  if (options.mode === 'tracked' && options.providers.length !== 1) {
    throw new Error('tracked live drill currently runs one provider at a time')
  }

  const runtimeDir = path.join(
    cliRoot,
    `.tmp-live-workspace-live-sync-drill-${process.pid}-${Date.now()}`,
  )
  // Keep the live workspace out of OS temp directories: Codex read-only mode may
  // allow TMPDIR writes, which would make the negative direct-write probe invalid.
  const rootDir = options.rootDir ?? path.join(cliRoot, 'target', 'live-workspace-live-sync-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const siblingWorkspace = path.join(rootDir, 'sibling-repo')
  const targetWorkspace = path.join(rootDir, 'target-workspace')
  const targetCount = options.mode === 'off' ? 0 : (options.mode === 'tracked' ? options.trackedTargetCount : options.managedTargetCount)
  const targetWorkspaces = [
    targetWorkspace,
    ...Array.from(
      { length: Math.max(0, targetCount - 1) },
      (_, index) => path.join(rootDir, `target-workspace-${index + 2}`),
    ),
  ].slice(0, targetCount)
  const outputsDir = path.join(workspace, 'outputs')
  await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(runtimeDir, { recursive: true })
  await mkdir(outputsDir, { recursive: true })
  await writeFile(path.join(workspace, 'seed.txt'), 'seed-value-42\n', 'utf8')
  for (const provider of options.providers) {
    await writeFile(path.join(outputsDir, `${provider}-delete-me.txt`), 'delete-me\n', 'utf8')
    await writeFile(path.join(outputsDir, `${provider}-opaque-delete-me.bin`), Buffer.from([9, 8, 7]))
  }
  if (options.mode === 'tracked') {
    await initTrackedWorkspace(workspace, options.providers[0])
    await mkdir(siblingWorkspace, { recursive: true })
    await writeFile(path.join(siblingWorkspace, 'README.md'), 'sibling repo\n', 'utf8')
    await initGitWorktree(siblingWorkspace)
    for (const target of targetWorkspaces) {
      await initTrackedWorkspace(target, options.providers[0], options.targetBranch)
    }
  } else {
    await initGitWorktree(workspace)
    await mkdir(siblingWorkspace, { recursive: true })
    await writeFile(path.join(siblingWorkspace, 'README.md'), 'sibling repo\n', 'utf8')
    await initGitWorktree(siblingWorkspace)
    for (const target of targetWorkspaces) {
      await initManagedTargetWorkspace(target, options.providers)
    }
  }
  if (options.afterFixtureCommand) {
    await runAfterFixtureCommand(options.afterFixtureCommand, {
      rootDir,
      workspace,
      siblingWorkspace,
      targetWorkspace,
      targetWorkspaces,
      mode: options.mode,
    })
  }

  const { LocalIpcClient, requests } = await loadCliModules(runtimeDir)
  const {
    attachToSessionRequest,
    attachWorkspaceLinkRequest,
    createWorkspaceLinkRequest,
    createSessionRequest,
    endSessionRequest,
    getWorkspaceLiveSyncStatusRequest,
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
  const workerKernelRef = await resolveRemoteWorkerKernelRef(
    client,
    requests,
    options.machineRef,
    options.providers,
    options.timeoutMs,
    options.pollMs,
  )
  let sessionId = null
  try {
    const session = unwrap(await client.send(createSessionRequest(workspace, workspace)), 'SessionCreated').session
    sessionId = session.id
    if (setWorkspaceLiveSyncModeRequest && options.mode !== 'off') {
      await client.send(setWorkspaceLiveSyncModeRequest(session.id, options.mode))
    }
    const attachment = unwrap(
      await client.send(attachToSessionRequest(session.id, `workspace-live-sync-drill-${Date.now()}`)),
      'SessionAttached',
    ).attachment
    client.onKernelEvent((event) => events.push({ ...event, observed_at_ms: Date.now() }))
    await client.subscribeToKernelEvents(session.id, attachment.id)

    if (options.mode === 'tracked' || targetWorkspaces.length > 0) {
      const linkName = `${options.mode}-live-sync-${Date.now()}`
      await client.send(createWorkspaceLinkRequest(session.id, linkName))
      await client.send(attachWorkspaceLinkRequest(session.id, linkName, workspace))
      for (const target of targetWorkspaces) {
        await client.send(attachWorkspaceLinkRequest(session.id, linkName, target))
      }
    }

    const agents = await spawnWorkspaceLiveSyncPhaseAgents({
      client,
      sessionId: session.id,
      providers: options.providers,
      modelForProvider: (provider) => modelForProvider(provider, options),
      workspace,
      kernelRef: workerKernelRef,
      spawnAgentRequest,
      aliasSuffix: 'positive',
    })
    const targetOriginAgents = options.mode === 'tracked' && options.trackedBidirectional
      ? await spawnWorkspaceLiveSyncPhaseAgents({
          client,
          sessionId: session.id,
          providers: options.providers,
          modelForProvider: (provider) => modelForProvider(provider, options),
          workspace: targetWorkspace,
          kernelRef: options.remoteSourceSideEffects ? null : workerKernelRef,
          spawnAgentRequest,
          aliasSuffix: 'target-origin',
        })
      : []

    if (options.mode === 'off') {
      const completionSinceMs = Date.now()
      const requiredFiles = []
      const directFiles = []
      for (const { provider, agent } of agents) {
        const directFile = path.join(outputsDir, `${provider}-direct.txt`)
        const siblingFile = path.join(siblingWorkspace, `${provider}-sibling.txt`)
        requiredFiles.push(directFile, siblingFile)
        directFiles.push({ provider, directFile, siblingFile })
        await client.send(submitPromptRequest(
          session.id,
          attachment.id,
          agent.id,
          [
            'This is a Workspace Live Sync off-mode drill.',
            'Use only direct filesystem writes through shell/native file tools. Do not use any Arroba workspace live sync MCP/runtime tools.',
            `Create outputs/${provider}-direct.txt in the current repository with exactly "${provider}-off-selected-root\\n".`,
            `Create ../sibling-repo/${provider}-sibling.txt with exactly "${provider}-off-sibling-root\\n".`,
            `After both direct writes complete, reply exactly ${provider.toUpperCase()}_OFF_DIRECT_WRITES_DONE and nothing else.`,
          ].join(' '),
          [],
        ))
      }
      await waitForCompletionsAndFiles({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        events,
        expectedCompletionCount: agents.length,
        completionSinceMs,
        requiredFiles,
        forbiddenFiles: [],
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        historyDir,
      })
      const status = unwrapVariant(await client.send(getWorkspaceLiveSyncStatusRequest(session.id)), 'WorkspaceLiveSyncStatus').status
      const finalState = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
      const processes = unwrapVariant(await client.send(listProviderProcessesRequest()), 'ProviderProcessesListed').processes || []
      console.log(JSON.stringify({
        status: 'ok',
        mode: 'off-workspace-live-sync-live-drill',
        kernelUrl,
        machineRef: options.machineRef,
        workspace,
        siblingWorkspace,
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
        files: await Promise.all(directFiles.map(async ({ provider, directFile, siblingFile }) => ({
          provider,
          directContent: await readFile(directFile, 'utf8'),
          siblingContent: await readFile(siblingFile, 'utf8'),
        }))),
        workspaceLiveSyncStatus: status,
        completionCount: events.filter((event) => event.event === 'assistant_message_completed').length,
        terminalEventCount: events.filter((event) => event.event === 'terminal_output').length,
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

    if (options.mode === 'tracked') {
      await runTrackedWorkspaceLiveSyncDrill({
        client,
        session,
        attachment,
        events,
        provider: options.providers[0],
        agent: agents[0].agent,
        targetOriginAgent: targetOriginAgents[0],
        spawnedSessionId: agents[0].spawnedSessionId,
        workspace,
        siblingWorkspace,
        targetWorkspace,
        targetWorkspaces,
        historyDir,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        machineRef: options.machineRef,
        getSessionStateRequest,
        getWorkspaceLiveSyncStatusRequest,
        listProviderProcessesRequest,
        submitPromptRequest,
        startedAt,
        kernelUrl,
        options,
      })
      succeeded = true
      return
    }
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
          historyDir,
          providerErrorSinceMs: completionSinceMs,
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
          historyDir,
          providerErrorSinceMs: completionSinceMs,
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
        for (const targetWorkspace of targetWorkspaces) {
          await writeFile(path.join(targetWorkspace, 'outputs', sourceName), patchInitial, 'utf8')
        }
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
    await assertManagedTargetFanout(targetWorkspaces, options.providers, { deletesApplied: false })
    if (targetWorkspaces.length > 0) {
      await waitForHistoryNotices({
        historyDir,
        sinceMs: startedAt,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        requirements: options.providers.map((provider) => ({
          label: `${provider} managed fanout summary`,
          includes: [
            'Workspace live sync managed summary',
            `outputs/${provider}.txt`,
            'target user',
            'Next action:',
          ],
        })),
      })
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
        managedTargets: await managedTargetFanoutSnapshot(targetWorkspaces, options.providers),
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
            workerKernelRef,
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
      kernelRef: workerKernelRef,
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
          historyDir,
          providerErrorSinceMs: completionSinceMs,
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
        historyDir,
        providerErrorSinceMs: completionSinceMs,
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
      kernelRef: workerKernelRef,
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
      kernelRef: workerKernelRef,
      workspace,
      outputsDir,
      historyDir,
      providerHistoryDirs: options.providerHistoryDirs,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
      getSessionStateRequest,
      spawnAgentRequest,
      submitPromptRequest,
    })
    await assertFilesAbsent(directFiles, 'final workspace live sync direct-write check')
    await assertManagedTargetFanout(targetWorkspaces, options.providers, { deletesApplied: true })

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
      managedTargets: await managedTargetFanoutSnapshot(targetWorkspaces, options.providers),
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
