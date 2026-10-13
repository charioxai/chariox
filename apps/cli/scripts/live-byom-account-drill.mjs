#!/usr/bin/env node
// MP-08 / MP-10 / MP-11 BY-03 findings: real hosted Cloud + relay (WSS), built Web
// in Chrome DPR1/2 through the shaped 8 Mbit/s / >=60 ms network, real kernels
// enrolled through Add this machine, and the real TUI. Cells:
//  signout  - two tabs of one sign-in; Sign out in one ends the other's Cloud
//             session (API 401) and the relay closes its WSS.
//  control  - visible Sign out in My kernels and the waiting room.
//  shallow  - Copy of a shallow clone shows a specific, actionable refusal.
//  names    - two kernels of one host stay distinguishable where targets are picked.
// Capabilities and cookies stay in process memory; only statuses are recorded.
import assert from 'node:assert/strict'
import {createServer} from 'node:http'
import net from 'node:net'
import path from 'node:path'
import {pathToFileURL, fileURLToPath} from 'node:url'
import {mkdir, mkdtemp, open, readFile, writeFile, rm, statfs} from 'node:fs/promises'
import {LocalIpcClient} from '../../../packages/kernel-client/dist/ipc.js'
import {spawnOwned, signalOwnedProcessGroup, ownedProcessHandles, signalOwnedProcess} from '../../kernel/slice-linux-docker/owned-process-signals.mjs'

const required = key => { assert(process.env[key], `${key} required`); return process.env[key] }
const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const E = required('BYOM_EVIDENCE'), state = required('BYOM_STATE_ROOT'), work = required('BYOM_WORK_ROOT')
const origin = required('BYOM_ORIGIN'), relay = required('BYOM_RELAY_URL'), binary = required('BYOM_KERNEL_BINARY'), cli = required('BYOM_CLI_BINARY')
const {chromium} = await import(pathToFileURL(required('PLAYWRIGHT_MODULE')))
const {startHostNetwork} = await import(pathToFileURL(path.join(required('BYOM_CLOUD_ROOT'), 'scripts/lib/project-environment-host-network.mjs')))
// The stack's dev-login secret and fixture identity are read, never printed.
const stackEnv = Object.fromEntries((await readFile(required('BYOM_STACK_CLOUD_ENV'), 'utf8')).split('\n').filter(line => line.includes('=')).map(line => [line.slice(0, line.indexOf('=')), line.slice(line.indexOf('=') + 1)]))
const devSecret = stackEnv.CHARIOX_CLOUD_DEV_AUTH_SECRET, fixtureEmail = stackEnv.VALENV_FIXTURE_EMAIL
assert(devSecret && /^[a-z0-9-]+@chariox\.invalid$/.test(fixtureEmail))
assert.equal(new URL(origin).protocol, 'https:'); assert.equal(new URL(relay).protocol, 'wss:')
for (const dir of [E, state, work]) { assert(path.isAbsolute(dir) && !dir.startsWith(repo + '/')); await mkdir(dir, {recursive: true}) }
const scratch = await mkdtemp(path.join(state, 'byom-account-')), workspace = await mkdtemp(path.join(work, 'public-project-'))
const children = [], confidential = [devSecret], cells = [], resources = []
let browser, network, proxy, contextId, socket, tuiLog, source, target
const report = {mpItems: ['MP-08', 'MP-10', 'MP-11'], area: 'apps-byom/BY-03', phase: required('BYOM_PHASE'), source: {oss: required('BYOM_OSS_HEAD'), cloud: required('BYOM_CLOUD_HEAD'), kernel: binary}, started: new Date().toISOString(), cells, resources,
  scope: 'Real hosted Cloud/relay/WSS with the product dev login (one sign-in = one identity session); built Web DPR1/2; real kernels/TUI. Real WorkOS sign-in and signed-installer acceptance are not established here.'}
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
const safe = text => confidential.reduce((value, secret) => value.replaceAll(secret, '[redacted]'), String(text))
const save = () => writeFile(path.join(E, 'RESULT.json'), JSON.stringify(report, null, 2))
const shot = async (page, name) => { let hidden; try { hidden = await page.addStyleTag({content: 'input, textarea, [data-sensitive] { visibility: hidden !important; }'}); await page.screenshot({path: path.join(E, name + '.png'), scale: 'device', timeout: 60000, mask: [page.locator('input, textarea, [data-sensitive]')]}) } catch (error) { (report.screenshotErrors ??= []).push({name, error: safe(error.message).split('\n')[0]}) } finally { await hidden?.evaluate(node => node.remove()).catch(() => {}) } }
async function sample() {
  const mem = Number((await readFile('/proc/meminfo', 'utf8')).match(/MemAvailable:\s+(\d+)/)[1]) * 1024
  const fs = await statfs('/'); resources.push({at: new Date().toISOString(), memAvailable: mem, diskFree: fs.bavail * fs.bsize})
  assert(mem >= 12 * 1024 ** 3 && fs.bavail * fs.bsize >= 15 * 1024 ** 3, 'MP-10 resource floor')
}
async function until(fn, label, timeout = 60000) {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) { const value = await fn(); if (value) return value; await pause(150) }
  throw Error(`${label} timed out`)
}
function spawn(command, args, env, stdio = ['pipe', 'pipe', 'pipe']) {
  const child = spawnOwned(command, args, {cwd: repo, env, stdio, detached: true, retainLeader: true}); children.push(child)
  const processRecord = {pid: child.pid ?? null, command: path.basename(command)}; (report.processes ??= []).push(processRecord)
  child.once('close', (code, signal) => Object.assign(processRecord, {exitCode: code, signal}))
  child.done = new Promise((resolve, reject) => { child.once('error', reject); child.once('close', (code, signal) => resolve({code, signal})) })
  return child
}
async function run(command, args, env = process.env, input = '') {
  const child = spawn(command, args, env); let out = '', err = ''
  child.stdout.on('data', bytes => out += bytes); child.stderr.on('data', bytes => err += bytes); child.stdin.end(input)
  const {code} = await child.done; assert.equal(code, 0, `${command} exit ${code}: ${safe(err).slice(-300)}`); return out
}
async function stop(child) {
  if (child.exitCode !== null) return
  const handles = ownedProcessHandles(child)
  ;(report.stoppedProcessIdentities ??= []).push(...handles.map(({pid, start}) => ({pid, start})))
  for (const handle of handles.filter(value => value.pid !== child.pid)) signalOwnedProcess(handle, 'SIGTERM')
  signalOwnedProcessGroup(child, 'SIGTERM'); await Promise.race([child.done, pause(5000)])
  if (child.exitCode === null) { signalOwnedProcessGroup(child, 'SIGKILL'); await child.done }
}
async function port() { const s = net.createServer(); await new Promise(r => s.listen(0, '127.0.0.1', r)); const p = s.address().port; await new Promise(r => s.close(r)); return p }
async function rpc(kernel, request) {
  const old = process.env.CHARIOX_HOME; process.env.CHARIOX_HOME = kernel.env.CHARIOX_HOME; const client = new LocalIpcClient(kernel.url)
  try { const response = await client.send(request); if (response.Error) throw Error(safe(response.Error.message ?? 'Kernel refused request')); return response }
  finally { await client.close(); if (old === undefined) delete process.env.CHARIOX_HOME; else process.env.CHARIOX_HOME = old }
}
async function automation(request) {
  return new Promise((resolve, reject) => { const s = net.createConnection(socket); let text = ''; s.setTimeout(15000, () => s.destroy(Error('TUI automation timeout'))); s.on('error', reject); s.on('connect', () => s.write(JSON.stringify({id: 1, ...request}) + '\n')); s.on('data', b => { text += b; const n = text.indexOf('\n'); if (n < 0) return; s.end(); const reply = JSON.parse(text.slice(0, n)); reply.ok ? resolve(reply.data) : reject(Error('TUI action refused')) }) })
}
// Pass-through observer for the source kernel's Cloud API: it records only the
// Copy context id, never changes requests or responses.
async function apiProxy() {
  const server = createServer(async (req, res) => {
    let body = ''; for await (const chunk of req) body += chunk
    if (req.url === '/v1/managed-kernels/context/ticket' && body) contextId = JSON.parse(body).ownerManaged?.contextId ?? contextId
    const headers = {}; for (const key of ['content-type', 'x-chariox-kernel-credential']) if (req.headers[key]) headers[key] = req.headers[key]
    const response = await fetch(origin + req.url, {method: req.method, headers, body: ['GET', 'HEAD'].includes(req.method) ? undefined : body, redirect: 'error'})
    res.writeHead(response.status, {'content-type': response.headers.get('content-type') ?? 'application/json'}); res.end(await response.text())
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve)); return {server, url: 'http://127.0.0.1:' + server.address().port}
}
async function signedInContext(dpr) {
  const context = await browser.newContext({deviceScaleFactor: dpr, viewport: {width: 1440, height: 1000}})
  const login = await context.request.post(origin + '/auth/dev/browser-login', {headers: {'x-chariox-dev-auth-secret': devSecret}, data: {email: fixtureEmail}})
  assert.equal(login.status(), 200, 'product dev login')
  return context
}
async function newPage(context) {
  const page = await context.newPage(); network.attach(page)
  // Playwright WebSocket instances distinguish target, lane and reconnect
  // generation without asynchronously decoding credential-bearing console data.
  page.relayEvents = []
  const sockets = new Map()
  page.on('websocket', socket => {
    const url = new URL(socket.url())
    if (url.origin !== new URL(relay).origin) return
    const connectionId = sockets.size + 1
    const fields = {connectionId, relayOrigin: url.origin}
    sockets.set(connectionId, true)
    page.relayEvents.push({event: 'connected', at: Date.now(), ...fields})
    socket.on('close', () => {
      sockets.set(connectionId, false)
      page.relayEvents.push({event: 'closed', at: Date.now(), ...fields})
    })
  })
  page.openRelayConnections = () => [...sockets].filter(([, open]) => open).map(([id]) => id)
  page.on('pageerror', () => { report.pageErrors = (report.pageErrors ?? 0) + 1 })
  return page
}
async function enroll(page, name, apiUrl) {
  const home = path.join(workspace, name + '-home'); await mkdir(home, {mode: 0o700})
  // No alias override: both kernels take this host's default machine name, like two installs on one host.
  const env = {...process.env, HOME: home, CHARIOX_HOME: path.join(scratch, name), CHARIOX_KERNEL_PORT: String(await port()), CHARIOX_MCP_PORT: String(await port()), CHARIOX_CODEX_PORT: String(await port()), CHARIOX_OPENCODE_PORT: String(await port()), CHARIOX_LOG_DIR: path.join(scratch, name + '-logs'), TMPDIR: path.join(scratch, 'tmp'), CODEX_HOME: path.join(home, '.codex'), CLAUDE_CONFIG_DIR: path.join(home, '.claude'), XDG_CONFIG_HOME: path.join(home, '.config'), XDG_STATE_HOME: path.join(home, '.local/state'), XDG_DATA_HOME: path.join(home, '.local/share')}
  for (const key of ['CHARIOX_RELAY_URL', 'CHARIOX_RELAY_TOKEN', 'CHARIOX_CLOUD_RELAY_CONFIG_JSON', 'CHARIOX_CLOUD_RELAY_CONFIG_PATH', 'CHARIOX_KERNEL_LOCAL_AUTH_TOKEN', 'CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE', 'CHARIOX_DAEMON_ALIAS', 'CHARIOX_MACHINE_ALIAS']) delete env[key]
  await mkdir(env.TMPDIR, {recursive: true})
  await page.getByRole('button', {name: 'Add this machine', exact: true}).click(); const dialog = page.getByRole('dialog')
  const command = await dialog.getByLabel('Install and enroll command').inputValue(); const ticket = command.split('\n')[1]; confidential.push(ticket); assert(ticket)
  const identity = JSON.parse((await run(binary, ['--owner-managed-self-enroll-stdin'], env, JSON.stringify({ticket, apiUrl}))).trim())
  const kernel = {env, identity, url: 'ws://127.0.0.1:' + env.CHARIOX_KERNEL_PORT + '/kernel'}
  const log = await open(path.join(scratch, name + '.log'), 'w', 0o600); spawn(binary, [], env, ['ignore', log.fd, log.fd]); await log.close()
  await until(async () => { try { return (await rpc(kernel, {RelayStatus: {}})).RelayStatus.status.connected } catch { return false } }, name + ' hosted WSS')
  await dialog.getByText(/^Connected:/).waitFor({timeout: 90000}); await dialog.getByRole('button', {name: 'Close', exact: true}).click(); return kernel
}
const myKernels = async page => { await page.goto(origin + '/terminals'); await page.getByRole('heading', {name: 'My kernels', exact: true}).waitFor({timeout: 60000}) }
const groupOf = (page, kernel) => page.locator('.kernel-account-group').filter({has: page.getByText(kernel.identity.kernelId, {exact: true})})
async function openCopy(page) {
  const group = groupOf(page, target)
  await until(() => group.getByRole('button', {name: 'Copy a kernel here', exact: true}).isEnabled(), 'Copy enabled')
  await group.getByRole('button', {name: 'Copy a kernel here', exact: true}).click()
  const dialog = page.getByRole('dialog', {name: 'Copy a kernel here', exact: true})
  await until(async () => await dialog.getByLabel('Source kernel', {exact: true}).locator('option').count() > 1, 'Copy source choices', 60000)
  return dialog
}
// Iteration aid only; acceptance runs leave it unset and run every cell.
const onlySignout = process.env.BYOM_ONLY_SIGNOUT === '1'
const shallowMessage = "This Project's repository is a shallow clone. Fetch its full history (git fetch --unshallow), then copy again."
const kernelShallowAdvice = 'fetch its full history with `git fetch --unshallow`, then copy again'
try {
  await sample(); await mkdir(path.join(scratch, 'tmp'), {recursive: true}); proxy = await apiProxy(); network = await startHostNetwork(relay, {httpsOrigins: [origin]})
  browser = await chromium.launch({headless: true, executablePath: required('BYOM_CHROMIUM'), proxy: {server: network.proxyUrl}, args: ['--no-sandbox', '--disable-dev-shm-usage'], env: {...process.env, TMPDIR: path.join(scratch, 'tmp')}})
  const setup = await signedInContext(1), page = await newPage(setup)
  await myKernels(page); source = await enroll(page, 'source', proxy.url); target = await enroll(page, 'target', origin)
  // Shallow clone of a public repository, as the BY-03 owner prepared it.
  const shallowRepo = path.join(workspace, 'slugify'); await run('git', ['clone', '--quiet', '--depth', '1', 'https://github.com/sindresorhus/slugify.git', shallowRepo])
  assert.equal((await run('git', ['-C', shallowRepo, 'rev-parse', '--is-shallow-repository'])).trim(), 'true')
  const linked = (await rpc(source, {LinkProviderAccountProfile: {provider: 'codex', label: 'byom-account', path: required('BYOM_CODEX_PROFILE')}})).ProviderAccountProfile.profile
  await rpc(source, {RefreshProviderAccountProfile: {provider: 'codex', account_profile: linked.profile_id}})
  await rpc(source, {SetDefaultProviderAccountProfile: {provider: 'codex', account_profile: linked.profile_id}})
  const session = (await rpc(source, {CreateSession: {workspace_id: shallowRepo, worktree_id: shallowRepo, alias: 'byom-account-shallow', project_selection: {kind: 'new'}}})).SessionCreated.session
  socket = path.join(scratch, 'tui.sock'); tuiLog = path.join(scratch, 'tui.log'); const log = await open(tuiLog, 'w', 0o600)
  spawn('python3', [path.join(repo, 'apps/cli/scripts/lib/owner-context-terminal.py'), cli, '--kernel-url', source.url, '--automation-socket', socket, '--session', session.id, '--workspace', shallowRepo, '--worktree', shallowRepo, '--provider', 'codex', '--account-profile', linked.profile_id, '--model', 'codex/gpt-6.1-sol'], source.env, ['pipe', log.fd, log.fd]); await log.close()
  await until(async () => { try { return (await automation({action: 'snapshot'})).session?.id } catch { return false } }, 'real TUI ready')
  // A real official provider turn confirms that the enrolled runtime and TUI
  // still use the normal linked-profile path. No login mutation is involved.
  await automation({action: 'submit_prompt', prompt: 'Reply with BYOMREL-R35-OK only.'})
  await until(async () => {
    const snapshot = await automation({action: 'snapshot'})
    return snapshot.transcript?.entries?.some(entry => entry.role === 'assistant' && entry.text?.includes('BYOMREL-R35-OK'))
      && snapshot.session?.agents?.every(agent => !agent.isProcessing)
  }, 'real provider turn settled', 180000)
  report.providerSmoke = {provider: 'codex', marker: 'BYOMREL-R35-OK', status: 'PASS'}
  const smokeLog = path.join(E, 'provider-tui.log'); await writeFile(smokeLog, safe(await readFile(tuiLog, 'utf8')))
  await run('python3', [path.join(repo, 'apps/cli/scripts/lib/owner-context-terminal.py'), '--render', smokeLog, path.join(E, 'provider-tui.png')])
  await setup.close()

  for (const dpr of [1, 2]) {
    const context = await signedInContext(dpr), page = await newPage(context)
    if (!onlySignout) {
    // names: My kernels and the Copy picker for two kernels of one host.
    await sample(); await myKernels(page)
    const sourceLabel = (await groupOf(page, source).locator('h3').innerText()).trim(), targetLabel = (await groupOf(page, target).locator('h3').innerText()).trim()
    await shot(page, `dpr${dpr}-names-my-kernels`)
    const dialog = await openCopy(page)
    const options = await dialog.getByLabel('Source kernel', {exact: true}).locator('option').allInnerTexts()
    const sourceOption = await dialog.getByLabel('Source kernel', {exact: true}).locator('option').filter({hasNotText: 'Choose your online kernel'}).first().innerText()
    const dialogText = await dialog.innerText()
    await shot(page, `dpr${dpr}-names-copy-picker`)
    const names = {cell: 'names', dpr, sourceLabel, targetLabel, sourceOptions: options, copyDestinationNamed: dialogText.includes('Copying to ')}
    names.status = sourceLabel !== targetLabel && names.copyDestinationNamed && !dialogText.split('\n').includes(`Copying to ${sourceOption}`) ? 'PASS' : 'FAIL'
    cells.push(names); await save(); console.log('MP-08 / MP-10 / MP-11 BY-03 names DPR' + dpr, names.status)

    // shallow: the source refuses before packaging; Web and TUI explain recovery.
    const shallow = {cell: 'shallow', dpr, status: 'FAIL'}; cells.push(shallow); contextId = null
    const peers = await page.evaluate(async () => (await (await fetch('/v1/owner-managed-context-tickets/peers', {headers: {'x-chariox-browser-session': sessionStorage.getItem('chariox-browser-cloud-session') ?? ''}})).json()).peers)
    await dialog.getByLabel('Source kernel', {exact: true}).selectOption(peers.find(p => p.peer.kernelId === source.identity.kernelId).targetId)
    await dialog.getByLabel('Project to copy').selectOption(session.project_id)
    await dialog.getByLabel('Include kernel context, without credentials').uncheck()
    await dialog.getByRole('button', {name: 'Review and copy', exact: true}).click()
    const proceed = dialog.getByRole('button', {name: 'Looks good, continue', exact: true}), stopped = dialog.getByText('Copy stopped.', {exact: true})
    await until(async () => await proceed.isVisible() || await stopped.isVisible(), 'source review or refusal', 120000)
    if (await proceed.isVisible()) { await shot(page, `dpr${dpr}-shallow-review`); shallow.reviewed = true; await proceed.click() }
    await stopped.waitFor({timeout: 120000}); await shot(page, `dpr${dpr}-shallow-refused`)
    shallow.visibleText = (await dialog.innerText()).split('\n').filter(line => /copy|clone|refused|stopped/i.test(line))
    assert(contextId, 'Copy context observed'); const status = (await rpc(source, {GetManagedContextTransferStatus: {contextId}})).ManagedContextTransferStatus.status
    shallow.authoritative = {phase: status.phase, failureCode: status.failureCode, acceptedBytes: status.acceptedBytes, packageSizeBytes: status.packageSizeBytes, receipt: Boolean(status.receipt)}
    shallow.webSpecific = shallow.visibleText.includes(shallowMessage)
    await automation({action: 'submit_prompt', prompt: '/kernel copy-status ' + contextId}); await pause(500)
    const terminal = path.join(E, `dpr${dpr}-shallow-tui.log`); const vt = safe(await readFile(tuiLog, 'utf8')); await writeFile(terminal, vt)
    await run('python3', [path.join(repo, 'apps/cli/scripts/lib/owner-context-terminal.py'), '--render', terminal, path.join(E, `dpr${dpr}-shallow-tui.png`)])
    shallow.safeKernelMessage = !status.failureMessage?.startsWith('managed context `')
    shallow.tuiActionable = JSON.stringify(await automation({action: 'snapshot'})).includes(kernelShallowAdvice) || vt.replace(/\x1b\[[0-9;?]*[A-Za-z]/g, '').includes('git fetch --unshallow')
    shallow.status = status.phase === 'failed' && status.acceptedBytes === 0 && !status.receipt && shallow.webSpecific && shallow.tuiActionable && shallow.safeKernelMessage ? 'PASS' : 'FAIL'
    await save(); console.log('MP-08 / MP-10 / MP-11 BY-03 shallow DPR' + dpr, shallow.status)
    await dialog.getByRole('button', {name: 'Close', exact: true}).click()

    // control: visible Sign out on My kernels and the waiting room.
    const control = {cell: 'control', dpr}; cells.push(control)
    control.myKernels = await page.getByRole('button', {name: 'Sign out', exact: true}).count()
    await shot(page, `dpr${dpr}-control-my-kernels`)
    await page.goto(origin + '/waiting-room'); await page.locator('[data-app-sidebar]').first().waitFor({timeout: 60000})
    const rail = page.locator('[data-app-sidebar]').getByRole('button', {name: 'Sign out', exact: true})
    control.waitingRoom = await rail.count()
    if (control.waitingRoom) await rail.hover()
    await shot(page, `dpr${dpr}-control-waiting-room`)
    control.status = control.myKernels === 1 && control.waitingRoom === 1 ? 'PASS' : 'FAIL'
    await save(); console.log('MP-08 / MP-10 / MP-11 BY-03 control DPR' + dpr, control.status)
    }
    await context.close()

    // signout: two tabs of one sign-in, each with its own Cloud session and relay grant.
    const signout = {cell: 'signout', dpr, status: 'FAIL'}; cells.push(signout)
    const login = await signedInContext(dpr), tabs = [await newPage(login), await newPage(login)]
    // A sibling browser retains the cookie authenticated before sign-out. Its
    // later real page load must not publish a new Cloud session for that login.
    const sibling = await browser.newContext({deviceScaleFactor: dpr, viewport: {width: 1440, height: 1000}})
    await sibling.addCookies(await login.cookies())
    const reloading = await newPage(sibling)
    for (const tab of tabs) {
      report.stage = `signout dpr${dpr} tab ${tabs.indexOf(tab)} connect`
      await tab.bringToFront(); await myKernels(tab); const copy = await openCopy(tab)
      // Selecting a source connects this tab to both kernels through the relay with its own grant.
      const peerList = await tab.evaluate(async () => (await (await fetch('/v1/owner-managed-context-tickets/peers', {headers: {'x-chariox-browser-session': sessionStorage.getItem('chariox-browser-cloud-session') ?? ''}})).json()).peers)
      await copy.getByLabel('Source kernel', {exact: true}).selectOption(peerList.find(p => p.peer.kernelId === source.identity.kernelId).targetId)
      await copy.getByLabel('Project to copy').waitFor({timeout: 90000})
      await until(() => tab.openRelayConnections().length >= 2, 'both target and source relay WSS', 90000)
    }
    const observedConnections = tabs[0].openRelayConnections()
    signout.otherTabConnectionsBefore = observedConnections.length
    const tokens = await Promise.all(tabs.map(tab => tab.evaluate(() => sessionStorage.getItem('chariox-browser-cloud-session'))))
    signout.distinctCloudSessions = Boolean(tokens[0] && tokens[1] && tokens[0] !== tokens[1]); confidential.push(...tokens.filter(Boolean))
    // A transient network error is recorded and retried; only HTTP 401 counts as rejection.
    const peersStatus = tab => tab.evaluate(async () => (await fetch('/v1/owner-managed-context-tickets/peers', {headers: {'x-chariox-browser-session': sessionStorage.getItem('chariox-browser-cloud-session') ?? ''}})).status)
      .catch(() => { signout.probeNetworkErrors = (signout.probeNetworkErrors ?? 0) + 1; return null })
    signout.otherTabBefore = await peersStatus(tabs[0])
    const [, signing] = tabs; await signing.bringToFront(); await signing.getByRole('button', {name: 'Close', exact: true}).click()
    const button = signing.getByRole('button', {name: 'Sign out', exact: true})
    signout.usedVisibleControl = await button.count() === 1
    report.stage = `signout dpr${dpr} sign out`
    const startedAt = Date.now()
    if (signout.usedVisibleControl) await Promise.all([signing.waitForURL(url => url.pathname === '/', {timeout: 60000}), button.click()])
    // Base has no control: send the request the product endpoint documents, so revocation itself is measured.
    else signout.endpointStatus = await signing.evaluate(async () => { const csrf = (await (await fetch('/auth/csrf')).json()).csrfToken; return (await fetch('/auth/browser-logout', {method: 'POST', headers: {'content-type': 'application/json', 'csrf-token': csrf, 'x-chariox-browser-session': sessionStorage.getItem('chariox-browser-cloud-session') ?? ''}, body: '{}'})).status })
    await shot(signing, `dpr${dpr}-signout-after`)
    const deadline = startedAt + 90000; let rejectedAt = null
    while (Date.now() < deadline && (rejectedAt === null || tabs[0].openRelayConnections().length)) {
      if (rejectedAt === null && await peersStatus(tabs[0]) === 401) rejectedAt = Date.now()
      await pause(1000)
    }
    signout.otherTabApiRejectedMs = rejectedAt && rejectedAt - startedAt
    signout.otherTabRelayEvents = tabs[0].relayEvents.map(e => ({event: e.event, connectionId: e.connectionId, relayOrigin: e.relayOrigin, msAfterSignOut: e.at - startedAt}))
    signout.otherTabOpenSocketsAfterWindow = tabs[0].openRelayConnections().length
    signout.everyObservedConnectionClosed = observedConnections.every(id => tabs[0].relayEvents.some(e => e.connectionId === id && e.event === 'closed'))
    const publication = reloading.waitForResponse(response => new URL(response.url()).pathname === '/auth/cloud-session' && response.request().method() === 'POST', {timeout: 60000})
    await reloading.goto(origin + '/terminals')
    signout.siblingPublicationAfterSignOut = (await publication).status()
    await shot(reloading, `dpr${dpr}-signout-stale-login-reload`)
    signout.sessionAfter = await tabs[0].evaluate(async () => (await (await fetch('/auth/session')).json()).authenticated)
    await tabs[0].bringToFront(); await shot(tabs[0], `dpr${dpr}-signout-other-tab`)
    await myKernels(tabs[0]).catch(() => {}); await shot(tabs[0], `dpr${dpr}-signout-other-tab-my-kernels`)
    signout.status = signout.distinctCloudSessions && signout.otherTabBefore === 200 && signout.usedVisibleControl && rejectedAt && signout.otherTabOpenSocketsAfterWindow === 0 && signout.everyObservedConnectionClosed && signout.siblingPublicationAfterSignOut === 403 && signout.sessionAfter === false ? 'PASS' : 'FAIL'
    await save(); console.log('MP-08 / MP-10 / MP-11 BY-03 signout DPR' + dpr, signout.status)
    await sibling.close(); await login.close()
  }
  report.network = await network.snapshot(); report.status = cells.every(cell => cell.status === 'PASS') && !report.screenshotErrors?.length ? 'PASS' : 'FAIL'
  if (report.status === 'FAIL') process.exitCode = 1
} catch (error) { for (const [index, open] of (browser?.contexts() ?? []).flatMap(context => context.pages()).entries()) await shot(open, `failure-${index}`); report.status = 'FAIL'; report.error = safe(error.message).split('Call log:')[0].slice(0, 1200); process.exitCode = 1 }
finally {
  if (browser) await browser.close(); if (network) report.networkCleanup = await network.close()
  for (const child of children.reverse()) await stop(child)
  if (proxy) { proxy.server.closeAllConnections(); await new Promise(resolve => proxy.server.close(resolve)) }
  await rm(scratch, {recursive: true, force: true}); await rm(workspace, {recursive: true, force: true})
  report.finished = new Date().toISOString(); report.cleanup = {scratchRemoved: true, workspaceRemoved: true, ownedChildrenStopped: children.length}
  await save(); console.log('MP-08 / MP-10 / MP-11 BY-03 account drill', report.status)
}
