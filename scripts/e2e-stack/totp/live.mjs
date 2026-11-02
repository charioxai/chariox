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
import { spawnOwned, signalOwnedProcess, ownedProcessHandles, trackOwnedProcess } from '../../../apps/kernel/slice-linux-docker/owned-process-signals.mjs'
import { startTotpSite } from './site.mjs'

const tree = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const required = name => { const value = process.env[name]; assert.ok(value?.startsWith('/'), `${name} requires an absolute path`); return value }
const binary = required('CHARIOX_TOTP_KERNEL')
const profile = required('CHARIOX_TOTP_PROFILE')
const root = required('CHARIOX_TOTP_STATE')
const evidence = required('CHARIOX_TOTP_EVIDENCE')
const origin = process.env.CHARIOX_TOTP_ORIGIN
assert.ok(origin?.startsWith('https://pr') && origin.includes('vtotp.'), 'use this lane’s owned hosted new-UI stack')
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
  results.phase = label
  await record()
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
let kernel, client, browser, browserServer, viewer, site, page, browserRoot, sessionId
let rawKernelLog = ''
let stateEnvironment, workspace
let ownsRoot = false, ownsWorkspace = false
async function deadline(promise, ms = 5000) {
  let timer
  try { return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(Error('owned cleanup deadline')), ms) })]) } finally { clearTimeout(timer) }
}
async function screenshot(step) { await page.screenshot({ path: path.join(evidence, `${step}.png`), timeout: 5000 }) }

try {
  await mkdir(evidence, { recursive: true, mode: 0o700 })
  await mkdir(root, { mode: 0o700 })
  ownsRoot = true
  results.driverSha256 = createHash('sha256').update(await readFile(fileURLToPath(import.meta.url))).digest('hex')
  results.fixtureSha256 = createHash('sha256').update(await readFile(new URL('./site.mjs', import.meta.url))).digest('hex')
  results.driverHead = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: tree, encoding: 'utf8' }).trim()
  results.kernelSha256 = createHash('sha256').update(await readFile(binary)).digest('hex')
  const provenance = JSON.parse(await readFile(path.join(path.dirname(binary), 'PROVENANCE.json'), 'utf8'))
  assert.equal(provenance['chariox-kernel'], results.kernelSha256, 'kernel provenance must match the executable')
  results.head = provenance.head
  results.worktree = provenance.worktree
  results.localProtocol = Number(execFileSync(binary, ['--print-local-daemon-protocol-version'], { encoding: 'utf8' }).trim())
  assert.equal(results.localProtocol, 945, 'use the S8 kernel; the base has no TOTP kind')
  workspace = process.env.CHARIOX_TOTP_WORKSPACE ?? `/root/work/agent-vtotp-drill-${dpr}-${process.pid}`
  assert.ok(workspace.startsWith('/root/work/agent-vtotp-drill-'), 'use an owned drill workspace outside runtime state')
  await mkdir(workspace)
  ownsWorkspace = true
  execFileSync('git', ['init', '-q', '-b', 'totp-drill', workspace])
  const home = path.join(root, 'home')
  const config = path.join(root, 'config')
  await mkdir(path.join(config, 'chariox'), { recursive: true })
  await mkdir(home)
  await writeFile(path.join(home, 'config.toml'), `version = 1\n[history.operational]\npath = "${home}/history/operational.db"\n[credential_vault]\nbackend = "chariox_encrypted"\npath = "${home}/vault/vault.json"\nunlock_policy = "kernel_init"\nagent_management = "allow"\n`, { mode: 0o600 })
  const ports = await Promise.all(Array.from({ length: 4 }, freePort))
  const inheritedEnvironment = { ...process.env }
  for (const key of Object.keys(inheritedEnvironment)) { if (key.startsWith('CHARIOX_') || ['CODEX_THREAD_ID', 'CODEX_SESSION_ID'].includes(key)) delete inheritedEnvironment[key] }
  stateEnvironment = { ...inheritedEnvironment, CHARIOX_HOME: home, XDG_CONFIG_HOME: config,
    XDG_STATE_HOME: path.join(root, 'state'), XDG_CACHE_HOME: path.join(root, 'cache'),
    CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]),
    CHARIOX_OPENCODE_PORT: String(ports[2]), CHARIOX_CODEX_PORT: String(ports[3]),
    CHARIOX_DAEMON_SOCKET: path.join(root, 'kernel.sock'), CHARIOX_SESSION_HISTORY_DIR: path.join(root, 'history'),
    CHARIOX_DAEMON_ID: name, CHARIOX_DAEMON_ALIAS: name, CHARIOX_MACHINE_ALIAS: name, CODEX_HOME: profile, TMPDIR: process.env.TMPDIR,
    CHARIOX_KERNEL_BROWSER_DISPLAY: '1', CHARIOX_KERNEL_BROWSER_MIRROR: '1' }
  if (process.getuid?.() === 0) {
    // The real host adapter and pinned Chromium run as a normal Unix user.
    // Never disable Chromium's sandbox or change permissions on shared paths.
    browserRoot = await mkdtemp('/opt/vtotp-browser-')
    results.browserScratch = browserRoot
    await chmod(browserRoot, 0o755)
    const assets = path.join(browserRoot, 'assets'), owned = path.join(browserRoot, 'owned')
    await cp(path.join(tree, 'apps/kernel/slice-linux-docker/docker'), assets, { recursive: true })
    await mkdir(owned, { mode: 0o700 })
    const uid = Number(execFileSync('id', ['-u', 'ubuntu'], { encoding: 'utf8' }).trim())
    const gid = Number(execFileSync('id', ['-g', 'ubuntu'], { encoding: 'utf8' }).trim())
    await chown(owned, uid, gid)
    const sourceChrome = process.env.CHARIOX_TOTP_SOURCE_CHROME ?? '/usr/lib/chariox/browser/chromium/155.0.8059.79/chrome-linux64/chrome'
    assert.ok(sourceChrome.startsWith('/'), 'source Chromium must be an absolute path')
    results.sourceBrowserVersion = execFileSync(sourceChrome, ['--version'], { encoding: 'utf8' }).trim()
    assert.ok(results.sourceBrowserVersion.includes('155.0.8059.79'), 'use product-pinned Chrome for Testing')
    const wrapper = path.join(browserRoot, 'node-host')
    const quote = value => "'" + value.replaceAll("'", "'\\''") + "'"
    await writeFile(wrapper, `#!/bin/sh\nset -eu\n[ "$#" -eq 3 ] && [ "$2" = stdio ]\nexec /usr/bin/setpriv --reuid=${uid} --regid=${gid} --clear-groups /usr/bin/env -i PATH=/usr/local/bin:/usr/bin:/bin HOME=${quote(owned)} TMPDIR=${quote(owned)} CHARIOX_KERNEL_BROWSER_EXECUTABLE=${quote(sourceChrome)} CHARIOX_KERNEL_BROWSER_DISPLAY=1 CHARIOX_KERNEL_BROWSER_MIRROR=1 /usr/bin/node "$1" stdio ${quote(owned)}\n`, { mode: 0o700 })
    stateEnvironment.CHARIOX_BROWSER_CONTROLLER_NODE = wrapper
    stateEnvironment.CHARIOX_KERNEL_BROWSER_SCRIPT = path.join(assets, 'kernel-browser-host.mjs')
    results.browserAdapter = 'real host adapter as ubuntu; sandbox enabled; headed pinned Chromium'
  }
  const log = createWriteStream(path.join(evidence, 'kernel.private.log'), { mode: 0o600 })
  kernel = spawnOwned(binary, [], { cwd: workspace, env: stateEnvironment, stdio: ['ignore', 'pipe', 'pipe'], detached: true })
  results.kernelPid = kernel.pid
  results.workspace = workspace
  await record()
  for (const output of [kernel.stdout, kernel.stderr]) output.on('data', chunk => { rawKernelLog += chunk.toString(); log.write(chunk) })
  const url = `ws://127.0.0.1:${ports[0]}`
  await until(async () => {
    if (kernel.exitCode !== null) throw Error('owned kernel exited')
    const candidate = new LocalIpcClient(url, { localAuthEnvironment: stateEnvironment, controlRequestRetryDeadlineMs: 2000, controlResponseStallMs: 2000 })
    try { await candidate.send({ ListSessions: null }); client = candidate; return true } catch { await candidate.close(); return false }
  }, 'kernel ready')
  const profiles = variant(await client.send({ ListProviderAccountProfiles: { provider: 'codex' } }), 'ProviderAccountProfilesListed').profiles
  const linked = profiles.find(entry => entry.provider === 'codex' && entry.is_default) ?? variant(await client.send({ LinkProviderAccountProfile: { provider: 'codex', label: name, path: profile } }), 'ProviderAccountProfile').profile
  await client.send({ SetDefaultProviderAccountProfile: { provider: 'codex', account_profile: linked.profile_id } })
  const refreshed = variant(await client.send({ RefreshProviderAccountProfile: { provider: 'codex', account_profile: linked.profile_id } }), 'ProviderAccountProfile').profile
  results.providerAuthState = refreshed.auth_state
  assert.equal(refreshed.auth_state, 'authenticated', 'linked official provider must be signed in')
  console.log('MP-08 / MP-09 / MP-10 / MP-11: own kernel and linked provider ready')
  const { chromium } = await import(process.env.CHARIOX_TOTP_PLAYWRIGHT ?? '/w/valenv/tools/node_modules/playwright-core/index.mjs')
  // This is only the test viewer. The source browser above retains its sandbox.
  browserServer = await chromium.launchServer({ headless: true, executablePath: '/opt/google/chrome/chrome', args: ['--no-sandbox', '--disable-dev-shm-usage'] })
  viewer = trackOwnedProcess(browserServer.process())
  results.viewerPid = viewer.pid
  browser = await chromium.connect(browserServer.wsEndpoint())
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
    await sleep(Math.max(5000, (login.interval_seconds ?? 5) * 1000))
    const result = variant(await client.send({ PollCloudRelayLogin: { api_url: origin, device_code: login.device_code } }), 'CloudRelayLoginPolled').result
    if (result.status === 'denied') throw Error('device enrollment denied')
    return result.status === 'approved'
  }, 'own kernel enrollment', 120_000)
  console.log('MP-08 / MP-09 / MP-10 / MP-11: hosted enrollment approved')
  const afterEnrollment = variant(await client.send({ ListProviderAccountProfiles: { provider: 'codex' } }), 'ProviderAccountProfilesListed').profiles
  results.postEnrollmentAccounts = afterEnrollment.map(entry => ({ provider: entry.provider, auth_state: entry.auth_state, is_default: entry.is_default }))
  const catalog = variant(await client.send({ GetProviderCatalog: { provider: 'codex', account_profiles: { codex: linked.profile_id } } }), 'ProviderCatalog').catalog
  results.providerCatalog = { connected: catalog.connected, providerCount: catalog.all?.length }
  await record()
  await page.goto(origin + '/terminal?ui=os', { waitUntil: 'domcontentloaded' })
  await page.locator('[data-feature-new="room"]').waitFor({ timeout: 60_000 })
  await page.locator('[data-feature-new="room"]').click()
  await page.locator('[data-new-room]').waitFor()
  async function chooseRow(row) {
    const selector = page.locator(`[data-new-room] [data-row="${row}"]`)
    if (!await selector.count()) return
    if ((await selector.innerText()).includes(name)) return
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
  async function chooseSetting(row, label) {
    await page.locator(`[data-new-room] [data-row="${row}"]`).click()
    const choice = page.locator('[data-new-room-choices] [role="menuitemradio"]').filter({ hasText: label }).first()
    await choice.click()
  }
  await until(() => page.locator('[data-new-room] [data-row="provider"]').innerText().then(text => /Codex/i.test(text)), 'new-UI provider catalog')
  await chooseSetting('model', /GPT-6-Luna/i)
  await chooseSetting('variant', /^Low$/i)
  results.agentCreation = 'terminal-os New Room form; Luna and low selected in UI'
  await page.locator('[data-new-room] [data-row="alias"]').click()
  await page.locator('[data-new-room] input[aria-label="Alias"]').fill(name)
  await page.keyboard.press('Enter')
  await page.locator('[data-new-room] button[type="submit"]').click()
  await page.locator('[data-prompt-input]').waitFor({ timeout: 60_000 })
  const session = await until(async () => variant(await client.send({ ListSessions: null }), 'SessionsListed').sessions.find(entry => entry.alias === name), 'new-UI Room')
  sessionId = session.id
  const agent = variant(await client.send({ GetSessionState: { session_id: sessionId } }), 'SessionState').session.agents[0]
  assert.ok(agent?.id, 'new-UI Room created its agent')
  results.agentId = agent.id
  await record()
  await page.locator(`[data-room-agent="${agent.id}"]`).click()
  await page.locator(`[data-room-pane="${agent.id}"]`).click()
  await until(() => page.locator(`[data-room-agent="${agent.id}"]`).getAttribute('aria-pressed').then(value => value === 'true'), 'select Luna agent')
  await screenshot('01-new-ui-room')
  console.log('MP-08 / MP-09 / MP-10 / MP-11: new-UI Room ready')
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
      else if (/Authorize sudo window|Run one sudo task/.test(title)) value = passphrase
      else if (title === 'Agent needs your action') throw Error('agent requested owner action before completing the browser drill')
      else continue
      let popup = page.locator('[data-passkey-prompt]').first()
      if (/Run one sudo task|Authorize sudo window/.test(title)) await popup.waitFor({ timeout: 20_000 })
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
      console.log('MP-08 / MP-09 / MP-10 / MP-11: answered protected owner interaction ' + title)
      results.interactions.push({ title, kind: interaction.kind, answeredIn: 'terminal-os masked field' })
      await record()
      await sleep(300)
    }
  }
  const request = (id, injection, title) => JSON.stringify({ credential: { id, allowed_hosts: [new URL(site.origin).host], allowed_uses: ['browser'], injection }, prompt: { title, message: 'Enter the disposable local test-site value in the protected field.', timeout_sec: 180 }, overwrite: false })
  await submit('Use chariox.request_credential_secret with ' + request('bootstrap', { kind: 'browser' }, 'TOTP bootstrap') + '. This creates the disposable test Vault. Do not use shell/file tools or ask for the value in chat. After storage reply BOOTSTRAP_READY.')
  await until(async () => {
    await answerOwnerInput()
    const registered = variant(await client.send({ ListCredentials: null }), 'CredentialsListed').credentials.some(credential => credential.id === 'bootstrap')
    const working = await page.locator(`[data-room-agent="${agent.id}"]`).getAttribute('data-status') === 'working'
    return registered && !working
  }, 'protected Vault bootstrap', 240_000)
  console.log('MP-08 / MP-09 / MP-10 / MP-11: protected bootstrap completed')
  await screenshot('02-protected-bootstrap')
  await page.locator('[data-action="open-room-view"]').last().click()
  await page.locator('[data-open-room-view="browser"]').click()
  await page.locator('[data-room-browser]').waitFor({ timeout: 60_000 })
  await submit('/sudo Complete this local 2FA test using Chariox browser tools. First call chariox.request_credential_secret with ' + request('authenticator', { kind: 'totp' }, 'TOTP enrollment key') + '. Then load the kernel browser tools and navigate to ' + site.origin + '/enable. Find the observed Authentication code input and fill it only using chariox.kernel_browser_paste_secret credential_id=authenticator, submit=false. Take a browser snapshot and screenshot BEFORE clicking enable; verify the filled field is redacted. Click the observed enable button and verify Two-factor authentication enabled. Log out. Wait at least 31 seconds using a shell sleep (do not calculate or read any codes). Sign in by filling the observed Authentication code field with the same credential handle; click login and verify Signed in. Log out, wait at least another 31 seconds, then repeat sign in in another period. Never read/echo any key or code, never use page evaluation or direct HTTP to submit a code. Use fresh observed node_ref/document_id/generation for every fill. After both TOTP sign-ins succeeded, call chariox.request_credential_secret with ' + request('recovery', { kind: 'backup_codes' }, 'TOTP recovery codes') + '. Then log out and sign in once more by filling the observed Backup code field only with chariox.kernel_browser_paste_secret credential_id=recovery and clicking login. Keep tools moving until both TOTP sign-ins and the backup-code sign-in succeeded; report TOTP_TWO_PERIODS_DONE. Do not finish early.')
  console.log('MP-08 / MP-09 / MP-10 / MP-11: submitted sudo TOTP drill')
  await until(async () => { await answerOwnerInput(); const stats = site.stats(); if (JSON.stringify(results.site) !== JSON.stringify(stats)) { results.site = stats; await record() }; return stats.enrollmentSuccesses === 1 && stats.loginSuccesses >= 2 && stats.backupSuccesses === 1 }, 'agent enrollment and two-period sign-in', 480_000)
  await until(async () => {
    await answerOwnerInput()
    return (await page.locator(`[data-room-pane="${agent.id}"]`).innerText()).includes('TOTP_TWO_PERIODS_DONE')
  }, 'provider completion after two sign-ins', 120_000)
  await screenshot('03-two-period-sign-in')
  const outline = variant(await client.send({ GetSessionHistoryOutline: { session_id: sessionId, agent_ids: [agent.id], latest_prompt_count: 10, cursor: null } }), 'SessionHistoryOutline')
  const blobs = []
  for (const entry of outline.agents) {
    const ids = [...new Set(entry.turns.flatMap(turn => turn.blobs.map(blob => blob.blob_id)))]
    if (ids.length) blobs.push(...variant(await client.send({ GetSessionHistoryBlobContents: { session_id: sessionId, agent_id: entry.agent_id, blob_ids: ids } }), 'SessionHistoryBlobContents').blobs)
  }
  assert.ok(blobs.some(blob => blob.entries.length > 0), 'absence check requires actual public history entries')
  const publicHistory = JSON.stringify({ outline, blobs })
  const transcript = publicHistory + await texts(path.join(root, 'history'))
  const visible = await page.locator(`[data-room-pane="${agent.id}"]`).innerText()
  assert.ok(site.checkAbsence(transcript + visible + rawKernelLog), 'seed/code absence check failed')
  assert.ok(!transcript.includes(passphrase) && !transcript.includes(bootstrapSecret), 'protected input absence check failed')
  results.absencePassed = true
  results.publicHistoryEntries = blobs.reduce((sum, blob) => sum + blob.entries.length, 0)
  await writeFile(path.join(evidence, 'public-history.json'), publicHistory, { mode: 0o600 })
  results.site = site.stats()
  results.passed = true
  results.note = 'Real kernel/provider/new-UI local fixture drill; real-service AU-10 remains separate.'
  console.log('PASS MP-08 / MP-09 / MP-10 / MP-11 local TOTP drill, DPR' + dpr)
} catch (error) {
  results.passed = false
  results.failurePhase = results.phase
  if (site) results.site = site.stats()
  const message = String(error.message)
  const safe = !message.includes(passphrase) && !message.includes(bootstrapSecret) && (!site || site.checkAbsence(message))
  results.failure = safe ? message : 'private-value absence check failed'
  if (page) {
    if (site) {
      const visible = (await page.locator('.agent-pane').allInnerTexts().catch(() => [])).join('\n')
      const transcript = visible + await texts(path.join(root, 'history')) + rawKernelLog
      results.failureAbsencePassed = site.checkAbsence(transcript) && !transcript.includes(passphrase) && !transcript.includes(bootstrapSecret)
      if (results.failureAbsencePassed) await writeFile(path.join(evidence, 'failure-public-pane.txt'), visible, { mode: 0o600 })
    }
    await screenshot('failure').catch(() => {})
  }
  console.error('FAIL MP-08 / MP-09 / MP-10 / MP-11 local TOTP drill; see result.json')
  process.exitCode = 1
} finally {
  console.log('MP-08 / MP-09 / MP-10 / MP-11: cleaning owned drill processes')
  const handles = kernel ? ownedProcessHandles(kernel) : []
  const viewerHandles = viewer ? ownedProcessHandles(viewer) : []
  if (client && sessionId) await deadline(client.send({ EndSession: { session_id: sessionId } })).catch(() => {})
  if (client) await deadline(client.send({ DisconnectCloudRelay: null })).catch(() => {})
  if (client) await deadline(client.close()).catch(() => {})
  if (browser) await deadline(browser.close()).catch(() => {})
  if (browserServer) await deadline(browserServer.close()).catch(() => {})
  for (const handle of viewerHandles) { try { signalOwnedProcess(handle, 'SIGKILL') } catch {} }
  if (site) await deadline(site.close()).catch(() => {})
  if (kernel) {
    if (kernel.exitCode === null && kernel.signalCode === null) signalOwnedProcess(kernel, 'SIGTERM')
    await until(() => kernel.exitCode !== null || kernel.signalCode !== null, 'own kernel exit', 20_000).catch(() => {})
    for (const handle of handles) { try { signalOwnedProcess(handle, 'SIGKILL') } catch {} }
    results.cleanup = { kernelPid: kernel.pid, exitCode: kernel.exitCode, signal: kernel.signalCode }
  }
  await record()
  if (ownsRoot) await rm(root, { recursive: true, force: true })
  if (browserRoot) await rm(browserRoot, { recursive: true, force: true })
  if (ownsWorkspace) await rm(workspace, { recursive: true, force: true })
}
