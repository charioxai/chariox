// MP-08 / MP-09 / MP-10 / MP-11 AU-15: new hosted terminal-os shell,
// real kernel, hosted encrypted relay and official Codex against the local service.
import { randomBytes, createHash } from 'node:crypto'
import { readFile, writeFile, mkdir, rm, readdir } from 'node:fs/promises'
import { createWriteStream } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { fileURLToPath, pathToFileURL } from 'node:url'
import path from 'node:path'
import net from 'node:net'
import { spawnOwned, signalOwnedProcess } from '../../../apps/kernel/slice-linux-docker/owned-process-signals.mjs'
import { createAuthService } from './service.mjs'

const repository = fileURLToPath(new URL('../../../', import.meta.url))
const options = {}
for (let index = 2; index < process.argv.length; index += 2) options[process.argv[index].replace(/^--/, '')] = process.argv[index + 1]
for (const name of ['kernel-bin', 'playwright-module', 'home', 'evidence']) {
  if (!options[name] || !path.isAbsolute(options[name])) throw new Error(`--${name} requires an absolute path`)
}
const origin = new URL(options['cloud-origin']).origin
if (!origin.startsWith('https://')) throw new Error('--cloud-origin must use hosted HTTPS')
const root = options.home, evidence = options.evidence
const contains = (parent, child) => { const relative = path.relative(parent, child); return !relative || (!relative.startsWith('..') && !path.isAbsolute(relative)) }
if ([root, evidence].some(value => contains(repository, value)) || contains(root, evidence) || contains(evidence, root)) throw new Error('Use separate external state and evidence directories')
if (!process.env.CODEX_HOME?.includes('/provider-accounts/')) throw new Error('CODEX_HOME must name an existing Chariox-linked Codex profile')
const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')
const { chromium } = await import(pathToFileURL(options['playwright-module']).href)
const delay = ms => new Promise(resolve => setTimeout(resolve, ms))
const variant = (response, name) => { if (!response[name]) throw new Error(`Expected ${name}`); return response[name] }
async function freePort() {
  const server = net.createServer()
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const port = server.address().port
  await new Promise(resolve => server.close(resolve))
  return port
}
async function files(directory) {
  const result = []
  for (const entry of await readdir(directory, { withFileTypes: true }).catch(() => [])) {
    const name = path.join(directory, entry.name)
    if (entry.isDirectory()) result.push(...await files(name))
    else if (entry.isFile()) result.push(name)
  }
  return result
}
let stopping = false
for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => { stopping = true })
async function until(predicate, timeout = 240000) {
  const deadline = Date.now() + Number(options['timeout-ms'] ?? timeout)
  while (Date.now() < deadline) {
    if (stopping) throw new Error('Drill stopped by its owner')
    if (await predicate()) return
    await delay(250)
  }
  throw new Error('auth.md live drill timed out')
}
const passphrase = randomBytes(32).toString('base64url')
const ports = await Promise.all(Array.from({ length: 4 }, freePort))
const env = { PATH: process.env.PATH, LANG: 'C.UTF-8', SHELL: '/bin/bash', CODEX_HOME: process.env.CODEX_HOME,
  HOME: `${root}/os-home`, CHARIOX_HOME: `${root}/home`, TMPDIR: `${root}/tmp`, XDG_CONFIG_HOME: `${root}/config`, XDG_STATE_HOME: `${root}/state`, XDG_DATA_HOME: `${root}/data`, XDG_CACHE_HOME: `${root}/cache`,
  CHARIOX_DAEMON_SOCKET: `${root}/kernel.sock`, CHARIOX_KERNEL_PORT: String(ports[0]), CHARIOX_MCP_PORT: String(ports[1]), CHARIOX_CODEX_PORT: String(ports[2]), CHARIOX_OPENCODE_PORT: String(ports[3]),
  CHARIOX_SESSION_HISTORY_DIR: `${root}/history`, CHARIOX_LOG_DIR: `${root}/logs`, CHARIOX_DAEMON_ALIAS: 'authmd-live', NODE_OPTIONS: '--max-old-space-size=4096', RUST_MIN_STACK: '16777216',
}
const result = { MP: ['MP-08', 'MP-09', 'MP-10', 'MP-11'], status: 'FAIL', started: new Date().toISOString(),
  head: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: repository, encoding: 'utf8' }).trim(),
  kernel_sha256: createHash('sha256').update(await readFile(options['kernel-bin'])).digest('hex'),
  kernel_source_head: options['kernel-source-head'], web_source_head: options['web-source-head'],
  client: 'apps/web terminal-os hosted (real application entry)', cloud_origin: origin, scenarios: [],
}
let service, browser, page, client, sessionId, ownsRoot = false
function stage(value) { result.stage = value; console.log(JSON.stringify({ stage: value })) }
const children = [], logs = [], privateValues = [passphrase]
function absent(text) {
  service?.assertAbsent(text)
  if (privateValues.some(value => text.includes(value))) throw new Error('Protected runtime value appeared in evidence')
}
function start(binary, extraEnv, name) {
  const child = spawnOwned(binary, [], { cwd: repository, env: { ...env, ...extraEnv }, stdio: ['ignore', 'pipe', 'pipe'] })
  children.push(child)
  const log = createWriteStream(`${root}/${name}.private.log`, { mode: 0o600 })
  logs.push(`${root}/${name}.private.log`)
  child.stdout.pipe(log); child.stderr.pipe(log)
  return child
}
async function stop(child) {
  if (child.exitCode !== null || child.signalCode !== null) return
  signalOwnedProcess(child, 'SIGTERM')
  for (let count = 0; count < 60 && child.exitCode === null && child.signalCode === null; count++) await delay(250)
  if (child.exitCode === null && child.signalCode === null) {
    signalOwnedProcess(child, 'SIGKILL')
    await new Promise(resolve => child.once('exit', resolve))
  }
}
async function capture(name) {
  const fields = await page.locator('input:not([type="password"]), textarea').evaluateAll(elements => elements.map(element => element.value).join('\n'))
  absent((await page.locator('body').innerText()) + fields)
  await page.screenshot({ path: `${evidence}/${name}.png` })
}
const diagnostics = []
const claimed = new Set()
async function interact() {
  // Every answer goes through the actual new UI's controls.
  const passkey = page.locator('[data-passkey-prompt]')
  if (await passkey.count()) {
    const approve = passkey.locator('[data-passkey-popup-approve]')
    if (await approve.isEnabled()) {
      await passkey.locator('[data-passkey-popup-input]').fill(passphrase)
      await capture('owner-decision')
      await approve.click()
    }
    return
  }
  const secretDetail = page.locator('[data-interaction-detail]:has(input[type="password"])')
  if (await secretDetail.count()) {
    await secretDetail.locator('input[type="password"]').fill(passphrase)
    await secretDetail.locator('button.work-editor-save').click()
    return
  }
  const secretReply = page.locator('[data-interaction-id] [data-action="reply-interaction"]')
  if (await secretReply.count()) { await secretReply.first().click(); return }
  const claim = page.locator('.ap-ask[aria-label="Claim service account"], [data-interaction-id^="agent-auth-claim-"]')
  if (await claim.count()) {
    const message = await claim.first().innerText()
    const id = await claim.first().getAttribute('data-interaction-id') ?? message
    if (claimed.has(id)) return
    const link = message.match(/https?:\/\/\S+/)?.[0], code = message.match(/code (\d{6})/)?.[1]
    if (!link || !code) throw new Error('Claim card has no link or user code')
    await capture('claim-card')
    const owner = await browser.newPage()
    try {
      await owner.goto(link)
      await owner.getByLabel('Account email').fill('owner@example.test')
      await owner.getByLabel('Code').fill(code)
      await owner.getByRole('button', { name: 'Claim account' }).click()
      await owner.getByText('Account claimed.').waitFor()
      await owner.screenshot({ path: `${evidence}/service-claimed.png` })
    } finally { await owner.close() }
    claimed.add(id)
  }
}
try {
  await mkdir(path.dirname(root), { recursive: true })
  await mkdir(root, { mode: 0o700 }); ownsRoot = true
  await mkdir(evidence, { recursive: true, mode: 0o700 })
  for (const name of ['os-home', 'home', 'tmp', 'config', 'state', 'data', 'cache', 'logs']) await mkdir(`${root}/${name}`, { mode: 0o700 })
  await writeFile(`${root}/home/config.toml`, `version = 1\n[history.archive]\nmode = "disabled"\n[credential_vault]\nbackend = "chariox_encrypted"\npath = "${root}/home/vault.json"\nunlock_policy = "kernel_init"\nagent_management = "allow"\n`, { mode: 0o600 })
  stage('kernel startup')
  service = await createAuthService()
  start(options['kernel-bin'], {}, 'kernel')
  await writeFile(`${root}/started-pids.json`, JSON.stringify(children.map(child => child.pid)), { mode: 0o600 })
  const kernelUrl = `ws://127.0.0.1:${ports[0]}/kernel`
  await until(async () => {
    const next = new LocalIpcClient(kernelUrl, { timeoutMs: 2000, localAuthEnvironment: env })
    try { await next.send(requests.listSessionsRequest()); client = next; return true } catch { await next.close(); return false }
  }, 60000)
  browser = await chromium.launch({ headless: true, executablePath: options['chrome-bin'] ?? '/opt/google/chrome/chrome', args: ['--no-sandbox', '--disable-dev-shm-usage'] })
  page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, deviceScaleFactor: Number(options.dpr ?? 1) })
  page.on('console', async message => {
    if (message.type() !== 'error') return
    const parts = await Promise.all(message.args().map(handle => handle.evaluate(value => value instanceof Error ? value.message : String(value)).catch(() => 'Unreadable browser diagnostic')))
    const text = parts.join(' ')
    try { absent(text); diagnostics.push(text.slice(0, 2000)) } catch { diagnostics.push('Protected browser diagnostic suppressed') }
  })
  stage('hosted device enrollment')
  const relayStatus = variant(await client.send({ RelayStatus: null }), 'RelayStatus').status
  const login = variant(await client.send({ StartCloudRelayLogin: { api_url: origin, client_id: 'authmd-live', client_alias: 'Authmd drill', machine_id: relayStatus.machine_id, machine_alias: 'Authmd owned drill' } }), 'CloudRelayLoginStarted').login
  privateValues.push(login.device_code)
  if (new URL(login.verification_url).origin !== origin) throw new Error('Device approval left the owned hosted stack')
  await page.goto(login.verification_url)
  await capture('device-approval')
  const approve = page.getByRole('button', { name: /approve|allow|connect/i })
  await approve.waitFor({ timeout: 30000 })
  await approve.click()
  await capture('device-approved')
  let enrolled = false
  for (let attempt = 0; attempt < 30; attempt++) {
    if (stopping) throw new Error('Drill stopped by its owner')
    await delay(Math.max(5000, (login.interval_seconds ?? 5) * 1000))
    const poll = variant(await client.send({ PollCloudRelayLogin: { api_url: origin, device_code: login.device_code } }), 'CloudRelayLoginPolled').result
    if (poll.status === 'approved') { enrolled = true; break }
    if (poll.status !== 'pending' && poll.status !== 'authorization_pending') throw new Error(`Device enrollment: ${poll.status}`)
  }
  if (!enrolled) throw new Error('Hosted device enrollment timed out')
  stage('hosted relay connection')
  await client.send({ ConnectCloudRelay: null })
  await until(async () => {
    const status = variant(await client.send({ RelayStatus: null }), 'RelayStatus').status
    if (!status.connected) return false
    if (!status.relay_url.startsWith('wss://')) throw new Error('Hosted relay must use wss')
    return true
  }, 60000)
  stage('verify linked native provider precondition')
  await client.send({ RefreshProviderAccountProfile: { provider: 'codex', account_profile: 'default' } })
  // Administrative fixture setup uses public owner APIs; the scenario itself is driven through the new UI.
  const created = variant(await client.send(requests.createSessionRequest(repository, repository, 'authmd drill', { provider: 'codex', model: 'gpt-6-luna', effort: 'low', account_profile: 'default', execution_mode: 'build', permission_level: 'yolo' }, undefined, 'unrestricted')), 'SessionCreated')
  sessionId = created.session.id
  let session = variant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState').session
  if (!session.agents.length) {
    await client.send(requests.spawnAgentRequest(sessionId, 'codex', 'authmd', 'gpt-6-luna', repository, 'low', 'build', 'yolo', undefined, undefined, undefined, 'default'))
    session = variant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState').session
  }
  const agentId = session.agents[0].id
  stage('new UI room navigation')
  await page.goto(`${origin}/waiting-room?ui=os`)
  await page.locator('.terminal-os').waitFor()
  await page.locator(`[data-room-id="${sessionId}"] [data-action="open-room"]`).click({ timeout: 60000 })
  await page.locator('[data-prompt-input]').waitFor()
  await capture('new-ui-room')
  // The current Vault window cannot initiate first-time setup. Start that
  // existing owner operation as fixture provisioning; its secret answers still
  // go exclusively through the real new-UI interaction controls.
  stage('prepare configured Vault precondition')
  let managed = false, manageFailure
  const managing = client.send({ ManageCredentialVault: { session_id: sessionId, agent_id: agentId } })
    .then(() => { managed = true }, error => { manageFailure = error })
  await until(async () => { await interact(); if (manageFailure) throw manageFailure; return managed })
  await managing
  result.vault_precondition = 'Existing owner ManageCredentialVault API; all secret answers in new UI'
  for (const flow of ['anonymous', 'service_auth']) {
    stage(`provider ${flow}`)
    let sawProviderActivity = false
    const before = service.counts.accepted
    const args = { request_id: `live-${flow}`, resource_url: `${service.origin}/`, ...(flow === 'service_auth' ? { email: 'owner@example.test' } : {}) }
    const prompt = `/sudo MP-08 / MP-09 / MP-10 / MP-11 AU-15. Register this owner-authorized local test account. Use Chariox MCP tools only. Call chariox.agent_auth.register with exactly ${JSON.stringify(args)}. Then call chariox.http_request_with_credential with its returned credential_id, method GET and url ${service.origin}/api/check. Do not use shell, files or raw network tools. Never print a credential value. Report HTTP status and end with AU15_${flow}_DONE.`
    await page.locator('[data-prompt-input]').fill(prompt)
    await page.locator('[data-prompt-input]').press('Enter')
    await until(async () => {
      await interact()
      const refusal = diagnostics.find(message => message.includes('prompt-bar.send'))
      if (refusal) throw new Error(refusal)
      const snapshot = variant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState')
      absent(JSON.stringify(snapshot))
      const state = snapshot.session
      const promptState = state.prompt_states?.[agentId] ?? {}
      const active = promptState.active_prompt ?? (state.active_prompt?.target_agent_id === agentId ? state.active_prompt : null)
      const queued = promptState.queued_prompts ?? (state.queued_prompts ?? []).filter(prompt => prompt.target_agent_id === agentId)
      const agent = state.agents.find(agent => agent.id === agentId)
      const busy = active || queued.length || agent?.is_processing || agent?.state === 'Working'
      if (busy) sawProviderActivity = true
      if (sawProviderActivity && !busy && agent && service.counts.accepted === before) throw new Error('Provider finished without a credential-backed successful API call')
      return service.counts.accepted > before && !busy && agent
    })
    const text = await page.locator('body').innerText()
    absent(text)
    await capture(`${flow}-http-200`)
    result.scenarios.push({ flow, status: 'PASS', http_status: 200 })
  }
  const listed = variant(await client.send(requests.listCredentialsRequest()), 'CredentialsListed').credentials
  if (listed.filter(value => value.metadata?.created_by_kind === 'agent_auth').length !== 2) throw new Error('Expected two auth.md Vault handles')
  absent(JSON.stringify(listed))
  result.status = 'PASS'
} catch (error) {
  // Framework errors may quote a protected field argument. Only a safe classification leaves the drill.
  const message = String(error?.message ?? error)
  try { absent(message); result.failure = message } catch { result.failure = 'Protected value in failure diagnostics' }
  if (page) await capture('failure').catch(() => {})
} finally {
  if (browser) await browser.close()
  let checked = 0
  try {
    const historyFiles = (await files(root)).filter(file => /\/(?:history|sessions|session-history)\//.test(file) || file.endsWith('.jsonl'))
    for (const file of [...new Set([...historyFiles, ...logs])]) { absent(await readFile(file, 'utf8')); checked++ }
    result.history_files_checked = historyFiles.length
    result.absence = { checked_files: checked, session_state_checked: Boolean(sessionId), service_values_absent: true }
  } catch { result.status = 'FAIL'; result.failure = 'Protected value found in kernel/provider history or logs' }
  if (client) { if (sessionId) await client.send(requests.endSessionRequest(sessionId)).catch(() => {}); await client.close() }
  for (const child of children.reverse()) await stop(child)
  try { for (const file of logs) absent(await readFile(file, 'utf8')) } catch { result.status = 'FAIL'; result.failure = 'Protected value found in shutdown logs' }
  result.counts = service?.counts
  result.browser_diagnostics = diagnostics
  if (service) await service.close()
  result.finished = new Date().toISOString()
  if (ownsRoot) { await writeFile(`${evidence}/result.json`, JSON.stringify(result, null, 2) + '\n', { mode: 0o600 }); await rm(root, { recursive: true, force: true }) }
}
console.log(JSON.stringify({ MP: result.MP, status: result.status, evidence: `${evidence}/result.json` }))
if (result.status !== 'PASS') process.exitCode = 1
