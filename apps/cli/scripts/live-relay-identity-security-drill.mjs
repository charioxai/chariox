#!/usr/bin/env node
import { createHmac } from 'node:crypto'
import net from 'node:net'
import { spawn } from 'node:child_process'
import { setTimeout as sleep } from 'node:timers/promises'
import WebSocket from 'ws'

const repoRoot = new URL('../../..', import.meta.url).pathname
const issuer = 'arroba-cloud-drill'
const secret = 'relay-identity-drill-secret'

function base64url(input) {
  return Buffer.from(input).toString('base64url')
}

function signToken(claims) {
  const claimsPayload = base64url(JSON.stringify(claims))
  const signature = createHmac('sha256', secret).update(claimsPayload).digest('base64url')
  return `arroba-scoped-v1.${claimsPayload}.${signature}`
}

function claims({
  subject,
  subjectKind,
  realm,
  actions,
  targets = null,
  expiresAt = Date.now() + 60_000,
}) {
  return {
    issuer,
    subject,
    subject_kind: subjectKind,
    realm_id: realm,
    allowed_actions: actions,
    allowed_targets: targets,
    issued_at_ms: Date.now(),
    expires_at_ms: expiresAt,
    token_id: `${subject}-${Date.now()}`,
    account_id: 'account-drill',
    organization_id: null,
    device_id: subject,
    machine_id: subjectKind === 'kernel' || subjectKind === 'machine' ? subject : null,
    client_id: subjectKind === 'client' ? subject : null,
    public_key_thumbprint: `${subject}-thumbprint`,
    entitlements_version: 'drill',
  }
}

function daemonRegistration({ token, daemonId, machineId }) {
  return {
    kind: 'daemon_register',
    registration: {
      auth_token: token,
      daemon_id: daemonId,
      machine_id: machineId,
      machine_alias: machineId,
      os_name: 'drill-os',
      kernel_started_at_ms: Date.now(),
      daemon_alias: daemonId,
      kernel_alias: daemonId,
      public_key: `${daemonId}-public-key`,
      capabilities: ['kernel_ws'],
      available_providers: ['codex'],
      accepting_remote_leases: false,
      leased_agent_count: 0,
      local_session_count: 0,
    },
  }
}

async function freePort() {
  return await new Promise((resolve, reject) => {
    const server = net.createServer()
    server.listen(0, '127.0.0.1', () => {
      const port = server.address().port
      server.close(() => resolve(port))
    })
    server.on('error', reject)
  })
}

async function waitForRelay(url, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      const ws = await connect(url)
      ws.close()
      return
    } catch {
      await sleep(100)
    }
  }
  throw new Error(`relay did not accept websocket connections at ${url}`)
}

async function connect(url) {
  return await new Promise((resolve, reject) => {
    const ws = new WebSocket(url)
    ws.once('open', () => resolve(ws))
    ws.once('error', reject)
  })
}

async function sendJson(ws, payload) {
  ws.send(JSON.stringify(payload))
}

async function nextJson(ws, label = 'relay message', timeoutMs = 3_000) {
  return await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`timed out waiting for ${label}`)), timeoutMs)
    ws.once('message', (data) => {
      clearTimeout(timer)
      resolve(JSON.parse(String(data)))
    })
    ws.once('close', () => {
      clearTimeout(timer)
      reject(new Error(`relay connection closed while waiting for ${label}`))
    })
  })
}

async function expectCloseAfterSend(url, payload, label) {
  const ws = await connect(url)
  const closed = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`${label} was not rejected`)), 3_000)
    ws.once('close', () => {
      clearTimeout(timer)
      resolve()
    })
    ws.once('message', (data) => {
      clearTimeout(timer)
      reject(new Error(`${label} unexpectedly received ${String(data)}`))
    })
  })
  await sendJson(ws, payload)
  await closed
}

function requireCondition(condition, message, detail = null) {
  if (!condition) {
    const suffix = detail == null ? '' : `\n${JSON.stringify(detail, null, 2)}`
    throw new Error(`${message}${suffix}`)
  }
}

async function main() {
  const port = await freePort()
  const url = `ws://127.0.0.1:${port}`
  const relay = spawn('cargo', ['run', '--manifest-path', `${repoRoot}/apps/relay/Cargo.toml`, '--bin', 'arroba-relay'], {
    cwd: repoRoot,
    env: {
      ...process.env,
      ARROBA_RELAY_HOST: '127.0.0.1',
      ARROBA_RELAY_PORT: String(port),
      ARROBA_RELAY_SCOPED_ISSUER: issuer,
      ARROBA_RELAY_SCOPED_HMAC_SECRET: secret,
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  let stderr = ''
  relay.stderr.on('data', (chunk) => { stderr += String(chunk) })
  relay.stdout.on('data', () => {})

  try {
    await waitForRelay(url)

    const daemonAToken = signToken(claims({
      subject: 'daemon-a',
      subjectKind: 'kernel',
      realm: 'realm-a',
      actions: ['daemon_register', 'daemon_heartbeat', 'peer_request', 'peer_event'],
    }))
    const daemonBToken = signToken(claims({
      subject: 'daemon-b',
      subjectKind: 'kernel',
      realm: 'realm-b',
      actions: ['daemon_register'],
    }))
    const clientAToken = signToken(claims({
      subject: 'client-a',
      subjectKind: 'client',
      realm: 'realm-a',
      actions: ['client_connect', 'client_metadata_read'],
    }))
    const clientBToken = signToken(claims({
      subject: 'client-b',
      subjectKind: 'client',
      realm: 'realm-b',
      actions: ['client_connect', 'client_metadata_read'],
    }))
    const expiredClientToken = signToken(claims({
      subject: 'client-revoked',
      subjectKind: 'client',
      realm: 'realm-a',
      actions: ['client_connect'],
      targets: ['daemon-a'],
      expiresAt: Date.now() - 1,
    }))

    const daemonA = await connect(url)
    await sendJson(daemonA, daemonRegistration({ token: daemonAToken, daemonId: 'daemon-a', machineId: 'machine-a' }))
    const daemonB = await connect(url)
    await sendJson(daemonB, daemonRegistration({ token: daemonBToken, daemonId: 'daemon-b', machineId: 'machine-b' }))
    await sleep(100)
    requireCondition(daemonA.readyState === WebSocket.OPEN, 'daemon A registration was rejected')
    requireCondition(daemonB.readyState === WebSocket.OPEN, 'daemon B registration was rejected')

    const clientA = await connect(url)
    const connectedAPromise = nextJson(clientA, 'client A connect')
    await sendJson(clientA, {
      kind: 'client_connect',
      auth_token: clientAToken,
      target: { daemon_id: 'daemon-a', daemon_alias: null },
    })
    const connectedA = await connectedAPromise
    requireCondition(connectedA.kind === 'client_connected', 'valid paired client did not connect', connectedA)

    const metadataA = await connect(url)
    const metadataAPromise = nextJson(metadataA, 'realm A metadata')
    await sendJson(metadataA, {
      kind: 'client_metadata_request',
      request_id: 'metadata-a',
      auth_token: clientAToken,
      query: { kind: 'list_live_machines' },
    })
    const metadataResponse = await metadataAPromise
    requireCondition(metadataResponse.machines?.length === 1, 'realm A metadata leaked or missed machines', metadataResponse)
    requireCondition(metadataResponse.machines[0].machine_id === 'machine-a', 'realm A metadata returned the wrong machine', metadataResponse)

    await expectCloseAfterSend(url, {
      kind: 'daemon_register',
      registration: daemonRegistration({ token: 'invalid-machine-token', daemonId: 'daemon-x', machineId: 'machine-x' }).registration,
    }, 'unpaired machine registration')

    await expectCloseAfterSend(url, {
      kind: 'daemon_register',
      registration: daemonRegistration({ token: clientAToken, daemonId: 'daemon-client-token', machineId: 'machine-client-token' }).registration,
    }, 'client token daemon registration')

    await expectCloseAfterSend(url, {
      kind: 'client_connect',
      auth_token: 'invalid-client-token',
      target: { daemon_id: 'daemon-a', daemon_alias: null },
    }, 'unpaired client connect')

    await expectCloseAfterSend(url, {
      kind: 'client_metadata_request',
      request_id: 'daemon-token-metadata',
      auth_token: daemonAToken,
      query: { kind: 'list_live_machines' },
    }, 'daemon token metadata query')

    await expectCloseAfterSend(url, {
      kind: 'client_connect',
      auth_token: expiredClientToken,
      target: { daemon_id: 'daemon-a', daemon_alias: null },
    }, 'expired/revoked client connect')

    await expectCloseAfterSend(url, {
      kind: 'client_connect',
      auth_token: clientAToken,
      target: { daemon_id: 'daemon-b', daemon_alias: null },
    }, 'cross-realm client route')

    const clientB = await connect(url)
    const metadataBPromise = nextJson(clientB, 'realm B metadata')
    await sendJson(clientB, {
      kind: 'client_metadata_request',
      request_id: 'metadata-b',
      auth_token: clientBToken,
      query: { kind: 'list_live_machines' },
    })
    const metadataB = await metadataBPromise
    requireCondition(metadataB.machines?.length === 1, 'realm B metadata leaked or missed machines', metadataB)
    requireCondition(metadataB.machines[0].machine_id === 'machine-b', 'realm B metadata returned the wrong machine', metadataB)

    console.log('live relay identity security drill passed')
  } finally {
    relay.kill('SIGTERM')
    await sleep(200)
    if (relay.exitCode == null) relay.kill('SIGKILL')
    if (process.env.ARROBA_DEBUG_DRILL === '1' && stderr.trim()) {
      console.error(stderr)
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
