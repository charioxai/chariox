// MP-08 / MP-09 / MP-10 / MP-11: real Codex, kernel, scoped relay and TUI.
// Hold the next official catalog helper before exec, then exercise independent
// requests while its supervisor waits. No provider responses are fabricated.
// Example (build kernel-client and CLI first):
// node scripts/drills/kernel-catalog-stall.mjs --kernel-binary /abs/kernel \
//   --relay-binary /abs/relay --codex-scope /product/linked/codex \
//   --state-root /root/.chariox/dev/kernelhang/drill --evidence-dir /abs/evidence
import assert from 'node:assert/strict'
import { spawn, execFileSync } from 'node:child_process'
import { createHash, createHmac, randomBytes, randomUUID } from 'node:crypto'
import { mkdirSync, mkdtempSync, openSync, readFileSync, writeFileSync, existsSync, rmSync, readdirSync } from 'node:fs'
import { createServer, createConnection } from 'node:net'
import { resolve, join } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { LocalIpcClient } from '../../packages/kernel-client/dist/ipc.js'
import { RelayClientIdentity, createRelayKeypair } from '../../packages/kernel-client/dist/relay-crypto.js'
import { getProviderCatalogRequest, launchProviderRunRequest } from '../../packages/kernel-client/dist/ipc-requests.js'

const flags = new Map()
for (let i = 2; i < process.argv.length; i += 2) flags.set(process.argv[i], process.argv[i + 1])
const required = name => { assert(flags.get(name), `${name} is required`); return resolve(flags.get(name)) }
const kernelBinary = required('--kernel-binary'), relayBinary = required('--relay-binary')
const codexScope = required('--codex-scope'), stateRoot = required('--state-root'), evidence = required('--evidence-dir')
assert(stateRoot.startsWith('/root/.chariox/dev/'), 'use a disposable lane-owned state directory')
assert(!evidence.startsWith(resolve(import.meta.dirname, '../..') + '/'), 'evidence must be outside the repository')
mkdirSync(stateRoot, { recursive: true, mode: 0o700 })
mkdirSync(evidence, { recursive: true, mode: 0o700 })
const root = mkdtempSync(join(stateRoot, 'run-'))
const steps = [], children = [], clients = [], sockets = []
let tuiSocketPath, tuiServer, tmuxEnvironment
const record = (name, detail = {}) => {
  steps.push({ at: new Date().toISOString(), name, ...detail })
  writeFileSync(join(evidence, 'steps.json'), JSON.stringify(steps, null, 2), { mode: 0o600 })
  console.log(`MP-08/09/10/11 ${name}`)
}
async function bounded(promise, label, ms = 2000) {
  const at = performance.now()
  let timer
  try {
    const result = await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`${label} exceeded ${ms} ms`)), ms) })])
    record(label, { elapsed_ms: performance.now() - at, deadline_ms: ms })
    return result
  } finally { clearTimeout(timer) }
}
async function until(check, label, ms = 30000) {
  const end = Date.now() + ms
  while (Date.now() < end) { if (await check()) return; await delay(100) }
  throw new Error(`${label} did not become ready`)
}
async function port() {
  const server = createServer()
  await new Promise(r => server.listen(0, '127.0.0.1', r))
  const value = server.address().port
  await new Promise(r => server.close(r))
  return value
}
function start(program, args, env, label, stdio = null) {
  const log = openSync(join(evidence, `${label}.log`), 'a', 0o600)
  const child = spawn(program, args, { env, cwd: join(root, 'work'), stdio: stdio ?? ['ignore', log, log] })
  assert(Number.isSafeInteger(child.pid) && child.pid > 1, 'owned child PID')
  children.push(child)
  record('process-start', { label, pid: child.pid })
  return child
}
async function stop(child) {
  if (child.exitCode !== null || child.signalCode !== null) return
  assert(Number.isSafeInteger(child.pid) && child.pid > 1, 'refuse reserved PID')
  child.kill('SIGTERM')
  try { await until(() => child.exitCode !== null || child.signalCode !== null, 'owned process exit', 15000) }
  catch {
    child.kill('SIGKILL')
    await until(() => child.exitCode !== null || child.signalCode !== null, 'owned process reap', 5000)
  }
  record('process-exit', { pid: child.pid, code: child.exitCode, signal: child.signalCode })
}
const secret = randomBytes(32).toString('base64url'), issuer = `catalog-stall-${randomUUID()}`
function token(subject, subject_kind, allowed_actions, thumbprint) {
  const header = Buffer.from(JSON.stringify({ alg: 'HS256', typ: 'JWT' })).toString('base64url')
  const payload = Buffer.from(JSON.stringify({ iss: issuer, sub: subject, subject_kind, realm_id: issuer, allowed_actions,
    allowed_targets: null, iat: Math.floor(Date.now() / 1000) - 1, exp: Math.floor(Date.now() / 1000) + 1800,
    jti: randomUUID(), user_id: 'local', ...(thumbprint ? { public_key_thumbprint: thumbprint } : {}) })).toString('base64url')
  const input = `${header}.${payload}`
  return `${input}.${createHmac('sha256', secret).update(input).digest('base64url')}`
}
const baseEnv = { PATH: process.env.PATH, NODE_OPTIONS: '--max-old-space-size=3072', IS_SANDBOX: '1', SHELL: '/bin/bash', TERM: 'xterm-256color', LANG: 'C.UTF-8' }
const rpc = async (client, request) => {
  const result = await client.send(request)
  if (result.Error) throw new Error(`${Object.keys(request)[0]}: ${result.Error.code}: ${result.Error.message}`)
  return result
}
async function automation(path, request) {
  return new Promise((resolve, reject) => {
    const socket = createConnection(path), timer = setTimeout(() => { socket.destroy(); reject(new Error('TUI automation deadline')) }, 15000)
    let data = ''
    socket.on('error', error => { clearTimeout(timer); reject(error) })
    socket.on('connect', () => socket.write(JSON.stringify({ id: randomUUID(), ...request }) + '\n'))
    socket.on('data', chunk => {
      data += chunk
      if (!data.includes('\n')) return
      clearTimeout(timer); socket.end()
      const result = JSON.parse(data.split('\n')[0])
      if (result.error) reject(new Error(result.error)); else resolve(result.data)
    })
  })
}

const tmux = (...args) => execFileSync('tmux', ['-S', tuiSocketPath, ...args], { encoding: 'utf8', env: tmuxEnvironment })
async function photographTui(label, env) {
  const text = tmux('capture-pane', '-p', '-t', 'drill')
  writeFileSync(join(evidence, `${label}.txt`), text)
  const html = join(root, `${label}.html`)
  writeFileSync(html, '<!doctype html><meta charset="utf-8"><body style="background:#111;color:#ddd"><pre style="font:14px monospace">' + text.replaceAll('&', '&amp;').replaceAll('<', '&lt;') + '</pre>')
  const browser = start('google-chrome', ['--headless=new', '--no-sandbox', '--disable-gpu', '--hide-scrollbars', '--window-size=1300,800', `--user-data-dir=${root}/capture-browser`, `--screenshot=${evidence}/${label}.png`, `file://${html}`], env, `capture-${label}`)
  await bounded(until(() => browser.exitCode !== null || browser.signalCode !== null, 'screen capture exit', 30000), `capture-${label}`, 35000)
  assert.equal(browser.exitCode, 0, 'TUI screen capture must succeed')
}

function watchdogNotices(directory) {
  const notices = []
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name)
    if (entry.isDirectory()) notices.push(...watchdogNotices(path))
    else if (entry.isFile()) {
      for (const line of readFileSync(path, 'utf8').split('\n')) {
        if (!line.trim()) continue
        try {
          const notice = JSON.parse(line)
          if (notice.message === 'kernel operation stalled' && notice.component === 'codex.catalog'
            && ['ensure', 'supervisor_reply'].includes(notice.operation)) notices.push(line)
        } catch { /* A logger may still be appending the last line. */ }
      }
    }
  }
  return notices
}

// The peer probe uses the product crypto implementation and an ephemeral scoped
// kernel identity. It only creates/destroys a lease; it never runs a provider.
async function peerProbe(relayUrl, target, publicKey) {
  const identity = new RelayClientIdentity(createRelayKeypair().privateKey), id = `probe-${randomUUID()}`
  const socket = new WebSocket(relayUrl)
  sockets.push(socket)
  const pending = new Map()
  socket.addEventListener('message', event => {
    const message = JSON.parse(String(event.data))
    if (message.kind === 'daemon_peer_response') {
      const waiter = pending.get(message.request_id)
      if (!waiter) return
      pending.delete(message.request_id)
      if (message.error) waiter.reject(new Error(`peer: ${message.error.code}`))
      else waiter.resolve(JSON.parse(identity.decrypt(message.encrypted_response, publicKey)))
    }
  })
  await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, { once: true }); socket.addEventListener('error', reject, { once: true }) })
  socket.send(JSON.stringify({ kind: 'daemon_register', registration: {
    auth_token: token(id, 'kernel', ['daemon.register', 'daemon.heartbeat', 'packet.route', 'peer.request', 'peer.event'], identity.publicKeyThumbprint),
    daemon_id: id, machine_id: id, public_key: identity.publicKeyBase64, capabilities: ['kernel_ws'],
  } }))
  await until(async () => {
    try { return await targetPublicKey(relayUrl, id) === identity.publicKeyBase64 } catch { return false }
  }, 'peer probe relay registration')
  return { id, request: request => new Promise((resolve, reject) => {
    const request_id = randomUUID()
    pending.set(request_id, { resolve, reject })
    socket.send(JSON.stringify({ kind: 'daemon_peer_request', request_id, target: { daemon_id: target }, encrypted_request: identity.encrypt(publicKey, JSON.stringify(request)) }))
  }) }
}

async function targetPublicKey(relayUrl, kernelId) {
  const socket = new WebSocket(relayUrl), request_id = randomUUID()
  sockets.push(socket)
  const response = new Promise((resolve, reject) => {
    socket.addEventListener('message', event => {
      const message = JSON.parse(String(event.data))
      if (message.kind !== 'client_metadata_response' || message.request_id !== request_id) return
      if (message.error) reject(new Error(`relay metadata: ${message.error.code}`))
      else resolve(message.kernel?.public_key)
    })
    socket.addEventListener('error', reject, { once: true })
  })
  await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, { once: true }); socket.addEventListener('error', reject, { once: true }) })
  socket.send(JSON.stringify({ kind: 'client_metadata_request', request_id, auth_token: token('catalog-metadata', 'client', ['client.metadata.read']), query: { kind: 'get_live_kernel', kernel_ref: kernelId } }))
  const key = await bounded(response, 'relay-public-key-metadata', 10000)
  assert(key, 'registered kernel must have a public key')
  socket.close()
  return key
}

async function main() {
  for (const dir of ['chariox', 'home', 'tmp', 'work', 'config', 'cache', 'state', 'history', 'logs']) mkdirSync(join(root, dir), { mode: 0o700 })
  writeFileSync(join(root, 'chariox/config.toml'), `[credential_vault]\nbackend = "chariox_encrypted"\npath = "${root}/chariox/vault.json"\nunlock_policy = "kernel_init"\n[state]\npath = "${root}/chariox/state.db"\n`, { mode: 0o600 })
  const realCodex = execFileSync('which', ['codex'], { encoding: 'utf8' }).trim()
  const wrapper = join(root, 'codex-wrapper')
  // Only catalog app-servers (without runtime MCP overrides) see the fault.
  writeFileSync(wrapper, `#!/usr/bin/python3\nimport os,sys,time,signal,pathlib,subprocess\nr=pathlib.Path(${JSON.stringify(root)})\nif 'app-server' in sys.argv and not any('mcp_servers.' in a for a in sys.argv):\n signal.signal(signal.SIGTERM,signal.SIG_IGN)\n (r/'catalog.pid').write_text(str(os.getpid()))\n if (r/'stall-next').exists():\n  (r/'stall-next').unlink()\n  (r/'stalled').write_text(str(os.getpid()))\n  end=time.monotonic()+25\n  while not (r/'release').exists() and time.monotonic()<end: time.sleep(.02)\n child=subprocess.Popen([${JSON.stringify(realCodex)}]+sys.argv[1:])\n while child.poll() is None: time.sleep(.02)\n sys.exit(child.returncode)\nos.execv(${JSON.stringify(realCodex)},[${JSON.stringify(realCodex)}]+sys.argv[1:])\n`, { mode: 0o700 })
  const relayPort = await port(), kernelPort = await port(), mcpPort = await port(), codexPort = await port(), opencodePort = await port()
  const relayUrl = `ws://127.0.0.1:${relayPort}`, endpoint = `ws://127.0.0.1:${kernelPort}/kernel`
  start(relayBinary, [], { ...baseEnv, HOME: root, CHARIOX_HOME: join(root, 'relay'), CHARIOX_RELAY_HOST: '127.0.0.1', CHARIOX_RELAY_PORT: String(relayPort), CHARIOX_RELAY_SCOPED_ISSUER: issuer, CHARIOX_RELAY_SCOPED_HMAC_SECRET: secret, CHARIOX_RELAY_STUN_ADDRESS: 'off' }, 'relay')
  const env = { ...baseEnv, HOME: join(root, 'home'), TMPDIR: join(root, 'tmp'), CHARIOX_HOME: join(root, 'chariox'),
    XDG_CONFIG_HOME: join(root, 'config'), XDG_CACHE_HOME: join(root, 'cache'), XDG_STATE_HOME: join(root, 'state'),
    CHARIOX_LOG_DIR: join(root, 'logs'), CHARIOX_SESSION_HISTORY_DIR: join(root, 'history'),
    CHARIOX_DAEMON_SOCKET: join(root, 'k.sock'), CHARIOX_KERNEL_HOST: '127.0.0.1', CHARIOX_KERNEL_PORT: String(kernelPort),
    CHARIOX_MCP_PORT: String(mcpPort), CHARIOX_CODEX_PORT: String(codexPort), CHARIOX_OPENCODE_PORT: String(opencodePort), CHARIOX_CODEX_BIN: wrapper }
  start(kernelBinary, [], env, 'kernel')
  const client = new LocalIpcClient(endpoint, { localAuthEnvironment: env, controlRequestRetryDeadlineMs: 1000 })
  clients.push(client)
  await until(async () => { try { return !!(await rpc(client, { RelayStatus: null })).RelayStatus } catch { return false } }, 'owner access')
  const status = (await rpc(client, { RelayStatus: null })).RelayStatus.status
  await rpc(client, { ConfigureRelay: { relay_url: relayUrl, relay_token: token(status.daemon_id, 'kernel', ['daemon.register', 'daemon.heartbeat', 'packet.route', 'peer.request', 'peer.event']) } })
  await until(async () => (await rpc(client, { RelayStatus: null })).RelayStatus.status.connected, 'scoped relay registration')
  record('owner-access-and-scoped-relay')
  const profile = (await bounded(rpc(client, { LinkProviderAccountProfile: { provider: 'codex', label: 'catalog stall drill', path: codexScope } }), 'link-product-profile', 30000)).ProviderAccountProfile.profile.profile_id
  await bounded(rpc(client, { GetProviderAuthStatus: { provider: 'codex', account_profile: profile } }), 'real-provider-auth', 45000)
  const catalogRequest = getProviderCatalogRequest({ provider: 'codex', accountProfile: profile })
  await bounded(rpc(client, catalogRequest), 'real-provider-catalog', 45000)
  const oldHelperPid = Number(readFileSync(join(root, 'catalog.pid'), 'utf8'))
  assert(Number.isSafeInteger(oldHelperPid) && oldHelperPid > 1, 'catalog wrapper PID')
  const oldHelperBirth = readFileSync(`/proc/${oldHelperPid}/stat`, 'utf8').split(') ').at(-1).split(' ')[19]
  record('catalog-helper-observed', { pid: oldHelperPid, start_ticks: oldHelperBirth })
  const session = (await rpc(client, { CreateSession: { workspace_id: join(root, 'work'), worktree_id: join(root, 'work'), alias: 'catalog-stall', agent_defaults: { provider: 'codex', account_profile: profile, model: 'gpt-6-luna', effort: 'low', permission_level: 'yolo' } } })).SessionCreated
  const tuiSocket = join(root, 'tui.sock')
  // tmux supplies a real PTY and captures the screen produced by normal keys.
  const tuiArgs = ['bun', resolve('apps/cli/dist/index.js'), '--kernel-url', endpoint, '--session', session.session.id, '--provider', 'codex', '--account-profile', profile, '--model', 'gpt-6-luna', '--effort', 'low', '--automation-socket', tuiSocket]
  tuiSocketPath = join(root, 'tmux.sock')
  tmuxEnvironment = env
  tmux('-f', '/dev/null', 'new-session', '-d', '-s', 'drill', '-x', '140', '-y', '42', '-c', join(root, 'work'),
    ...Object.entries(env).flatMap(([key, value]) => ['-e', `${key}=${value}`]), ...tuiArgs)
  tuiServer = Number(tmux('display-message', '-p', '#{pid}').trim())
  assert(Number.isSafeInteger(tuiServer) && tuiServer > 1, 'owned tmux server PID')
  record('tui-start', { server_pid: tuiServer, pane_pid: Number(tmux('display-message', '-p', '#{pane_pid}').trim()) })
  await until(() => existsSync(tuiSocket), 'real TUI ready', 45000)
  await until(async () => (await automation(tuiSocket, { action: 'snapshot' })).attachmentId, 'TUI attached to Room')
  tmux('send-keys', '-t', 'drill', '-l', 'Reply exactly KERNEL_CATALOG_STALL_OK. Do not call tools.')
  // Keep submit out of the terminal's bracketed-paste/input batch.
  await delay(100)
  tmux('send-keys', '-t', 'drill', 'Enter')
  await until(async () => {
    const snapshot = await automation(tuiSocket, { action: 'snapshot' })
    writeFileSync(join(evidence, 'tui-snapshot.json'), JSON.stringify(snapshot, null, 2))
    return Object.values(snapshot.agentPanes ?? {}).flat().concat(snapshot.transcript?.entries ?? [])
      .some(entry => entry.role === 'assistant' && entry.text?.trim() === 'KERNEL_CATALOG_STALL_OK')
  }, 'real provider answer in TUI', 150000)
  record('real-tui-provider-answer', { model: 'gpt-6-luna', effort: 'low' })
  await photographTui('provider-answer', env)

  const pairing = (await rpc(client, { CreateTerminalPairingLink: { terminal_type: 'cli', expires_in_ms: 300000 } })).TerminalPairingLinkCreated.pairing
  const identity = new RelayClientIdentity(createRelayKeypair().privateKey)
  const remote = new LocalIpcClient(relayUrl, { transport: 'relay', relayAuthToken: token(pairing.terminal_id, 'client', ['client.connect', 'client.metadata.read', 'packet.route'], identity.publicKeyThumbprint), targetDaemonId: status.daemon_id, relayIdentity: identity, controlRequestRetryDeadlineMs: 1000 })
  clients.push(remote)
  await bounded(rpc(remote, { JoinTerminalPairingLink: { pairing_link: pairing.pairing_link, terminal_id: pairing.terminal_id, terminal_type: 'cli', public_key_thumbprint: identity.publicKeyThumbprint } }), 'relay-client-pairing', 10000)
  const att = (await rpc(remote, { AttachToSession: { session_id: session.session.id, client_id: 'catalog-probe', capability_level: 'FullTerminal' } })).SessionAttached.attachment.id
  const targetKey = await targetPublicKey(relayUrl, status.daemon_id)
  const peer = await peerProbe(relayUrl, status.daemon_id, targetKey)
  const contender = (await rpc(client, { CreateSession: { workspace_id: join(root, 'work'), worktree_id: join(root, 'work'), alias: 'concurrent-provider-launch' } })).SessionCreated
  const launchClient = new LocalIpcClient(endpoint, { localAuthEnvironment: env, controlRequestRetryDeadlineMs: 1000 })
  clients.push(launchClient)
  writeFileSync(join(root, 'stall-next'), '1')
  const refresh = bounded(rpc(client, { RefreshProviderAccountProfile: { provider: 'codex', account_profile: profile } }), 'catalog-refresh-completes', 45000)
  refresh.catch(() => {})
  await until(() => existsSync(join(root, 'stalled')), 'catalog helper fault armed')
  let oldHelperState = 'gone'
  if (existsSync(`/proc/${oldHelperPid}/stat`)) {
    const fields = readFileSync(`/proc/${oldHelperPid}/stat`, 'utf8').split(') ').at(-1).split(' ')
    oldHelperState = fields[19] === oldHelperBirth ? fields[0] : 'reused'
  }
  assert(['gone', 'reused', 'Z', 'X'].includes(oldHelperState), 'TERM-ignoring catalog wrapper must exit before replacement')
  record('term-ignoring-helper-exited', { pid: oldHelperPid, state: oldHelperState })
  record('catalog-helper-held', { pid: Number(readFileSync(join(root, 'stalled'), 'utf8')) })
  // The incident's middle edge was another provider launch holding STORE while
  // waiting for the environment lock held by catalog startup.
  const launch = bounded(rpc(launchClient, launchProviderRunRequest(contender.session.id, 'codex', profile, 'gpt-6-luna', 'low', contender.agent.id)), 'provider-launch-during-stall', 30000)
  launch.catch(() => {})
  await delay(2500)
  const notices = watchdogNotices(join(root, 'logs'))
  assert(notices.length > 0, 'watchdog must name the stalled catalog operation')
  writeFileSync(join(evidence, 'watchdog-notices.jsonl'), notices.join('\n') + '\n')
  record('watchdog-names-stalled-operation', { count: notices.length })
  let events = 0
  remote.onKernelEvent(() => { events++ })
  const leaseRequest = { kind: 'create_execution_lease', home_kernel_id: peer.id, home_session_id: session.session.id, home_agent_id: session.agent.id, home_agent_metaagent: false, owner_user_id: 'local' }
  const [, , , lease] = await Promise.all([
    bounded(rpc(remote, { CreateSession: { workspace_id: join(root, 'work'), worktree_id: join(root, 'work'), alias: 'during-catalog-stall' } }), 'CreateSession-during-stall'),
    bounded(rpc(remote, { ListSessions: null }), 'ListSessions-during-stall'),
    bounded(remote.subscribeToKernelEvents(session.session.id, att, { heartbeatIntervalMs: 500 }), 'Subscribe-during-stall'),
    bounded(peer.request(leaseRequest), 'CreateExecutionLease-during-stall'),
  ])
  assert.equal(lease.kind, 'execution_lease_created')
  await bounded(peer.request({ kind: 'destroy_execution_lease', lease_id: lease.lease.id }), 'DestroyExecutionLease-during-stall')
  await until(() => events > 0, 'subscription delivers during stall', 2000)
  record('events-delivered-during-stall', { count: events })
  assert(!existsSync(join(root, 'release')), 'probes must finish before releasing the helper')
  await photographTui('during-catalog-stall', env)
  writeFileSync(join(root, 'release'), '1')
  await refresh
  await launch
  await bounded(rpc(client, catalogRequest), 'catalog-recovers', 45000)
  // Watchdog output is saved by the normal logger in the disposable log root.
  record('PASS')
  await automation(tuiSocket, { action: 'exit' }).catch(() => {})
}

record('binaries', { kernel: createHash('sha256').update(readFileSync(kernelBinary)).digest('hex'), relay: createHash('sha256').update(readFileSync(relayBinary)).digest('hex') })
try { await main() }
catch (error) { record('FAIL', { message: error.message }); process.exitCode = 1 }
finally {
  writeFileSync(join(root, 'release'), '1')
  for (const socket of sockets) socket.close()
  for (const client of clients.reverse()) await client.close().catch(() => {})
  if (tuiServer) {
    // The socket was created by this drill and has exactly one owned session.
    try { tmux('kill-session', '-t', 'drill') } catch {}
    await until(() => !existsSync(tuiSocketPath), 'owned tmux server exit', 5000).catch(() => {})
  }
  for (const child of children.reverse()) await stop(child)
  // Keep normal logs, never provider profiles, identities or tokens, as evidence.
  execFileSync('cp', ['-r', join(root, 'logs'), join(evidence, 'kernel-logs')])
  rmSync(root, { recursive: true, force: true })
  record('cleanup-complete')
}
