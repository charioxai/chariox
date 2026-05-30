#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const {
  attachToSessionRequest,
  createSessionRequest,
  createSliceRequest,
  deleteSliceRequest,
  endSessionRequest,
  getSliceDisplayEndpointRequest,
  getSliceRequest,
  importSliceProviderAuthRequest,
  listSessionsRequest,
  listSlicesRequest,
  startSliceRequest,
  stopSliceRequest,
} = await import('../../../packages/kernel-client/dist/ipc-requests.js')

function log(message, details) {
  if (details === undefined) console.log(`[slice-lifecycle-drill] ${message}`)
  else console.log(`[slice-lifecycle-drill] ${message}`, JSON.stringify(details))
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

async function assertDockerReady() {
  const result = await run('docker', ['info', '--format', '{{json .ServerVersion}}'])
  if (result.code !== 0) {
    throw new Error(`Docker is required for the slice lifecycle drill. Start Docker/Colima and retry.\n${result.stdout}${result.stderr}`)
  }
}

async function buildKernel() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return binary
}

function startDaemon(binary, env) {
  const child = spawn(binary, [], {
    cwd: repoRoot,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  child.stdoutText = ''
  child.stderrText = ''
  child.stdout.on('data', (chunk) => { child.stdoutText += chunk.toString() })
  child.stderr.on('data', (chunk) => { child.stderrText += chunk.toString() })
  return child
}

async function stopDaemon(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  child.kill('SIGTERM')
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    new Promise((resolve) => setTimeout(resolve, 3_000)),
  ])
  if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
}

async function waitForDaemon(kernelUrl) {
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(listSessionsRequest())
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await new Promise((resolve) => setTimeout(resolve, 250))
    }
  }
  throw new Error(`daemon did not become ready: ${lastError?.message ?? String(lastError)}`)
}

function variant(response, key) {
  if (!response?.[key]) throw new Error(`expected ${key}, got ${JSON.stringify(response)}`)
  return response[key]
}

function assert(condition, message) {
  if (!condition) throw new Error(message)
}

async function main() {
  await assertDockerReady()

  const root = path.join(repoRoot, 'target', 'live-slice-lifecycle-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(root, 'workspace')
  const home = path.join(root, 'home')
  const configHome = path.join(root, 'config')
  const stateHome = path.join(root, 'state')
  const kernelPort = 50500 + Math.floor(Math.random() * 1000)
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const env = {
    ...process.env,
    HOME: home,
    XDG_CONFIG_HOME: configHome,
    XDG_STATE_HOME: stateHome,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(kernelPort + 1000),
    ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
    ARROBA_CODEX_PORT: String(kernelPort + 2001),
    ARROBA_DAEMON_ID: `slice-lifecycle-drill-${process.pid}-${Date.now()}`,
    ARROBA_DAEMON_SOCKET: path.join(root, 'daemon.sock'),
  }

  let daemon = null
  let client = null
  let succeeded = false
  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(path.join(configHome, 'arroba'), { recursive: true })
    await mkdir(path.join(home, '.codex'), { recursive: true })
    await writeFile(path.join(configHome, 'arroba', 'config.toml'), 'version = 1\n', 'utf8')
    await writeFile(path.join(home, '.codex', 'auth.json'), JSON.stringify({ account_id: 'slice-drill-account', auth_mode: 'api-key' }), 'utf8')

    const kernelBinary = await buildKernel()
    daemon = startDaemon(kernelBinary, env)
    await waitForDaemon(kernelUrl)
    client = new LocalIpcClient(kernelUrl)

    const created = variant(await client.send(createSliceRequest({
      name: 'slice-drill',
      displayMode: 'headed',
      workspaceId: workspace,
      worktreeId: workspace,
      workspaceMount: workspace,
    })), 'SliceCreated').slice
    assert(created.display_mode === 'headed', 'created slice should be headed')
    assert(created.worktree_id === workspace, 'created slice should be scoped to the worktree')
    log('created', { slice: created.id, displayMode: created.display_mode })

    const listed = variant(await client.send(listSlicesRequest()), 'SlicesListed').slices
    assert(listed.some((slice) => slice.id === created.id), 'created slice should be listed')

    const started = variant(await client.send(startSliceRequest(created.id)), 'SliceStarted').slice
    assert(started.status === 'running', `slice should be running after start, got ${started.status}`)
    assert(started.worker_kernel_id, 'started slice should discover a worker kernel')
    log('started', { slice: started.id, workerKernelId: started.worker_kernel_id })

    const endpoint = variant(await client.send(getSliceDisplayEndpointRequest(created.id)), 'SliceDisplayEndpoint').endpoint
    assert(endpoint.url, 'headed slice should expose a display endpoint URL')
    log('screen', { url: endpoint.url, access: endpoint.access })

    const imported = variant(await client.send(importSliceProviderAuthRequest(created.id, 'codex')), 'SliceProviderAuthImported')
    assert(imported.status === 'imported', `provider auth import should succeed, got ${imported.status}`)
    assert(imported.slice.provider_auth?.some((auth) => auth.provider === 'codex' && auth.account_id === 'slice-drill-account'), 'codex account summary should be recorded on the slice')
    log('auth-imported', { provider: imported.provider })

    const session = variant(await client.send(createSessionRequest(workspace, workspace, 'slice-drill-session', undefined, created.id)), 'SessionCreated').session
    await client.send(attachToSessionRequest(session.id, `slice-lifecycle-drill-${process.pid}`))
    const withAgent = variant(await client.send(getSliceRequest(created.id)), 'Slice').slice
    assert(withAgent.agent_ids?.length > 0, 'slice should track the session initial agent')
    let deleteBlocked = false
    try {
      await client.send(deleteSliceRequest(created.id))
    } catch {
      deleteBlocked = true
    }
    assert(deleteBlocked, 'delete should be blocked while an agent is attached')
    log('reuse-bound', { agents: withAgent.agent_ids?.length ?? 0, sessions: withAgent.session_ids?.length ?? 0 })

    await client.send(endSessionRequest(session.id))
    await client.send(stopSliceRequest(created.id))
    const deleted = variant(await client.send(deleteSliceRequest(created.id)), 'SliceDeleted').slice
    assert(deleted.id === created.id, 'deleted slice id should match')
    succeeded = true
    log('pass', { workspace })
  } finally {
    await client?.close?.().catch(() => {})
    await stopDaemon(daemon)
    if (succeeded) await rm(root, { recursive: true, force: true })
    else log('artifacts-kept', { root, daemonStdout: daemon?.stdoutText?.slice(-2000), daemonStderr: daemon?.stderrText?.slice(-2000) })
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
