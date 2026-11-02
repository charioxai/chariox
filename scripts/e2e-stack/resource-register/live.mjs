// MP-08 / MP-09 / MP-10 / MP-11 AU-11: production terminal-os, real relay/kernel/providers.
// Local development proof only. Real-service AU-11 is NOT RUN until owner accounts exist.
import assert from 'node:assert/strict'
import { mkdir, readFile, writeFile, readdir } from 'node:fs/promises'
import path from 'node:path'
import { createHash } from 'node:crypto'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { setTimeout as delay } from 'node:timers/promises'
import { Fixture, until } from '../room-exec/fixture.mjs'
import { startKeyService } from './service.mjs'
import * as req from '../../../packages/kernel-client/dist/ipc-requests.js'

const options = new Map()
for (let i = 2; i < process.argv.length; i += 2) options.set(process.argv[i], process.argv[i + 1])
const required = name => { const value = options.get(name); if (!value) throw new Error(`missing ${name}`); return path.resolve(value) }
const home = required('--home'), evidence = required('--evidence'), cloud = required('--cloud')
const kernelBinary = required('--kernel-bin'), relayBinary = required('--relay-bin')
const codexScope = options.has('--codex-scope') ? required('--codex-scope') : null
const stack = options.get('--stack')
if (!stack || !/^[a-z0-9-]+$/.test(stack)) throw new Error('set --stack to your owned valenv stack name')
const expectedRed = options.get('--expected-red') === 'yes'
const providers = (options.get('--providers') ?? 'codex,claude-p').split(',')
if (providers.some(provider => !['codex','claude-p'].includes(provider))) throw new Error('validation providers are codex and claude-p')
if (providers.includes('codex') && !codexScope) throw new Error('set --codex-scope for Codex')
const { chromium } = await import(pathToFileURL(path.join(cloud, 'node_modules/playwright-core/index.mjs')))
const fixture = new Fixture({ home, evidence, kernelBinary, relayBinary })
let browser, page, service, context
let currentConsoleErrors = []
const privateValues = []
const report = { mp: ['MP-08','MP-09','MP-10','MP-11'], area: 'apps-autonomy', row: 'AU-11',
  scope: 'local disposable service; real-service acceptance NOT RUN', client: 'production Cloud entry -> terminal-os (?ui=os), hosted WSS',
  cells: [], owner_interactions: [], failures: [] }
const safe = text => { for (const value of privateValues) if (value && text.includes(value)) throw new Error('private value in prospective evidence'); return text }
const save = async (name, value) => writeFile(path.join(evidence, name), safe(JSON.stringify(value, null, 2)), { mode: 0o600 })

async function files(root) {
  const result = []
  for (const entry of await readdir(root, { withFileTypes: true }).catch(() => [])) {
    const file = path.join(root, entry.name)
    if (entry.isDirectory()) result.push(...await files(file))
    else if (entry.isFile()) result.push(file)
  }
  return result
}
async function history(session, agent) {
  const outline = (await fixture.A.send({ GetSessionHistoryOutline: { session_id: session,
    agent_ids: [agent], latest_prompt_count: 5, include_prefetched_content: true } })).SessionHistoryOutline
  const entries = [], ids = new Set()
  for (const owner of outline.agents) for (const turn of owner.turns) {
    entries.push(turn.user_prompt.entry, ...turn.entries.map(item => item.entry))
    if (turn.summary) entries.push(turn.summary.entry)
    for (const blob of turn.blobs) ids.add(blob.blob_id)
  }
  if (ids.size) {
    const response = (await fixture.A.send({ GetSessionHistoryBlobContents: { session_id: session,
      agent_id: agent, blob_ids: [...ids] } })).SessionHistoryBlobContents
    for (const blob of response.blobs) entries.push(...blob.entries.map(item => item.entry))
  }
  return entries.sort((a,b) => a.timestamp_ms-b.timestamp_ms)
}
function completedTools(entries) {
  const merged = new Map()
  for (const entry of entries) {
    if (entry.kind !== 'provider_tool') continue
    let update
    try { update = JSON.parse(entry.text) } catch { continue }
    const key = entry.merge_key ?? update.id, previous = merged.get(key) ?? {}
    merged.set(key, { ...previous, ...update, tool: update.tool === 'tool_result' ? previous.tool : update.tool ?? previous.tool,
      input: update.input ?? previous.input })
  }
  return [...merged.values()].filter(item => item.status === 'completed')
}
function objects(value, result = []) {
  if (typeof value === 'string') { try { objects(JSON.parse(value), result) } catch {} }
  else if (value && typeof value === 'object') { result.push(value); for (const item of Object.values(value)) objects(item, result) }
  return result
}
async function approve() {
  const passkey = page.locator('[data-passkey-popup-input]')
  if (await passkey.count() && await passkey.isVisible() && await passkey.isEnabled()) {
    await passkey.fill(fixture.passphrase)
    await page.locator('[data-passkey-popup-approve]').click()
    report.owner_interactions.push({ kind: 'sudo', client: 'terminal-os', at: new Date().toISOString() })
  }
}
async function storeKey(key, handle) {
  await fixture.A.send({ UpsertCredential: { credential: { id: handle, description: 'Disposable AU-11 development key',
    source: { type: 'vault', key: handle }, allowed_hosts: [new URL(service.origin).host], allowed_uses: ['http'],
    injection: { kind: 'header', name: 'Authorization', value: 'Bearer ${secret}' } } } })
  await fixture.A.send({ SetCredentialSecret: { session_id: null, agent_id: null, key: handle, value: key.value } })
}
async function openRoom(alias) {
  await page.locator('[data-action="open-room"]').filter({ hasText: alias }).first().click()
  await page.locator('[data-prompt-input][placeholder^="Message"]').waitFor({ timeout: 30000 })
}
async function prompt(text) {
  await page.locator('[data-prompt-input]').fill(text)
  await page.getByRole('button', { name: 'Send prompt', exact: true }).click()
  await until(async () => {
    const failure = currentConsoleErrors.find(message => message.startsWith('[terminal-os] prompt'))
    if (failure) throw new Error(`new-UI send failed: ${failure}`)
    await approve()
    if (await page.locator('[data-prompt-input]').inputValue() === '') return true
    await delay(200)
    return false
  }, 'new-UI send acknowledgement', 120000)
}

try {
  await mkdir(path.dirname(home), { recursive: true })
  await mkdir(evidence, { recursive: true })
  await fixture.start()
  console.log('MP-08 / MP-09 / MP-10 / MP-11: disposable kernels and relay ready')
  privateValues.push(fixture.secret, fixture.passphrase)
  service = await startKeyService()
  const stackRoot = path.join('/w/valenv/prs', stack)
  const metadata = JSON.parse(await readFile(path.join(stackRoot, 'metadata.json'), 'utf8'))
  const origin = `https://${metadata.hostname}`
  const envText = await readFile(path.join(stackRoot, 'config/cloud.env'), 'utf8')
  const devSecret = envText.split('\n').find(line => line.startsWith('CHARIOX_CLOUD_DEV_AUTH_SECRET='))?.split('=').slice(1).join('=').trim()
  if (!devSecret) throw new Error('owned stack has no dev-auth input')
  privateValues.push(devSecret)
  report.stack = stack; report.origin = origin; report.cloud_revision = metadata.cloud_revision ?? metadata.commit
  browser = await chromium.launch({ executablePath: '/opt/google/chrome/chrome', headless: true, args: ['--no-sandbox','--disable-dev-shm-usage'] })
  async function login(ctx) {
    const csrf = await (await ctx.request.get(origin + '/auth/csrf')).json()
    const browserLogin = await ctx.request.post(origin + '/auth/dev/browser-login', {
      headers: { 'x-chariox-dev-auth-secret': devSecret }, data: { email: metadata.fixture_email,
        displayName: 'AU-11 Private Validation', providerSubject: 'dev|' + metadata.fixture_email } })
    if (!browserLogin.ok()) throw new Error('fixture browser login failed: HTTP ' + browserLogin.status())
    const auth = await ctx.request.post(origin + '/auth/cloud-session', {
      headers: { 'csrf-token': csrf.csrfToken }, data: { accountSlug: stack, accountName: stack } })
    if (!auth.ok()) throw new Error('fixture Cloud session failed: HTTP ' + auth.status())
    const token = (await auth.json()).cloudSessionToken
    privateValues.push(token)
    await ctx.addInitScript(value => sessionStorage.setItem('chariox-browser-cloud-session', value), token)
  }
  const enrollment = await browser.newContext()
  try {
    await login(enrollment)
    const status = (await fixture.A.send({ RelayStatus: null })).RelayStatus.status
    const started = (await fixture.A.send({ StartCloudRelayLogin: { api_url: origin,
      client_id: `vreg-${Date.now()}`, client_alias: 'AU-11 development', machine_id: status.machine_id,
      machine_alias: `vreg-au11-${Date.now()}` } })).CloudRelayLoginStarted.login
    const approved = await enrollment.request.post(origin + '/auth/dev/device/approve', {
      headers: { 'x-chariox-dev-auth-secret': devSecret }, data: { userCode: started.user_code,
        email: metadata.fixture_email, accountSlug: stack, accountName: stack,
        displayName: 'AU-11 Private Validation', providerSubject: 'dev|' + metadata.fixture_email } })
    if (!approved.ok()) throw new Error('fixture device enrollment failed: HTTP ' + approved.status())
    await until(async () => {
      const result = (await fixture.A.send({ PollCloudRelayLogin: { api_url: origin, device_code: started.device_code } })).CloudRelayLoginPolled.result
      if (result.status === 'denied') throw new Error('owned kernel enrollment denied')
      if (result.status === 'approved') return true
      await delay(5000); return false
    }, 'owned kernel Cloud enrollment', 120000)
    await fixture.A.send({ ConnectCloudRelay: null })
    await until(async () => (await fixture.A.send({ RelayStatus: null })).RelayStatus.status.connected, 'hosted WSS connection', 60000)
    report.relay_url = (await fixture.A.send({ RelayStatus: null })).RelayStatus.status.relay_url
    assert.ok(report.relay_url.startsWith('wss://'), 'client uses the hosted relay')
  } finally { await enrollment.close() }
  console.log('MP-08 / MP-09 / MP-10 / MP-11: kernel enrolled on owned hosted WSS stack')
  let codex, claude
  if (providers.includes('codex')) {
    codex = (await fixture.A.send({ LinkProviderAccountProfile: { provider: 'codex', label: 'AU-11 Codex', path: codexScope } })).ProviderAccountProfile.profile.profile_id
    const verified = (await fixture.A.send({ RefreshProviderAccountProfile: { provider: 'codex', account_profile: codex } })).ProviderAccountProfile.profile
    if (verified.auth_state === 'Unauthorized') throw new Error('Codex provider 401/unauthorized during product profile verification; report to coordinator')
  }
  if (providers.includes('claude-p')) {
    claude = (await fixture.A.send({ CreateProviderAccountProfile: { provider: 'claude', label: 'AU-11 Claude' } })).ProviderAccountProfile.profile.profile_id
    let input = ''
    process.stdin.setEncoding('utf8')
    for await (const chunk of process.stdin) { input += chunk; if (input.length > 4096) throw new Error('setup-token input exceeds 4096 characters') }
    const setupToken = input.trim()
    privateValues.push(setupToken)
    if (!setupToken.startsWith('sk-ant-oat01-')) throw new Error('feed the coordinator setup token on stdin')
    await fixture.A.send({ SetProviderAccountCredential: { session_id: null, agent_id: null, provider: 'claude', account_profile: claude, value: setupToken, run: false, overwrite: false } })
    const verified = (await fixture.A.send({ RefreshProviderAccountProfile: { provider: 'claude', account_profile: claude } })).ProviderAccountProfile.profile
    if (verified.auth_state === 'Unauthorized') throw new Error('Claude provider 401/unauthorized during product profile verification; report to coordinator')
  }
  report.drill_sha256 = createHash('sha256').update(await readFile(fileURLToPath(import.meta.url))).digest('hex')
  const webAsset = (await readdir(path.join(stackRoot, 'web-dist/assets'))).find(name => /^terminal-os\..*\.js$/.test(name))
  if (!webAsset) throw new Error('owned stack has no production terminal-os asset')
  report.web_asset = webAsset
  report.web_sha256 = createHash('sha256').update(await readFile(path.join(stackRoot, 'web-dist/assets', webAsset))).digest('hex')
  report.kernel_sha256 = createHash('sha256').update(await readFile(kernelBinary)).digest('hex')
  let sequence = 0
  for (const provider of providers) for (const dpr of (expectedRed ? [1] : [1,2])) {
    ++sequence
    const model = provider === 'codex' ? 'gpt-6-luna' : 'claude-haiku-5-5'
    const profile = provider === 'codex' ? codex : claude
    const alias = `vreg-${provider}-${Date.now()}`
    const work = path.join(home, 'A/work')
    const made = (await fixture.A.send(req.createSessionRequest(work, work, alias, {
      provider, account_profile: profile, model, effort: 'low', execution_mode: 'build', permission_level: 'yolo',
    }))).SessionCreated
    const session = made.session.id, agent = made.agent.id
    const key = service.issue(), rotated = service.issue(), handle = `vreg-service-${sequence}`
    privateValues.push(key.value, rotated.value)
    await storeKey(key, handle)
    const rotatedHandle = `${handle}-rotated`
    await storeKey(rotated, rotatedHandle)
    const launch = await fixture.A.send(req.launchProviderRunRequest(session, provider, profile, model, 'low', agent))
    const run = (launch.ProviderRunLaunched ?? launch.ProviderRunLaunchAccepted).provider_run
    await until(async () => {
      const current = (await fixture.A.send({ GetProviderRun: { provider_run_id: run.id } })).ProviderRun.provider_run
      if (String(current.state).toLowerCase() === 'ended') throw new Error('provider ended before first prompt')
      return ['running','ready','waiting','idle'].includes(String(current.state).toLowerCase())
    }, 'provider startup', 120000)
    context = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: dpr,
      recordVideo: { dir: path.join(evidence, 'video'), size: { width: 1440, height: 900 } } })
    await login(context)
    page = await context.newPage()
    const consoleErrors = currentConsoleErrors = []
    page.on('pageerror', error => consoleErrors.push(String(error)))
    page.on('console', event => { if (event.type() === 'error') consoleErrors.push(event.text()) })
    await page.goto(origin + '/waiting-room?ui=os')
    await page.locator('.terminal-os').waitFor({ timeout: 60000 })
    assert.equal(await page.locator('html').getAttribute('data-terminal-ui'), 'os')
    await openRoom(alias)
    console.log(`MP-08 / MP-09 / MP-10 / MP-11: ${provider} DPR${dpr} new-UI Room open`)
    await page.screenshot({ path: path.join(evidence, `${provider}-dpr${dpr}-room.png`) })
    const marker = `VREG_DONE_${sequence}`
    const task = [
      '/sudo Validate the created-resource register using this authorized disposable local test service. Use Chariox tools only; no shell, files, or other services. Never read or print a secret.',
      `1. Call chariox.vault.generate with request_id "${alias}-password" and origin "${service.origin}". Keep only its handle.`,
      `2. Call http_request_with_credential with credential_id "${handle}", method POST, url "${service.origin}/resources" to create a deploy-key resource.`,
      `3. Call chariox.mandate.record_resource with request_id "${alias}-key", service "${service.origin}", kind "api_key", name "deploy key", credential_id "${handle}", revoke {kind:"http",request:{credential_id:"${handle}",method:"DELETE",url:"${service.origin}/keys/${key.id}"}}. Use its returned resource id.`,
      '4. Call chariox.mandate.list_resources. Verify both the automatic password row and your deploy-key row exist.',
      '5. Call chariox.mandate.revoke_resource for your deploy-key row, then list again and verify its state is revoked.',
      `6. Call http_request_with_credential with credential_id "${rotatedHandle}" and url "${service.origin}/check" to prove the rotated key works.`,
      `Then reply ${marker} followed by the generated credential handle and the resource id. If a tool is unavailable, reply VREG_MISSING_TOOL with its name and stop.`,
    ].join('\n')
    const began = Date.now()
    await prompt(task)
    console.log(`MP-08 / MP-09 / MP-10 / MP-11: ${provider} DPR${dpr} prompt submitted`)
    let sawRunning = false
    await until(async () => {
      await approve()
      const state = (await fixture.A.send({ GetSessionState: { session_id: session } })).SessionState
      const row = state.session.agents.find(item => item.id === agent)
      const active = state.session.prompt_states?.[agent]?.active_prompt ?? state.session.active_prompt
      if (active || row?.is_processing) sawRunning = true
      const entries = await history(session, agent)
      const assistant = entries.filter(entry => entry.kind === 'provider_output').map(entry => entry.text).join('\n')
      const idle = !active && !row?.is_processing
      if (idle && assistant.includes(marker) && service.events.some(event => event.key_id === key.id && event.method === 'DELETE' && event.status === 200)) return true
      if (idle && expectedRed && assistant.includes('VREG_MISSING_TOOL')) return true
      if (entries.some(entry => entry.kind === 'provider_error')) throw new Error('provider reported an error; inspect private-safe public history')
      if (Date.now() - began > 600000) throw new Error('provider did not complete register workflow')
      await delay(1000)
      return false
    }, `${provider} register workflow`, 610000)
    await delay(1500)
    const before = await service.check(key), after = await service.check(rotated)
    const events = service.events.filter(event => [key.id, rotated.id].includes(event.key_id))
    const entries = await history(session, agent)
    assert.ok(entries.length > 0, 'public history must exist for the absence check')
    const publicText = safe(JSON.stringify(entries))
    const tools = completedTools(entries)
    if (!expectedRed) {
      for (const suffix of ['vault_generate','mandate_record_resource','mandate_list_resources','mandate_revoke_resource'])
        assert.ok(tools.some(tool => String(tool.tool).replaceAll('.', '_').endsWith(suffix)), `missing completed ${suffix}`)
      const values = objects(tools)
      assert.ok(values.some(value => value.kind === 'password' && value.credential_id), 'automatic generated-password row must be listed')
      assert.ok(values.some(value => value.kind === 'api_key' && value.state === 'revoked'), 'confirmed revoked row must be listed')
    }
    const revoked = events.some(event => event.method === 'DELETE' && event.status === 200)
    const status = expectedRed ? (!revoked ? 'EXPECTED RED' : 'UNEXPECTED PASS') : (revoked && before === 401 && after === 200 ? 'PASS' : 'FAIL')
    await page.screenshot({ path: path.join(evidence, `${provider}-dpr${dpr}-result.png`) })
    await save(`${provider}-dpr${dpr}-history.json`, entries)
    await save(`${provider}-dpr${dpr}-console.json`, consoleErrors)
    const executedRunIds = [...new Set(entries.filter(entry => ['provider_tool','provider_output'].includes(entry.kind) && entry.provider_run_id).map(entry => entry.provider_run_id))]
    assert.ok(executedRunIds.length > 0, 'executed provider run must be recorded')
    const resolvedRuns = []
    for (const id of executedRunIds) {
      const actual = (await fixture.A.send({ GetProviderRun: { provider_run_id: id } })).ProviderRun.provider_run
      assert.equal(actual.model, model, 'resolved provider model must match the validation model')
      resolvedRuns.push({ id, provider: actual.provider, model: actual.model, state: actual.state })
    }
    report.cells.push({ provider, model, dpr, status, turns: 1, elapsed_ms: Date.now() - began, revoked_key_status: before,
      rotated_key_status: after, events, history_entries: entries.length, history_secret_absence: !!publicText, completed_tools: tools.map(tool => tool.tool),
      saw_running: sawRunning, resolved_runs: resolvedRuns })
    await fixture.A.send({ EndSession: { session_id: session } })
    const recording = page.video()
    await context.close(); context = null
    report.cells.at(-1).video = path.relative(evidence, await recording.path())
    if (status !== 'PASS' && status !== 'EXPECTED RED') throw new Error('register workflow assertion failed')
  }
  for (const directory of ['A/history','A/logs','B/logs']) for (const filename of await files(path.join(home, directory))) safe((await readFile(filename)).toString('utf8'))
  report.private_absence_check = 'PASS: supplied service keys and passkey absent from public history and kernel logs'
  report.service_events = service.events
} catch (error) {
  let message
  message = String(error.message)
  for (const value of privateValues) if (value) message = message.split(value).join('[private input]')
  message = safe(message)
  report.failures.push(message)
  await save('failure-console.json', currentConsoleErrors).catch(() => undefined)
  process.exitCode = 1
  if (page) {
    for (const selector of ['#pairing-link','#relay-client-token']) if (await page.locator(selector).count()) await page.locator(selector).fill('').catch(() => undefined)
    await page.screenshot({ path: path.join(evidence, 'failure.png') }).catch(() => undefined)
  }
} finally {
  await context?.close().catch(() => undefined)
  await browser?.close().catch(() => undefined)
  await service?.close()
  await save('result.json', report)
  await fixture.A?.send(req.teardownProviderProcessesRequest()).catch(() => undefined)
  await fixture.cleanup()
}
console.log(JSON.stringify({ cells: report.cells.map(({provider,status})=>({provider,status})), failures: report.failures }))
