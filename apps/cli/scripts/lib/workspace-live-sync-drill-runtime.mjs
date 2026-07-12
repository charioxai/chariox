import { spawn } from 'node:child_process'
import { access, mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { resolveBuiltBinary } from './drill-runtime-helpers.mjs'

export async function loadCliModules(runtimeDir, cliRoot) {
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

export function makePorts() {
  const base = 57000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort: base,
    mcpPort: base + 1000,
    opencodePort: base + 2000,
    codexPort: base + 2001,
  }
}

export async function resolveBinary(binaryPath, manifestPath, binName) {
  return await resolveBuiltBinary(binaryPath, manifestPath, binName)
}

export async function terminateChild(child, signal = 'SIGTERM') {
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

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
export const unwrap = (resp, key) => resp?.[key] ?? resp
export const unwrapVariant = (resp, ...keys) => keys.map((key) => resp?.[key]).find((value) => value != null) ?? resp

export function requestName(request) {
  if (!request || typeof request !== 'object') return String(request)
  return Object.keys(request)[0] ?? 'unknown'
}

export function withTimeout(promise, timeoutMs, label) {
  let timer = null
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => {
      reject(new Error(`${label} timed out after ${timeoutMs}ms`))
    }, timeoutMs)
  })
  return Promise.race([promise, timeout]).finally(() => {
    if (timer) clearTimeout(timer)
  })
}

export function wrapClientSendWithTimeout(client, timeoutMs) {
  const rawSend = client.send.bind(client)
  client.send = (request) => withTimeout(
    rawSend(request),
    timeoutMs,
    `daemon request ${requestName(request)}`,
  )
}

export async function runCommand(command, args, cwd) {
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

export async function initGitWorktree(workspace, branch = 'main') {
  await runCommand('git', ['init'], workspace)
  await runCommand('git', ['config', 'user.email', 'workspace-live-sync-drill@example.com'], workspace)
  await runCommand('git', ['config', 'user.name', 'Workspace Live Sync Drill'], workspace)
  await runCommand('git', ['add', '.'], workspace)
  await runCommand('git', ['commit', '-m', 'seed workspace live sync fixture'], workspace)
  if (branch && branch !== 'main') {
    await runCommand('git', ['checkout', '-b', branch], workspace)
  }
}

export async function initTrackedWorkspace(workspace, provider, branch = 'main') {
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

export async function gitHead(workspace) {
  const { stdout } = await runCommand('git', ['rev-parse', 'HEAD'], workspace)
  return stdout.trim()
}

export async function resetTrackedWorkspace(workspace) {
  await runCommand('git', ['reset', '--hard', 'HEAD'], workspace)
  await runCommand('git', ['clean', '-fdx'], workspace)
}

export async function runAfterFixtureCommand(command, context) {
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

export async function initManagedTargetWorkspace(workspace, providers) {
  const outputsDir = path.join(workspace, 'outputs')
  await mkdir(outputsDir, { recursive: true })
  await writeFile(path.join(workspace, 'seed.txt'), 'seed-value-42\n', 'utf8')
  for (const provider of providers) {
    await writeFile(path.join(outputsDir, `${provider}-delete-me.txt`), 'delete-me\n', 'utf8')
    await writeFile(path.join(outputsDir, `${provider}-opaque-delete-me.bin`), Buffer.from([9, 8, 7]))
  }
  await initGitWorktree(workspace)
}

export function workspaceLiveSyncSpawnAgentRequest(spawnAgentRequest, sessionId, provider, alias, model, worktreeId, effort, kernelRef) {
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

export function modelForProvider(provider, options) {
  const explicit = options.providerModels[provider]
  if (explicit) return explicit
  if (provider === 'opencode' && !options.model.includes('/')) return `opencode/${options.model}`
  return options.model
}

export function workspaceLiveSyncToolNames(provider) {
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

export function workspaceLiveSyncMoveSourceName(provider) {
  return provider === 'opencode' ? `${provider}-source.txt` : `${provider}-patch.txt`
}

export async function spawnWorkspaceLiveSyncPhaseAgents({
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

export async function destroyWorkspaceLiveSyncAgent({ client, destroyAgentRequest, sessionId, agent }) {
  if (!destroyAgentRequest || !agent?.id) return
  await client.send(destroyAgentRequest(sessionId, agent.id)).catch(() => {})
}

export async function waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace, worktree) {
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

export async function resolveRemoteWorkerKernelRef(client, requests, machineRef, providers, timeoutMs, pollMs) {
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

export async function fileExists(filePath) {
  try {
    await access(filePath)
    return true
  } catch {
    return false
  }
}

export async function assertFileContent(filePath, expected) {
  const actual = await readFile(filePath, 'utf8')
  if (actual !== expected) {
    throw new Error(`unexpected content for ${filePath}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
  }
  return actual
}

export async function assertFileBytes(filePath, expected) {
  const actual = await readFile(filePath)
  const expectedBytes = Buffer.from(expected)
  if (!actual.equals(expectedBytes)) {
    throw new Error(`unexpected bytes for ${filePath}: expected ${expectedBytes.toString('hex')}, got ${actual.toString('hex')}`)
  }
  return actual
}
