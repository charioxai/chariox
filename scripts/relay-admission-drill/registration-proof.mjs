// Relay registration proof drill (relay peer 449): a real relay and a real kernel, plus an impersonator that holds the
// kernel's leaked relay token and public key but not its identity key. The kernel registers by signing the relay's
// challenge; the impersonator is refused and never displaces it; the kernel registers again after a relay restart.
// Usage: RELAY_BIN=<chariox-relay> KERNEL_BIN=<chariox-kernel> DRILL_STATE=<empty scratch dir> DRILL_EVIDENCE=<dir>
//   node scripts/relay-admission-drill/registration-proof.mjs
// Every token and HMAC secret is minted here for this run and never printed.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { appendFileSync, mkdirSync, openSync } from 'node:fs'
import { createHash, createHmac, generateKeyPairSync, randomBytes, sign } from 'node:crypto'
import { createRequire } from 'node:module'
import net from 'node:net'

const repo = new URL('../..', import.meta.url).pathname.replace(/\/$/, '')
const WebSocket = createRequire(`${repo}/packages/kernel-client/package.json`)('ws')
const { RELAY_BIN, KERNEL_BIN, DRILL_STATE, DRILL_EVIDENCE } = process.env
assert.ok(RELAY_BIN && KERNEL_BIN && DRILL_STATE && DRILL_EVIDENCE, 'set RELAY_BIN, KERNEL_BIN, DRILL_STATE and DRILL_EVIDENCE')
const KERNEL_ID = 'drill-kernel', REALM = 'drill-realm', ISSUER = 'drill-cloud'
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
const sha256hex = text => createHash('sha256').update(text).digest('hex')
const b64url = value => Buffer.from(value).toString('base64url')
const record = entry => {
  console.log(JSON.stringify(entry))
  appendFileSync(`${DRILL_EVIDENCE}/registration-proof.jsonl`, JSON.stringify({ at: new Date().toISOString(), ...entry }) + '\n')
}

const freePort = () => new Promise((resolve, reject) => {
  const server = net.createServer().listen(0, '127.0.0.1', () => { const { port } = server.address(); server.close(() => resolve(port)) })
  server.on('error', reject)
})

const children = new Set()
function start(name, bin, env) {
  const log = openSync(`${DRILL_STATE}/${name}.log`, 'a')
  const child = spawn(bin, [], { env: { PATH: process.env.PATH, ...env }, stdio: ['ignore', log, log] })
  children.add(child)
  child.on('exit', () => children.delete(child))
  return child
}
async function stop(child) {
  if (child.exitCode !== null || child.signalCode !== null) return
  const exited = new Promise(resolve => child.once('exit', resolve))
  child.kill('SIGTERM')
  if (await Promise.race([exited.then(() => true), sleep(15000).then(() => false)])) return
  child.kill('SIGKILL')
  await exited
}

// A scoped relay token as Cloud issues it (HS256 over the hosted claims).
function relayToken(secret, claims) {
  const now = Math.floor(Date.now() / 1000)
  const body = { iss: ISSUER, realm_id: REALM, iat: now, exp: now + 3600, jti: randomBytes(8).toString('hex'), ...claims }
  const input = `${b64url('{"alg":"HS256","typ":"JWT"}')}.${b64url(JSON.stringify(body))}`
  return `${input}.${createHmac('sha256', secret).update(input).digest('base64url')}`
}

function open(url) {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(url)
    let challenge
    socket.once('upgrade', response => { challenge = response.headers['x-chariox-registration-challenge'] })
    socket.once('open', () => resolve({ socket, challenge }))
    socket.once('error', reject)
  })
}
// The next envelope of one of `kinds`, or the reason the relay closed the connection with.
function next(socket, kinds, ms = 5000) {
  return new Promise(resolve => {
    const done = value => { clearTimeout(timer); socket.off('message', onMessage); socket.off('close', onClose); resolve(value) }
    const onMessage = data => { const envelope = JSON.parse(String(data)); if (kinds.includes(envelope.kind)) done(envelope) }
    const onClose = () => done({ kind: 'socket_closed' })
    const timer = setTimeout(() => done({ kind: 'no_answer' }), ms)
    socket.on('message', onMessage); socket.once('close', onClose)
  })
}

// The public key a client reaches through the relay for the kernel, or null while the kernel is not registered.
async function registeredKey(url, clientToken) {
  const { socket } = await open(url)
  socket.send(JSON.stringify({ kind: 'client_connect', auth_token: clientToken, target: { daemon_id: KERNEL_ID } }))
  const answer = await next(socket, ['client_connected', 'close'])
  socket.close()
  return answer.kind === 'client_connected' ? answer.daemon_public_key : null
}
async function waitForKey(url, clientToken, ms = 90000) {
  const deadline = Date.now() + ms
  for (;;) {
    const key = await registeredKey(url, clientToken).catch(() => null)
    if (key || Date.now() > deadline) return key
    await sleep(500)
  }
}

// What the relay answers a registration signed by `privateKey` for the challenge of this connection.
async function impersonate(url, registration, privateKey) {
  const { socket, challenge } = await open(url)
  const message = JSON.stringify(['chariox-relay-registration-v1', new URL(url).host.toLowerCase(), registration.daemon_id, sha256hex(registration.auth_token), challenge])
  const proof = sign('sha256', Buffer.from(message), { key: privateKey, dsaEncoding: 'ieee-p1363' }).toString('base64')
  socket.send(JSON.stringify({ kind: 'daemon_register', registration, proof }))
  const answer = await next(socket, ['close'])
  socket.close()
  return answer.reason ?? answer.kind
}

async function relay(port, env) {
  const child = start('relay', RELAY_BIN, { CHARIOX_RELAY_HOST: '127.0.0.1', CHARIOX_RELAY_PORT: String(port), CHARIOX_RELAY_STUN_ADDRESS: 'off', ...env })
  for (let i = 0; i < 100; i++) {
    if (await fetch(`http://127.0.0.1:${port}/healthz`).then(r => r.ok, () => false)) return child
    await sleep(100)
  }
  throw new Error(`relay did not start; see ${DRILL_STATE}/relay.log`)
}
// The kernel's identity belongs to its local port, so every start uses the same one.
async function kernel(port, relayUrl, relayTokenValue) {
  return start('kernel', KERNEL_BIN, {
    HOME: `${DRILL_STATE}/home`, TMPDIR: `${DRILL_STATE}/tmp`, CHARIOX_HOME: `${DRILL_STATE}/chariox`,
    CHARIOX_KERNEL_PORT: String(port), CHARIOX_DAEMON_ID: KERNEL_ID,
    CHARIOX_RELAY_URL: relayUrl, CHARIOX_RELAY_TOKEN: relayTokenValue,
  })
}

for (const dir of ['home', 'tmp', 'chariox']) mkdirSync(`${DRILL_STATE}/${dir}`, { recursive: true, mode: 0o700 })
mkdirSync(DRILL_EVIDENCE, { recursive: true })
const results = []
const check = (step, ok, detail) => { results.push(ok); record({ step, ok, ...detail }) }
try {
  const port = await freePort(), kernelPort = await freePort()
  const url = `ws://127.0.0.1:${port}`

  // Learn the kernel's public key the way any client does, so Cloud's token can be bound to it.
  const shared = randomBytes(24).toString('hex')
  let relayProcess = await relay(port, { CHARIOX_RELAY_TOKEN: shared })
  let kernelProcess = await kernel(kernelPort, url, shared)
  const kernelKey = await waitForKey(url, shared)
  check('kernel_registers_with_shared_token', Boolean(kernelKey), {})
  assert.ok(kernelKey, `kernel never registered; see ${DRILL_STATE}/kernel.log`)
  await stop(kernelProcess); await stop(relayProcess)

  // Hosted relay: the kernel's token is bound to its key, as Cloud issues it.
  const secret = randomBytes(32).toString('hex')
  const scoped = { CHARIOX_RELAY_SCOPED_ISSUER: ISSUER, CHARIOX_RELAY_SCOPED_HMAC_SECRET: secret }
  const kernelToken = relayToken(secret, {
    sub: KERNEL_ID, subject_kind: 'kernel', allowed_actions: ['daemon.register', 'daemon.heartbeat'],
    public_key_thumbprint: sha256hex(kernelKey),
  })
  const clientToken = relayToken(secret, {
    sub: 'drill-client', subject_kind: 'client', allowed_actions: ['client.connect'], allowed_targets: [KERNEL_ID],
  })
  relayProcess = await relay(port, scoped)
  kernelProcess = await kernel(kernelPort, url, kernelToken)
  check('kernel_registers_with_proof', await waitForKey(url, clientToken) === kernelKey, {})

  // The impersonator holds the leaked token and the kernel's public key, and signs with its own key.
  const impersonatorKey = generateKeyPairSync('ec', { namedCurve: 'prime256v1' })
  const ownPublicKey = impersonatorKey.publicKey.export({ format: 'der', type: 'spki' }).subarray(-65).toString('base64')
  const registration = {
    auth_token: kernelToken, daemon_id: KERNEL_ID, machine_id: 'impersonator-machine', os_name: 'Linux',
    kernel_started_at_ms: Date.now(), public_key: kernelKey,
  }
  const asKernel = await impersonate(url, registration, impersonatorKey.privateKey)
  check('impersonator_with_kernel_public_key_refused',
    asKernel === 'registration proof is not signed by the kernel identity key for this connection', { reason: asKernel })
  const asItself = await impersonate(url, { ...registration, public_key: ownPublicKey }, impersonatorKey.privateKey)
  check('impersonator_with_own_key_refused', asItself === 'relay token key does not match daemon registration', { reason: asItself })
  check('kernel_stays_registered', await registeredKey(url, clientToken) === kernelKey && kernelProcess.exitCode === null, {})

  // The real kernel reconnects normally: a restarted relay offers a new challenge and the kernel signs it.
  await stop(relayProcess)
  relayProcess = await relay(port, scoped)
  check('kernel_registers_again_after_relay_restart', await waitForKey(url, clientToken) === kernelKey, {})
  await stop(kernelProcess); await stop(relayProcess)
} finally {
  for (const child of children) await stop(child)
}
const passed = results.length === 6 && results.every(Boolean)
record({ step: 'verdict', ok: passed })
process.exit(passed ? 0 : 1)
