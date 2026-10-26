#!/usr/bin/env node
// MP-08 / MP-11 BY-03: when Cloud ends this terminal's login, the real TUI shows ONE definitive
// state ("Signed out. Sign in again: run /cloud login.") and never a stale waiting room or a raw
// "directory unavailable" error. Real TUI (bun dist) in a PTY against a real hosted Cloud (HTTPS)
// through the shaped host network, driven by the product's own commands:
//   /cloud login (dev device approval) -> My kernels ready -> the same profile signs out from
//   another terminal (`chariox cloud logout`) -> this TUI is refreshed -> signed-out state -> /cloud login again.
// TUIOUT_PHASE=red runs the base TUI (expected to FAIL); green runs the change.
import assert from 'node:assert/strict'
import net from 'node:net'
import path from 'node:path'
import {spawn} from 'node:child_process'
import {pathToFileURL} from 'node:url'
import {mkdir, mkdtemp, open, readFile, writeFile, rm} from 'node:fs/promises'

const required = key => { assert(process.env[key], `${key} required`); return process.env[key] }
const E = required('TUIOUT_EVIDENCE'), state = required('TUIOUT_STATE_ROOT'), cliDir = required('TUIOUT_CLI_DIR')
const origin = required('TUIOUT_ORIGIN'), relay = required('TUIOUT_RELAY_URL'), cloudRoot = required('TUIOUT_CLOUD_ROOT')
const scriptDir = path.dirname(new URL(import.meta.url).pathname), terminal = path.join(scriptDir, 'lib/owner-context-terminal.py')
const {startHostNetwork} = await import(pathToFileURL(path.join(cloudRoot, 'scripts/lib/project-environment-host-network.mjs')))
const stackEnv = Object.fromEntries((await readFile(required('TUIOUT_STACK_CLOUD_ENV'), 'utf8')).split('\n').filter(line => line.includes('=')).map(line => [line.slice(0, line.indexOf('=')), line.slice(line.indexOf('=') + 1)]))
const devSecret = stackEnv.CHARIOX_CLOUD_DEV_AUTH_SECRET
assert(devSecret, 'stack dev secret')
for (const dir of [E, state]) await mkdir(dir, {recursive: true})
const scratch = await mkdtemp(path.join(state, 'tuiout-')), home = path.join(scratch, 'home'), socket = path.join(scratch, 'tui.sock'), tuiLog = path.join(scratch, 'tui.log')
await mkdir(home, {recursive: true, mode: 0o700})
const phase = required('TUIOUT_PHASE'), slug = `byomrel-tuiout-${Date.now().toString(36)}`
const report = {mpItems: ['MP-08', 'MP-11'], area: 'apps-byom/BY-03', phase, cloud: required('TUIOUT_CLOUD_HEAD'), oss: required('TUIOUT_OSS_HEAD'), cells: [], started: new Date().toISOString(),
  scope: 'Real TUI (bun dist) in a PTY, real hosted Cloud over HTTPS through the shaped host network, product dev device login. Real WorkOS is not involved; the Cloud-side trigger is the profile signing out from another terminal (client revoked).'}
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
const safe = text => String(text).replaceAll(devSecret, '[redacted]')
const save = () => writeFile(path.join(E, 'RESULT.json'), JSON.stringify(report, null, 2))
const children = []
let network, tui
const run = (command, args, env = {}) => new Promise((resolve, reject) => {
  const child = spawn(command, args, {cwd: cliDir, env: {...process.env, ...env}, stdio: ['ignore', 'pipe', 'pipe']}); let out = ''
  child.stdout.on('data', b => { out += b }); child.stderr.on('data', b => { out += b })
  child.on('error', reject); child.on('exit', code => code === 0 ? resolve(out) : reject(Error(`${path.basename(command)} exited ${code}: ${safe(out).slice(-300)}`)))
})
const automation = (request, timeoutMs = 15000) => new Promise((resolve, reject) => {
  const s = net.createConnection(socket); let text = ''
  s.setTimeout(timeoutMs, () => s.destroy(Error('TUI automation timeout'))); s.on('error', reject)
  s.on('connect', () => s.write(JSON.stringify({id: 1, ...request}) + '\n'))
  s.on('data', b => { text += b; const n = text.indexOf('\n'); if (n < 0) return; s.end(); const reply = JSON.parse(text.slice(0, n)); reply.ok ? resolve(reply.data) : reject(Error('TUI action refused')) })
})
async function until(fn, label, timeout = 60000) {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) { const value = await fn().catch(() => false); if (value) return value; await pause(200) }
  throw Error(`${label} timed out`)
}
const roomRows = snapshot => snapshot.waitingRoom?.rows ?? []
const cloudStatus = snapshot => roomRows(snapshot).filter(row => String(row.id ?? '').startsWith('cloud-notice')).map(row => row.value).join(' | ')
const header = snapshot => roomRows(snapshot).find(row => row.id === 'relay-header')
const headerValue = snapshot => header(snapshot)?.value
const signedIn = async () => { await automation({action: 'submit_prompt', prompt: '/cloud status'}); return /Terminal profile:/.test(cloudStatus(await automation({action: 'snapshot'}))) }
async function shot(name) {
  const log = path.join(E, name + '.log'); await writeFile(log, safe(await readFile(tuiLog, 'utf8')))
  await run('python3', [terminal, '--render', log, path.join(E, name + '.png')])
}
const used = new Set()
async function approvePending(label) {
  // The device code is printed by the TUI itself; a person would type it into the browser.
  const code = await until(async () => {
    const text = (await readFile(tuiLog, 'utf8')).replace(/\x1b\[[0-9;?]*[A-Za-z]/g, ''), codes = [...text.matchAll(/code=([A-Z0-9]{4}-[A-Z0-9]{4})/g)].map(match => match[1])
    return codes.find(value => !used.has(value))
  }, `${label}: device code shown`, 60000)
  used.add(code)
  const response = await fetch(`${origin}/auth/dev/device/approve`, {method: 'POST', headers: {'content-type': 'application/json', 'x-chariox-dev-auth-secret': devSecret},
    body: JSON.stringify({userCode: code, email: `${slug}@chariox.invalid`, accountSlug: slug, displayName: 'BY-03 TUI signed-out drill', providerSubject: `dev|${slug}`})})
  assert.equal(response.status, 200, `${label}: dev approval`)
}
async function main() {
  network = await startHostNetwork(relay, {httpsOrigins: [origin]}); report.networkCalibration = network.calibration
  const env = {CHARIOX_HOME: home, HOME: home, CHARIOX_CLOUD_API_URL: origin, HTTPS_PROXY: network.proxyUrl, https_proxy: network.proxyUrl, TERM: 'xterm-256color', TMPDIR: path.join(scratch, 'tmp'), NODE_OPTIONS: ''}
  await mkdir(env.TMPDIR, {recursive: true})
  const log = await open(tuiLog, 'w', 0o600)
  tui = spawn('python3', [terminal, 'bun', 'dist/index.js', '--detached', '--automation-socket', socket], {cwd: cliDir, env: {...process.env, ...env}, stdio: ['pipe', log.fd, log.fd], detached: false}); children.push(tui); await log.close()
  await until(async () => (await automation({action: 'snapshot'})).waitingRoom, 'real TUI waiting room')
  await shot('01-signed-out-start')

  const cell = {cell: 'tui-signed-out', status: 'FAIL', signedInCloudStatus: null}; report.cells.push(cell)
  // /cloud login returns when the person has approved in the browser: keep it pending.
  const login1 = automation({action: 'submit_prompt', prompt: '/cloud login'}, 180000)
  await approvePending('login 1'); await login1
  await until(signedIn, 'TUI signed in')
  await automation({action: 'refresh_waiting_room'}); await pause(1500)
  cell.signedInCloudStatus = cloudStatus(await automation({action: 'snapshot'})); await shot('02-signed-in-my-kernels')

  // The same profile signs out from another terminal: the product's own command.
  await run('bun', ['dist/index.js', 'cloud', 'logout'], env)
  const endedAt = Date.now()
  await automation({action: 'refresh_waiting_room'})
  const verdict = await until(async () => {
    const snapshot = await automation({action: 'snapshot'}), text = cloudStatus(snapshot)
    return /Signed out\. Sign in again/.test(text) ? {text, header: headerValue(snapshot)} : false
  }, 'definitive signed-out state', phase === 'red' ? 15000 : 30000).catch(() => null)
  const after = await automation({action: 'snapshot'})
  cell.afterLogout = {cloudStatus: cloudStatus(after), headerTitle: header(after)?.title, header: headerValue(after), visibleMs: verdict ? Date.now() - endedAt : null}
  await shot('03-after-signout-elsewhere')
  // It must stay: sample over several seconds, no flash that disappears and no later "directory unavailable".
  await pause(6000)
  const later = await automation({action: 'snapshot'}); cell.stillShownAfter6s = /Signed out\. Sign in again/.test(cloudStatus(later)); cell.headerAfter6s = headerValue(later)
  await shot('04-signed-out-after-6s')
  const screenText = safe(await readFile(tuiLog, 'utf8')).replace(/\x1b\[[0-9;?]*[A-Za-z]/g, '')
  cell.rawErrorOnScreen = /directory unavailable|Cloud client authentication failed|client_revoked|signed_out\)/.test(cloudStatus(later) + ' ' + (cell.headerAfter6s ?? ''))

  // Sign in again with the normal login action.
  if (verdict) {
    const login2 = automation({action: 'submit_prompt', prompt: '/cloud login'}, 180000)
    await approvePending('login 2'); await login2
    await until(signedIn, 'TUI signed in again')
    cell.signedInAgain = true; await shot('05-signed-in-again')
  }
  cell.status = verdict && cell.stillShownAfter6s && !cell.rawErrorOnScreen && cell.signedInAgain ? 'PASS' : 'FAIL'
  await save(); console.log('BY-03 TUI signed-out', JSON.stringify({phase, status: cell.status, afterLogout: cell.afterLogout, stillShownAfter6s: cell.stillShownAfter6s}))
}
try { await main(); report.status = report.cells.every(cell => cell.status === 'PASS') ? 'PASS' : 'FAIL' }
catch (error) { report.status = 'FAIL'; report.error = safe(error.message); report.lastRows = await automation({action: 'snapshot'}).then(snapshot => roomRows(snapshot).map(row => [row.id, row.title, row.value]), () => null); await shot('failure').catch(() => {}) }
finally {
  report.network = await network?.snapshot?.().catch(() => null); report.finished = new Date().toISOString(); await save()
  try { await automation({action: 'exit'}) } catch {}
  await pause(500); for (const child of children) if (child.pid && child.pid > 1 && child.exitCode === null) child.kill('SIGTERM')
  await network?.close?.().catch(() => {}); await rm(scratch, {recursive: true, force: true})
}
process.exit(report.status === 'PASS' ? 0 : 1)
