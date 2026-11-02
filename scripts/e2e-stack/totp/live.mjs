// MP-08 / MP-09 / MP-10 / MP-11: real provider + kernel, NEW UI only.
// Local fixture evidence supplements, and never closes, real-service AU-10.
import assert from 'node:assert/strict'
import { createHash, randomBytes } from 'node:crypto'
import { createWriteStream } from 'node:fs'
import { cp, mkdir, mkdtemp, readFile, readdir, rm, writeFile, chmod, chown } from 'node:fs/promises'
import { execFileSync } from 'node:child_process'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { LocalIpcClient } from '../../../packages/kernel-client/dist/ipc.js'
import { spawnOwned, signalOwnedProcess, ownedProcessHandles } from '../../../apps/kernel/slice-linux-docker/owned-process-signals.mjs'
import { startTotpSite } from './site.mjs'

const tree = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const required = name => { const value = process.env[name]; assert.ok(value?.startsWith('/'), `${name} requires an absolute path`); return value }
const binary = required('CHARIOX_TOTP_KERNEL')
const profile = required('CHARIOX_TOTP_PROFILE')
const root = required('CHARIOX_TOTP_STATE')
const evidence = required('CHARIOX_TOTP_EVIDENCE')
const origin = process.env.CHARIOX_TOTP_ORIGIN ?? 'https://val.51-255-87-178.sslip.io'
const dpr = Number(process.env.DPR ?? 1)
assert.ok([1, 2].includes(dpr))
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
const name = `vtotp-${dpr}-${Date.now()}`
const passphrase = randomBytes(32).toString('hex')
const bootstrapSecret = randomBytes(24).toString('hex')
const results = { MP: ['MP-08', 'MP-09', 'MP-10', 'MP-11'], dpr, client: 'terminal-os', provider: 'codex', model: 'gpt-6-luna', effort: 'low', interactions: [], localFixture: true, accepted: false }
const record = async () => writeFile(path.join(evidence, 'result.json'), JSON.stringify(results, null, 2) + '\n', { mode: 0o600 })
const variant = (value, key) => { assert.ok(value[key], `expected ${key}, received ${Object.keys(value).join(',')}`); return value[key] }
const freePort = () => new Promise((resolve, reject) => { const server = net.createServer(); server.on('error', reject); server.listen(0, '127.0.0.1', () => { const value = server.address().port; server.close(() => resolve(value)) }) })
async function until(check, label, timeout = 60_000) {
  const end = Date.now() + timeout
  while (Date.now() < end) { const value = await check(); if (value) return value; await sleep(500) }
  throw Error(`timed out: ${label}`)
}
async function texts(directory) {
  let output = ''
  for (const entry of await readdir(directory, { withFileTypes: true }).catch(() => [])) {
    const file = path.join(directory, entry.name)
    if (entry.isDirectory()) output += await texts(file)
    else if (/\.(jsonl|log)$/.test(entry.name)) output += await readFile(file, 'utf8')
  }
  return output
}
let kernel, client, browser, site, page, browserRoot, sessionId
let rawKernelLog = ''
let stateEnvironment, workspace
async function screenshot(step) { await page.screenshot({ path: path.join(evidence, `${step}.png`) }) }

try {
  await mkdir(root, { recursive: true, mode: 0o700 })
  await mkdir(evidence, { recursive: true, mode: 0o700 })
  results.head = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: tree, encoding: 'utf8' }).trim()
  results.kernelSha256 = createHash('sha256').update(await readFile(binary)).digest('hex')
  results.localProtocol = Number(execFileSync(binary, ['--print-local-daemon-protocol-version'], { encoding: 'utf8' }).trim())
  assert.equal(results.localProtocol, 945, 'use the S8 kernel; the base has no TOTP kind')
  workspace = process.env.CHARIOX_TOTP_WORKSPACE ?? `/root/work/agent-vtotp-drill-${dpr}-${process.pid}`
  assert.ok(workspace.startsWith('/root/work/agent-vtotp-drill-'), 'use an owned drill workspace outside runtime state')
  await mkdir(workspace)
  execFileSync('git', ['init', '-q', '-b', 'totp-drill', workspace])
  const home = path.join(root, 'home')
  const config = path.join(root, 'config')
  await mkdir(path.join(config, 'chariox'), { recursive: true })
  await mkdir(home)
  await writeFile(path.join(config, 'chariox/config.toml'), `version = 1\n[credential_vault]\nbackend = "chariox_encrypted"\npath = "${home}/vault/vault.json"\nunlock_policy = "kernel_init"\nagent_management = "allow"\n`, { mode: 0o600 })
  const ports = await Promise.all(Array.from({ length: 4 }, freePort))
  stateEnvironment = { ...process.env, CHARIOX_HOME: home, XDG_CONFIG_HOME: config,
    XDG_STATE_HOME: path.join(root, 'state'), XDG_CACHE_HOME: path.join(root, 'cache'),
    CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]),
    CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]),
    CHARIOX_DAEMON_SOCKET: path.join(root, 'kernel.sock'), CHARIOX_SESSION_HISTORY_DIR: path.join(root, 'history'),
    CHARIOX_DAEMON_ID: name, TMPDIR: process.env.TMPDIR,
    CHARIOX_KERNEL_BROWSER_DISPLAY: '1', CHARIOX_KERNEL_BROWSER_MIRROR: '1' }
  if (process.getuid?.() === 0) {
    // The real host adapter and pinned Chromium run as a normal Unix user.
    // Never disable Chromium's sandbox or change permissions on shared paths.
    browserRoot = await mkdtemp('/var/tmp/vtotp-browser-')
    await chmod(browserRoot, 0o755)
    const assets = path.join(browserRoot, 'assets'), owned = path.join(browserRoot, 'owned')
    await cp(path.join(tree, 'apps/kernel/slice-linux-docker/docker'), assets, { recursive: true })
    await mkdir(owned, { mode: 0o700 })
    const uid = Number(execFileSync('id', ['-u', 'ubuntu'], { encoding: 'utf8' }).trim())
    const gid = Number(execFileSync('id', ['-g', 'ubuntu'], { encoding: 'utf8' }).trim())
    await chown(owned, uid, gid)
    const wrapper = path.join(browserRoot, 'node-host')
    const quote = value => "'" + value.replaceAll("'", "'\\''") + "'"
    await writeFile(wrapper, `#!/bin/sh\nset -eu\n[ "$#" -eq 3 ] && [ "$2" = stdio ]\nexec /usr/sbin/runuser -u ubuntu -- /usr/bin/env -i PATH=/usr/local/bin:/usr/bin:/bin HOME=/home/ubuntu TMPDIR=${quote(owned)} CHARIOX_KERNEL_BROWSER_HEADLESS=1 CHARIOX_KERNEL_BROWSER_DISPLAY=1 CHARIOX_KERNEL_BROWSER_MIRROR=1 /usr/bin/node "$1" stdio ${quote(owned)}\n`, { mode: 0o700 })
    stateEnvironment.CHARIOX_BROWSER_CONTROLLER_NODE = wrapper
    stateEnvironment.CHARIOX_KERNEL_BROWSER_SCRIPT = path.join(assets, 'kernel-browser-host.mjs')
    results.browserAdapter = 'real host adapter as ubuntu; sandbox enabled; headless pinned Chromium'
  }
  const log = createWriteStream(path.join(evidence, 'kernel.private.log'), { mode: 0o600 })
  kernel = spawnOwned(binary, [], { cwd: workspace, env: stateEnvironment, stdio: ['ignore', 'pipe', 'pipe'], detached: true })
  results.kernelPid = kernel.pid
  for (const output of [kernel.stdout, kernel.stderr]) output.on('data', chunk => { rawKernelLog += chunk.toString(); log.write(chunk) })
  const url = `ws://127.0.0.1:${ports[0]}`
  await until(async () => {
    if (kernel.exitCode !== null) throw Error('owned kernel exited')
    const candidate = new LocalIpcClient(url, { localAuthEnvironment: stateEnvironment, timeoutMs: 2000 })
    try { await candidate.send({ ListSessions: null }); client = candidate; return true } catch { await candidate.close(); return false }
  }, 'kernel ready')
  const linked = variant(await client.send({ LinkProviderAccountProfile: { provider: 'codex', label: name, path: profile } }), 'ProviderAccountProfile').profile
  await client.send({ SetDefaultProviderAccountProfile: { provider: 'codex', account_profile: linked.profile_id } })
  const { chromium } = await import(process.env.CHARIOX_TOTP_PLAYWRIGHT ?? '/w/valenv/tools/node_modules/playwright-core/index.mjs')
  // This is only the test viewer. The source browser above retains its sandbox.
  browser = await chromium.launch({ headless: true, executablePath: '/opt/google/chrome/chrome', args: ['--no-sandbox', '--disable-dev-shm-usage'] })
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: dpr })
  for (const permission of ['local-network-access', 'local-network', 'loopback-network']) { try { await context.grantPermissions([permission], { origin }) } catch {} }
  page = await context.newPage()
  await page.goto(origin + '/validation/ready')
  const before = variant(await client.send({ RelayStatus: null }), 'RelayStatus').status
  const login = variant(await client.send({ StartCloudRelayLogin: { api_url: origin, client_id: name, client_alias: name, machine_id: before.machine_id, machine_alias: name } }), 'CloudRelayLoginStarted').login
  assert.equal(new URL(login.verification_url).origin, origin)
  await page.goto(login.verification_url)
  await page.getByRole('button', { name: /approve|allow|connect/i }).first().click()
  await until(async () => {
    const result = variant(await client.send({ PollCloudRelayLogin: { api_url: origin, device_code: login.device_code } }), 'CloudRelayLoginPolled').result
    if (result.status === 'denied') throw Error('device enrollment denied')
    return result.status === 'approved'
  }, 'own kernel enrollment', 120_000)
  await page.goto(origin + '/terminal?ui=os', { waitUntil: 'domcontentloaded' })
  await page.locator('[data-feature-new="room"]').waitFor({ timeout: 60_000 })
  await page.locator('[data-feature-new="room"]').click()
  await page.locator('[data-new-room]').waitFor()
  async function chooseRow(row) {
    const selector = page.locator(`[data-new-room] [data-row="${row}"]`)
    if (!await selector.count()) return
    await selector.click()
    const choices = page.locator('[data-new-room-choices] [role^="menuitem"]')
    await choices.first().waitFor({ timeout: 30_000 })
    const labels = await choices.allInnerTexts()
    const index = labels.findIndex(text => text.includes(name))
    assert.ok(index >= 0, `${row} choices must contain this drill's kernel`)
    await choices.nth(index).click()
  }
  await chooseRow('machine')
  await chooseRow('kernel')
  await page.locator('[data-new-room] [data-row="alias"]').click()
  await page.locator('[data-new-room] input[aria-label="Alias"]').fill(name)
  await page.keyboard.press('Enter')
  await page.locator('[data-new-room] button[type="submit"]').click()
  await page.locator('[data-prompt-input]').waitFor({ timeout: 60_000 })
  const session = await until(async () => variant(await client.send({ ListSessions: null }), 'SessionsListed').sessions.find(entry => entry.alias === name), 'new-UI Room')
  sessionId = session.id
  const agent = variant(await client.send({ SpawnAgent: { session_id: sessionId, provider: 'codex', account_profile: linked.profile_id, alias: 'TOTP', model: 'gpt-6-luna', effort: 'low', execution_mode: 'build', permission_level: 'yolo', worktree_id: null, kernel_ref: null, worktree_placement: null } }), 'AgentSpawned').agent
  await page.locator('[data-room-agent]').filter({ hasText: 'TOTP' }).first().click()
  await screenshot('01-new-ui-room')
  site = await startTotpSite()
  async function submit(prompt) { await page.locator('[data-prompt-input]').fill(prompt); await page.locator('[data-prompt-input]').press('Enter') }
  const answered = new Set()
  async function answerOwnerInput() {
    const session = variant(await client.send({ GetSessionState: { session_id: sessionId } }), 'SessionState').session
    for (const interaction of session.active_interactions ?? []) {
      if (answered.has(interaction.id)) continue
      const title = String(interaction.title ?? '')
      let value
      if (/Unlock Chariox Vault/.test(title)) value = passphrase
      else if (/TOTP bootstrap/.test(title)) value = bootstrapSecret
      else if (/TOTP enrollment key/.test(title)) value = `otpauth://totp/Local:fixture?secret=${site.supplySeedToOwner()}&issuer=Local`
      else if (/TOTP recovery codes/.test(title)) value = JSON.stringify([site.backupCodeForOwner()])
      else if (/Authorize sudo window/.test(title)) value = passphrase
      else continue
      let popup = page.locator('[data-passkey-prompt]').first()
      if (!await popup.isVisible().catch(() => false)) {
        const stripe = page.locator('[data-passkey-stripe]').first()
        if (await stripe.count()) await stripe.locator('button').first().click()
        else await page.locator('[data-action="reply-interaction"]').first().click()
        popup = page.locator('[data-passkey-prompt], [data-interaction-detail]').first()
      }
      await popup.waitFor({ timeout: 20_000 })
      const field = popup.locator('input[type="password"]')
      await field.fill(value)
      if (await popup.locator('[data-passkey-popup-approve]').count()) await popup.locator('[data-passkey-popup-approve]').click()
      else await popup.locator('button.work-editor-save').first().click()
      answered.add(interaction.id)
      results.interactions.push({ title, kind: interaction.kind, answeredIn: 'terminal-os masked field' })
      await sleep(300)
    }
  }
  const request = (id, injection, title) => JSON.stringify({ credential: { id, allowed_hosts: [new URL(site.origin).host], allowed_uses: ['browser'], injection }, prompt: { title, message: 'Enter the disposable local test-site value in the protected field.', timeout_sec: 180 }, overwrite: false })
  await submit('Use chariox.request_credential_secret with ' + request('bootstrap', { kind: 'browser' }, 'TOTP bootstrap') + '. This creates the disposable test Vault. Do not use shell/file tools or ask for the value in chat. After storage reply BOOTSTRAP_READY.')
  await until(async () => { await answerOwnerInput(); return (await page.locator('.agent-pane').filter({ hasText: 'TOTP' }).first().innerText()).includes('BOOTSTRAP_READY') }, 'protected Vault bootstrap', 240_000)
  await screenshot('02-protected-bootstrap')
  await page.locator('[data-action="open-room-view"]').last().click()
  await page.locator('[data-open-room-view="browser"]').click()
  await page.locator('.kernel-browser-omnibox').first().waitFor({ timeout: 60_000 })
  await submit('/sudo Complete this local 2FA test using Chariox browser tools. First call chariox.request_credential_secret with ' + request('authenticator', { kind: 'totp' }, 'TOTP enrollment key') + '. Then load the kernel browser tools and navigate to ' + site.origin + '/enable. Find the observed Authentication code input and fill it only using chariox.kernel_browser_paste_secret credential_id=authenticator, submit=false. Take a browser snapshot and screenshot BEFORE clicking enable; verify the filled field is redacted. Click the observed enable button and verify Two-factor authentication enabled. Log out. Wait at least 31 seconds using a shell sleep (do not calculate or read any codes). Sign in by filling the observed Authentication code field with the same credential handle; click login and verify Signed in. Log out, wait at least another 31 seconds, then repeat sign in in another period. Never read/echo any key or code, never use page evaluation or direct HTTP to submit a code. Use fresh observed node_ref/document_id/generation for every fill. Keep tools moving until both sign-ins succeeded; report TOTP_TWO_PERIODS_DONE. Do not finish early.')
  await until(async () => { await answerOwnerInput(); const stats = site.stats(); return stats.enrollmentSuccesses === 1 && stats.loginSuccesses >= 2 }, 'agent enrollment and two-period sign-in', 480_000)
  await screenshot('03-two-period-sign-in')
  const transcript = await texts(path.join(root, 'history'))
  const visible = await page.locator('.agent-pane').filter({ hasText: 'TOTP' }).first().innerText()
  assert.ok(site.checkAbsence(transcript + visible + rawKernelLog), 'seed/code absence check failed')
  assert.ok(!transcript.includes(passphrase) && !transcript.includes(bootstrapSecret), 'protected input absence check failed')
  results.absencePassed = true
  results.site = site.stats()
  results.passed = true
  results.note = 'Real kernel/provider/new-UI local fixture drill; real-service AU-10 remains separate.'
  console.log('PASS MP-08 / MP-09 / MP-10 / MP-11 local TOTP drill, DPR' + dpr)
} catch (error) {
  results.passed = false
  const message = String(error.message)
  const safe = !message.includes(passphrase) && !message.includes(bootstrapSecret) && (!site || site.checkAbsence(message))
  results.failure = safe ? message : 'private-value absence check failed'
  if (page) await screenshot('failure').catch(() => {})
  console.error('FAIL MP-08 / MP-09 / MP-10 / MP-11 local TOTP drill; see result.json')
  process.exitCode = 1
} finally {
  if (client && sessionId) await client.send({ EndSession: { session_id: sessionId } }).catch(() => {})
  if (client) await client.send({ DisconnectCloudRelay: null }).catch(() => {})
  await client?.close().catch(() => {})
  await browser?.close().catch(() => {})
  await site?.close().catch(() => {})
  if (kernel) {
    const handles = ownedProcessHandles(kernel)
    if (kernel.exitCode === null && kernel.signalCode === null) signalOwnedProcess(kernel, 'SIGTERM')
    await until(() => kernel.exitCode !== null || kernel.signalCode !== null, 'own kernel exit', 20_000).catch(() => {})
    for (const handle of handles) { try { signalOwnedProcess(handle, 'SIGKILL') } catch {} }
    results.cleanup = { kernelPid: kernel.pid, exitCode: kernel.exitCode, signal: kernel.signalCode }
  }
  await record()
  await rm(root, { recursive: true, force: true })
  if (browserRoot) await rm(browserRoot, { recursive: true, force: true })
  if (workspace) await rm(workspace, { recursive: true, force: true })
}
