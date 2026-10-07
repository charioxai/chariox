#!/usr/bin/env bun
// MP-08 / MP-10 / MP-11, A05: supplementary live refusal drill using the built
// kernel, built TUI and official Codex. Full S01–S04 acceptance requires the
// production App adapter, owning user browser and hosted conditions separately.
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { createConnection, createServer } from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { LocalIpcClient } from '../../../packages/kernel-client/dist/ipc.js'
import * as requests from '../../../packages/kernel-client/dist/ipc-requests.js'

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const options = {}
for (let index = 2; index < process.argv.length; index += 2) options[process.argv[index].replace(/^--/, '')] = process.argv[index + 1]
for (const name of ['kernel', 'cli', 'home', 'workspace-parent', 'evidence', 'provider-profile']) {
  if (!options[name] || !path.isAbsolute(options[name])) throw new Error(`--${name} must be an absolute path`)
}
if (!['darwin', 'linux'].includes(process.platform)) throw new Error('macOS or Linux is required')
options.home = path.resolve(options.home)
if (!options.home.startsWith(path.join(process.env.HOME, '.chariox/dev/'))) throw new Error('home must be below ~/.chariox/dev/')
for (const name of ['home', 'workspace-parent', 'evidence']) {
  const resolved = path.resolve(options[name])
  if (resolved === repo || resolved.startsWith(repo + path.sep)) throw new Error(`${name} must be outside the repository`)
}
process.env.CHARIOX_HOME = options.home
const report = { mp: ['MP-08', 'MP-10', 'MP-11'], node: process.env.NODE ?? null, platform: process.platform, architecture: process.arch, cases: [], status: 'running', acceptance: 'BLOCKED', scope: 'local built TUI/kernel/official Codex resource denial only' }
let stage = 'setup', kernel, tui, client, automation, sessionId, workspace, socketPath, homeCreated = false, screen = ''
const step = (name) => { stage = name; console.log(`MP-08/MP-10/MP-11 ${name}: running`) }
const passed = (name, data = {}) => { report.cases.push({ name, status: 'passed', ...data }); console.log(`MP-08/MP-10/MP-11 ${name}: passed`) }
const requireValue = (value) => {
  if (!value) { report.failedAssertionStage = stage; throw new Error(stage) }
}
function stopOwned(child, signal) {
  if (!child || child.exitCode != null) return
  if (!Number.isInteger(child.pid) || child.pid <= 1) throw new Error('invalid owned child PID')
  child.kill(signal)
}
async function exitOwned(child, name) {
  if (!child) return
  const ended = await Promise.race([child.exited.then(() => true), Bun.sleep(5_000).then(() => false)])
  if (!ended) {
    stopOwned(child, 'SIGKILL')
    report[`forced${name}Exit`] = true
    await child.exited
  }
}
const visibleScreen = () => screen.replace(/\x1b\[[0-9;?<>=]*[ -/]*[@-~]|\x1b[()][0-9A-Za-z]|\x1b[=>78]|\x1b\][^\x07\x1b]*(\x07|\x1b\\)/g, '')
async function press(key) {
  tui.terminal.write(key)
  await Bun.sleep(400)
}
async function freePort() {
  const server = createServer()
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  const port = server.address().port
  await new Promise((resolve) => server.close(resolve))
  return port
}
async function until(check, timeout = 90_000) {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) {
    if (/\b401\b|unauthorized/i.test(screen)) {
      report.providerUnauthorized = true
      console.error('MP-08/MP-10/MP-11 official provider returned unauthorized; coordinator action required')
      throw new Error(stage)
    }
    if (await check()) return
    await Bun.sleep(250)
  }
  report.timedOutStage = stage
  throw new Error(stage)
}
async function connectAutomation(socketPath) {
  const socket = createConnection(socketPath)
  await new Promise((resolve, reject) => { socket.once('connect', resolve); socket.once('error', reject) })
  let sequence = 0, buffer = ''
  const pending = new Map()
  socket.on('data', (chunk) => {
    buffer += chunk.toString()
    while (buffer.includes('\n')) {
      const end = buffer.indexOf('\n'), frame = JSON.parse(buffer.slice(0, end)); buffer = buffer.slice(end + 1)
      const waiter = pending.get(frame.id); pending.delete(frame.id)
      if (frame.ok) waiter?.resolve(frame.data); else {
        waiter?.reject(new Error(stage))
      }
    }
  })
  return {
    send(action, fields = {}) {
      const id = ++sequence
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject })
        socket.write(JSON.stringify({ id, action, ...fields }) + '\n')
      })
    },
    close() { socket.destroy() },
  }
}
try {
  await mkdir(options.evidence, { recursive: true })
  await mkdir(path.dirname(options.home), { recursive: true, mode: 0o700 })
  await mkdir(options.home, { recursive: process.platform === 'darwin', mode: 0o700 })
  homeCreated = process.platform === 'linux'
  report.sourceCommit = (await new Response(Bun.spawn(["git", "rev-parse", "HEAD"], {cwd: repo, stdout: "pipe"}).stdout).text()).trim()
  report.kernelSourceCommit = options['kernel-source-commit'] ?? report.sourceCommit
  workspace = await mkdtemp(path.join(options['workspace-parent'], 'capability-grant-drill-'))
  socketPath = `/tmp/chariox-capability-${process.pid}.sock`
  const port = await freePort(), mcpPort = await freePort(), codexPort = await freePort(), opencodePort = await freePort()
  const url = `ws://127.0.0.1:${port}`
  const inherited = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith('CHARIOX_')))
  const env = { ...inherited, CHARIOX_HOME: options.home, CHARIOX_LOG_DIR: path.join(options.home, 'logs'), CHARIOX_KERNEL_PORT: String(port), CHARIOX_MCP_PORT: String(mcpPort), CHARIOX_CODEX_PORT: String(codexPort), CHARIOX_OPENCODE_PORT: String(opencodePort), CHARIOX_ROOM_AGENT_TOOLS: '1', CHARIOX_TEST_TUI: '1', CHARIOX_DAEMON_SOCKET: path.join(options.home, 'daemon.sock'), CHARIOX_KERNEL_BROWSER_SCRIPT: path.join(repo, 'apps/kernel/slice-linux-docker/docker/kernel-browser-linux.mjs') }
  step('built-kernel-ready')
  kernel = Bun.spawn([options.kernel], { cwd: repo, env, stdout: 'ignore', stderr: 'ignore' })
  report.kernelPid = kernel.pid
  client = new LocalIpcClient(url, {})
  await until(async () => { try { return Boolean((await client.send(requests.listSessionsRequest())).SessionsListed) } catch { return false } }, 30_000)
  passed(stage)
  step('native-codex-login-available')
  let imported
  try {
    imported = await client.send(options['provider-profile']
      ? requests.linkProviderAccountProfileRequest('codex', 'capability-drill', options['provider-profile'])
      : requests.importNativeProviderAccountProfileRequest('codex'))
  } catch (error) {
    // A new kernel can register its bootstrap profile automatically. Reuse the
    // exact profile identified by the product's duplicate-scope response.
    const existing = String(error).match(/\(([A-Za-z0-9_-]+)\); use that profile instead/)
    if (!options['provider-profile'] || !existing) throw error
    imported = await client.send(requests.getProviderAccountProfileRequest('codex', existing[1]))
    report.reusedExistingProviderProfile = true
  }
  requireValue(imported.ProviderAccountProfile?.profile)
  const profileId = imported.ProviderAccountProfile.profile.profile_id
  const refreshed = await client.send(requests.refreshProviderAccountProfileRequest('codex', profileId))
  report.nativeAuthState = refreshed.ProviderAccountProfile.profile.auth_state
  requireValue(report.nativeAuthState.toLowerCase() === 'authenticated')
  await client.send(requests.setDefaultProviderAccountProfileRequest('codex', profileId))
  passed(stage)
  step('room-created')
  const created = await client.send(requests.createSessionRequest(workspace, workspace, 'capability-live', {provider: 'codex', model: 'default'}))
  sessionId = created.SessionCreated.session.id
  report.sessionId = sessionId
  passed(stage)
  step('rendered-tui-attached')
  tui = Bun.spawn([options.cli, '--kernel-url', url, '--session', sessionId, '--automation-socket', socketPath], {
    cwd: repo, env: {...env, TERM: 'xterm-256color'},
    terminal: {cols: 160, rows: 48, data(_terminal, chunk) { screen = (screen + new TextDecoder().decode(chunk)).slice(-100_000) }},
  })
  report.tuiPid = tui.pid
  await until(async () => {
    if (!screen.includes('Ctrl+T hotkeys')) return false
    try { automation = await connectAutomation(socketPath); return true } catch { return false }
  }, 30_000)
  const attached = await automation.send('snapshot')
  requireValue(attached.session?.id === sessionId && attached.attachmentId)
  passed(stage)
  step('owner-revokes-existing-focus-grants-through-tui')
  await automation.send('submit_prompt', {prompt: '/access revoke all'})
  const grants = async () => (await client.send(requests.kernelBrowserRequest({op: 'list_grants'}))).KernelBrowser.result.grants
  await until(async () => (await grants()).length === 0, 10_000)
  passed(stage)
  step('official-codex-requests-browser-tools-through-tui')
  await automation.send('submit_prompt', {prompt: 'Use the Chariox kernel browser to open https://www.wikipedia.org/. First call the Chariox load_kernel_browser tool exactly once. Do not use shell tools, files or another browser. If access is denied, stop and report the denial.'})
  let approval
  await until(async () => {
    const snapshot = await automation.send('snapshot')
    approval = snapshot.interactions.find(item => item.title === 'Chariox resource access')
    return Boolean(approval)
  }, 180_000)
  requireValue(approval.agentId == null && approval.choices.some(choice => choice.id === 'deny'))
  requireValue((await grants()).length === 0)
  // Kernel-owned decisions use the shared F8 approval panel. Agent interaction
  // automation does not drive that panel, so use the actual terminal keys.
  await press('\x1b[19~')
  await until(async () => visibleScreen().includes('Chariox resource access'), 10_000)
  await writeFile(path.join(options.evidence, 'resource-approval-tui.ansi'), screen)
  passed(stage, {newAuthorityBeforeOwnerReply: false})
  step('owner-denies-resource-acquisition-through-tui')
  // Opening starts with no selected decision. Up selects the final Deny
  // choice; Down would select the first Allow choice.
  requireValue(approval.choices.at(-1)?.id === 'deny')
  await press('\x1b[A')
  await until(async () => visibleScreen().includes('Selected: Deny'), 5_000)
  await press('\r')
  await until(async () => !(await automation.send('snapshot')).interactions.some(item => item.id === approval.id), 30_000)
  await Bun.sleep(2_000)
  requireValue((await grants()).length === 0)
  await writeFile(path.join(options.evidence, 'resource-denied-tui.ansi'), screen)
  passed(stage, {newAuthorityAfterDenial: false})
  report.status = 'passed'
  report.blockers = [
    'S01/S03: trusted installed production GitHub App with admitted executable adapter/key/capabilities and linked real GitHub account is required.',
    'S01/S03: non-root owning kernel with linked provider and user-domain Chromium is required on this root-only builder.',
    'S04/hosted acceptance: authorized isolated hosted wss enrollment and leased worker topology are required; existing relay and Apps machines are off-limits.',
    'Full acceptance still requires every fixed public site at DPR 1/2, shaped real relay network and multi-hour stability.'
  ]
} catch (error) {
  if (screen) {
    await Bun.sleep(250)
    await writeFile(path.join(options.evidence, 'failed-tui.ansi'), screen)
  }
  report.providerUnauthorized = report.providerUnauthorized || /401|unauthorized/i.test(String(error))
  report.status = 'failed'; report.failedStage = stage
  console.error(`MP-08/MP-10/MP-11 ${stage}: failed`)
  process.exitCode = 1
} finally {
  step('cleanup-owned-processes')
  if (sessionId) await client?.send(requests.endSessionRequest(sessionId)).catch(() => {})
  if (tui?.exitCode == null) tui?.terminal?.write('\x05')
  automation?.close()
  stopOwned(tui, 'SIGTERM')
  await client?.close().catch(() => {})
  stopOwned(kernel, 'SIGTERM')
  await exitOwned(tui, 'Tui')
  tui?.terminal?.close()
  await exitOwned(kernel, 'Kernel')
  if (socketPath) await rm(socketPath, {force: true})
  // Linux uses a new disposable home. Preserve macOS kernel homes under the
  // existing operator key-retention contract.
  if (workspace) await rm(workspace, {recursive: true, force: true})
  if (homeCreated) await rm(options.home, {recursive: true, force: true})
  report.cleanup = {kernelExited: true, tuiExited: true, ownedWorkspaceRemoved: true, disposableKernelHomeRemoved: homeCreated, kernelHomeRetained: process.platform === 'darwin' ? options.home : null}
  await writeFile(path.join(options.evidence, 'capability-live-refusal.json'), JSON.stringify(report, null, 2) + '\n')
  console.log(`MP-08/MP-10/MP-11 drill: ${report.status}; acceptance: ${report.acceptance}`)
}
