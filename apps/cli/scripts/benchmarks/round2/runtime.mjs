// MP-08/MP-10/MP-11: isolated ordinary kernel lifecycle for benchmark adapters.
import assert from 'node:assert/strict'
import { spawn, execFile } from 'node:child_process'
import { createWriteStream } from 'node:fs'
import { readFile, writeFile, mkdir, mkdtemp, chmod, rm, statfs, appendFile } from 'node:fs/promises'
import { createHash, randomBytes } from 'node:crypto'
import { promisify } from 'node:util'
import net from 'node:net'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { stopOwnedProcess } from './owned-processes.mjs'
import { observeKernelRpcErrors } from './rpc-errors.mjs'
import { sanitizeDrillMetadata } from '../../lib/drill-secrets.mjs'

const exec = promisify(execFile)
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
const hash = async file => createHash('sha256').update(await readFile(file)).digest('hex')
const one = (response, kind) => { assert(response?.[kind], `MP-10 expected ${kind}`); return response[kind] }
const port = () => new Promise(resolve => {
  const server = net.createServer()
  server.listen(0, '127.0.0.1', () => { const value = server.address().port; server.close(() => resolve(value)) })
})

export async function startOwnedRuntime({ lane, evidence, release, repo, clientRoot, image, accountHome, label, viewport, sandboxCompatibility = true }) {
  for (const value of [lane, evidence, release, repo, clientRoot, accountHome]) assert(path.isAbsolute(value), 'MP-10 absolute runtime roots required')
  assert(lane.startsWith('/root/.chariox/dev/browser-resume-20260930/agents/r2next/'))
  assert(evidence.startsWith('/root/.codex/evidence/browser-resume-20260930/r2next/'))
  assert.match(label, /^r2next-[a-z0-9-]+$/)
  assert.match(image, /^sha256:[a-f0-9]{64}$/)
  assert.equal(typeof sandboxCompatibility, 'boolean')
  await mkdir(evidence, { recursive: true, mode: 0o700 })
  await mkdir(lane, { recursive: true, mode: 0o700 })
  const root = await mkdtemp(`${lane}/runtime-`)
  const workspace = await mkdtemp('/root/work/agent-r2next-runtime-workspace-')
  await chmod(root, 0o711); await chmod(workspace, 0o777)
  const children = []
  const report = { mpItems: ['MP-08', 'MP-10', 'MP-11'], label, root, workspace, resources: [], unsigned: true }
  const save = () => writeFile(`${evidence}/runtime.json`, JSON.stringify(sanitizeDrillMetadata(report), null, 2) + '\n', { mode: 0o600 })
  let client, monitor, closing = false, interrupted = false
  const interrupt = () => { interrupted = true }
  process.on('SIGTERM', interrupt); process.on('SIGINT', interrupt)
  const guard = async () => {
    const memory = Number((await readFile('/proc/meminfo', 'utf8')).match(/MemAvailable:\s+(\d+)/)[1]) * 1024
    const fs = await statfs('/')
    const sample = { at: new Date().toISOString(), memAvailable: memory, diskAvailable: fs.bavail * fs.bsize }
    report.resources.push(sample)
    await appendFile(`${evidence}/runtime-resources.jsonl`, JSON.stringify(sample) + '\n')
    if (memory < 16 * 1024 ** 3 || sample.diskAvailable < 10 * 1024 ** 3) interrupted = true
    if (interrupted) throw new Error('MP-10 runtime interruption/resource floor')
    return sample
  }
  async function close({ preserveState = false } = {}) {
    if (closing) return report.cleanup
    closing = true; clearInterval(monitor)
    const cleanup = { processes: [], stateRemoved: false, workspaceRemoved: false, unresolvedRooms: [], unresolvedSlices: [] }
    if (preserveState) cleanup.externalOwnershipUnsettled = true
    if (client) {
      try {
        const sessions = one(await client.send({ ListSessions: null }), 'SessionsListed').sessions
        cleanup.unresolvedRooms = sessions.map(session => session.id)
        const slices = one(await client.send({ ListSlices: null }), 'SlicesListed').slices
        cleanup.unresolvedSlices = slices.map(slice => slice.id)
      } catch { cleanup.inventoryUnavailable = true }
      await client.close()
    }
    for (const child of [...children].reverse()) {
      try { cleanup.processes.push({ pid: child.pid, stopped: await stopOwnedProcess(child, { detached: true }) }) }
      catch (error) { cleanup.processes.push({ pid: child.pid, stopped: false, errorClass: error.name }) }
    }
    // Preserve unsettled state for recovery; never erase a live Room's authority.
    if (!preserveState && !cleanup.inventoryUnavailable && !cleanup.unresolvedRooms.length && !cleanup.unresolvedSlices.length && cleanup.processes.every(row => row.stopped)) {
      assert(root.startsWith(`${lane}/runtime-`) && workspace.startsWith('/root/work/agent-r2next-runtime-workspace-'))
      await rm(root, { recursive: true }); cleanup.stateRemoved = true
      await rm(workspace, { recursive: true }); cleanup.workspaceRemoved = true
    }
    report.cleanup = cleanup; report.finishedAt = new Date().toISOString(); await save()
    process.off('SIGTERM', interrupt); process.off('SIGINT', interrupt)
    return cleanup
  }
  try {
    await guard()
    const manifest = JSON.parse(await readFile(`${release}/usr/lib/chariox/r2next-dev-manifest.json`, 'utf8'))
    const kernel = `${release}/usr/local/bin/chariox-kernel`
    const relay = `${release}/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/chariox-relay`
    for (const [name, file] of [['chariox-kernel', kernel], ['chariox-relay', relay]]) {
      assert.equal(`sha256:${await hash(file)}`, manifest.artifacts.find(entry => entry.name === name)?.sha256)
    }
    const labels = JSON.parse((await exec('docker', ['image', 'inspect', image, '--format', '{{json .Config.Labels}}'])).stdout)
    assert.equal(labels['io.chariox.runtime-source-revision'], manifest.runtimeSourceRevision)
    report.source = { ...manifest, image, imageLabels: labels,
      protocol: (await exec(kernel, ['--print-local-daemon-protocol-version'])).stdout.trim(),
      clientIpcSha256: await hash(`${clientRoot}/dist/ipc.js`) }
    const [{ LocalIpcClient }, requests] = await Promise.all(['ipc', 'ipc-requests'].map(name => import(pathToFileURL(`${clientRoot}/dist/${name}.js`))))
    for (const dir of ['home', 'runtime', 'codex', 'claude', 'opencode', 'config', 'data', 'cache', 'xdg-state']) await mkdir(`${root}/${dir}`, { mode: 0o700 })
    await exec('git', ['init', '-q', workspace])
    await writeFile(`${root}/runtime/config.toml`, `version = 1\n[state]\npath = "${root}/state.db"\n[slices]\nroot = "${root}/slices"\n[slices.linux]\ndocker_image = "${image}"\nbuild_image = "never"\nallow_provider_sandbox_compatibility = ${sandboxCompatibility}\nmemory_mb = 2048\ncpus = "1"\nscreen_width = ${viewport.width}\nscreen_height = ${viewport.height}\n[credential_vault]\nbackend = "process_memory"\npath = "${root}/vault.db"\nservice = "${label}"\nagent_management = "allow"\n`, { mode: 0o600 })
    const ports = []; for (let i = 0; i < 5; i++) ports.push(await port())
    const env = { PATH: process.env.PATH, LANG: 'C.UTF-8', HOME: `${root}/home`, CODEX_HOME: `${root}/codex`,
      CLAUDE_CONFIG_DIR: `${root}/claude`, OPENCODE_CONFIG_DIR: `${root}/opencode`, XDG_CONFIG_HOME: `${root}/config`,
      XDG_DATA_HOME: `${root}/data`, XDG_STATE_HOME: `${root}/xdg-state`, XDG_CACHE_HOME: `${root}/cache`,
      CHARIOX_HOME: `${root}/runtime`, CHARIOX_LOG_DIR: `${root}/logs`, CHARIOX_DAEMON_SOCKET: `${root}/runtime/kernel.sock`,
      CHARIOX_KERNEL_HOST: '127.0.0.1', CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]),
      CHARIOX_CODEX_PORT: String(ports[2]), CHARIOX_OPENCODE_PORT: String(ports[3]), CHARIOX_RELAY_HOST: '0.0.0.0',
      CHARIOX_RELAY_PORT: String(ports[4]), CHARIOX_RELAY_URL: `ws://127.0.0.1:${ports[4]}`,
      CHARIOX_RELAY_TOKEN: randomBytes(32).toString('hex'), CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT: '1',
      CHARIOX_DAEMON_ALIAS: label, CHARIOX_SLICE_LOCAL_DEV_OWNER_UID: '0',
      CHARIOX_SLICE_LOCAL_DEV_RUNTIME_REVISION: manifest.runtimeSourceRevision,
      CHARIOX_SLICE_DOCKER_PROVISIONER: `${repo}/apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh`,
      CHARIOX_SLICE_DOCKER_PIDS_LIMIT: '1024' }
    for (const [binary, role] of [[relay, 'relay'], [kernel, 'kernel']]) {
      const child = spawn(binary, [], { cwd: root, env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] })
      await new Promise((resolve, reject) => { child.once('spawn', resolve); child.once('error', reject) })
      assert(Number.isSafeInteger(child.pid) && child.pid > 1)
      const log = createWriteStream(`${root}/${role}.log`, { mode: 0o600 })
      child.stdout.pipe(log); child.stderr.pipe(log); children.push(child)
    }
    report.ownedPids = children.map(child => child.pid); report.ports = ports; await save()
    const kernelUrl = `ws://127.0.0.1:${ports[0]}/kernel`
    const deadline = Date.now() + 120000
    while (Date.now() < deadline) {
      await guard()
      const candidate = new LocalIpcClient(kernelUrl, { controlResponseStallMs: 240000 })
      try { await candidate.send(requests.listSessionsRequest()); client = candidate; break }
      catch { await candidate.close(); await pause(500) }
    }
    assert(client, 'MP-10 kernel startup deadline')
    observeKernelRpcErrors(client, async failure => appendFile(`${evidence}/runtime-rpc-errors.jsonl`, JSON.stringify(failure) + '\n'))
    assert.equal(one(await client.send({ GetDaemonHealth: null }), 'DaemonHealth').projection.process.process_id, children[1].pid)
    monitor = setInterval(() => guard().catch(() => { interrupted = true }), 5000)
    const profile = one(await client.send(requests.linkProviderAccountProfileRequest('codex', label, accountHome)), 'ProviderAccountProfile').profile
    const status = one(await client.send(requests.refreshProviderAccountProfileRequest('codex', profile.profile_id)), 'ProviderAccountProfile').profile
    assert.equal(status.auth_state, 'authenticated', 'MP-08 product-linked provider not authenticated')
    report.account = { profileId: profile.profile_id, authState: status.auth_state }; await save()
    return { root, workspace, client, requests, kernelUrl, profileId: profile.profile_id, source: report.source, guard, close }
  } catch (error) {
    report.failure = { errorClass: error.name, message: 'MP-10 isolated runtime preflight failed' }
    await close(); throw error
  }
}
