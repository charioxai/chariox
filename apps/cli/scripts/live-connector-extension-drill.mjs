#!/usr/bin/env node
import http from 'node:http'
import { spawn } from 'node:child_process'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function log(message, details) {
  if (details === undefined) console.log(`[connector-extension-drill] ${message}`)
  else console.log(`[connector-extension-drill] ${message}`, JSON.stringify(details))
}

async function run(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('error', reject)
    child.on('close', (code, signal) => resolve({ code, signal, stdout, stderr }))
  })
}

async function buildKernel() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return binary
}

function startDaemon(binary, env) {
  const child = spawn(binary, [], { cwd: repoRoot, env, stdio: ['ignore', 'pipe', 'pipe'] })
  child.stdoutText = ''
  child.stderrText = ''
  child.stdout.on('data', (chunk) => { child.stdoutText += chunk.toString() })
  child.stderr.on('data', (chunk) => { child.stderrText += chunk.toString() })
  return child
}

async function stopDaemon(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  child.kill('SIGTERM')
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(4000)])
  if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
}

async function waitForDaemon(kernelUrl) {
  const deadline = Date.now() + 25_000
  let lastError = null
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(requests.listSessionsRequest())
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`daemon did not become ready: ${lastError?.message ?? String(lastError)}`)
}

function unwrap(response, key) {
  if (!response?.[key]) throw new Error(`expected ${key}, got ${JSON.stringify(response)}`)
  return response[key]
}

async function expectReject(label, fn, includes) {
  try {
    await fn()
  } catch (error) {
    const message = String(error?.message ?? error)
    if (includes && !message.includes(includes)) {
      throw new Error(`${label} failed with wrong error: ${message}`)
    }
    return
  }
  throw new Error(`${label} unexpectedly succeeded`)
}

function startApiServer(expectedSecret) {
  const seen = []
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, 'http://127.0.0.1')
    seen.push({ method: req.method, pathname: url.pathname, secret: url.searchParams.get('key') })
    res.setHeader('content-type', 'application/json')
    if (url.pathname === '/public') {
      res.end(JSON.stringify({ route: 'public', echo: url.searchParams.get('q') }))
      return
    }
    if (url.pathname === '/secret') {
      res.end(JSON.stringify({ route: 'secret', authorized: url.searchParams.get('key') === expectedSecret }))
      return
    }
    if (url.pathname === '/write') {
      res.end(JSON.stringify({ route: 'write', method: req.method }))
      return
    }
    if (url.pathname === '/destroy') {
      res.end(JSON.stringify({ route: 'destroy', method: req.method }))
      return
    }
    if (url.pathname === '/large') {
      res.end(JSON.stringify({ route: 'large', payload: 'x'.repeat(128) }))
      return
    }
    res.statusCode = 404
    res.end(JSON.stringify({ error: 'not_found' }))
  })
  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      resolve({ server, port: server.address().port, seen })
    })
  })
}

async function main() {
  const root = path.join(repoRoot, 'target', 'live-connector-extension-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(root, 'workspace')
  const home = path.join(root, 'home')
  const arrobaHome = path.join(root, 'arroba-home')
  const configHome = path.join(root, 'config')
  const stateHome = path.join(root, 'state')
  const kernelPort = 50000 + Math.floor(Math.random() * 1000)
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const secretValue = `connector-secret-${process.pid}`
  const vaultKey = `connector-drill-${process.pid}-${Date.now()}`
  const env = {
    ...process.env,
    ARROBA_HOME: arrobaHome,
    XDG_CONFIG_HOME: configHome,
    XDG_STATE_HOME: stateHome,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(kernelPort + 1000),
    ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
    ARROBA_CODEX_PORT: String(kernelPort + 2001),
    ARROBA_DAEMON_ID: `connector-extension-drill-${process.pid}-${Date.now()}`,
    ARROBA_DAEMON_SOCKET: path.join(root, 'daemon.sock'),
  }

  let daemon = null
  let client = null
  let api = null
  let succeeded = false
  try {
    await mkdir(workspace, { recursive: true })
    await mkdir(path.join(configHome, 'arroba'), { recursive: true })
    await writeFile(path.join(configHome, 'arroba', 'config.toml'), 'version = 1\n', 'utf8')
    api = await startApiServer(secretValue)
    const credentialPath = path.join(root, 'credential.yaml')
    const connectorPath = path.join(root, 'connector.yaml')
    const wrongHostConnectorPath = path.join(root, 'wrong-host.yaml')
    const cappedConnectorPath = path.join(root, 'capped.yaml')
    await writeFile(credentialPath, `
id: local-api
description: Local API drill key
source:
  type: vault
  key: ${vaultKey}
allowed_hosts:
  - 127.0.0.1:${api.port}
allowed_uses:
  - http
injection:
  kind: query
  name: key
`, 'utf8')
    await writeFile(connectorPath, `
kind: connector
name: local_api
description: Local HTTP API connector drill.
type: http
base_url: http://127.0.0.1:${api.port}
credential:
  required: false
timeout_ms: 30000
max_response_bytes: 1048576
operations:
  - name: public_echo
    description: Read a public echo route.
    safety: read
    input_schema:
      type: object
      required: [q]
      properties:
        q: { type: string }
      additionalProperties: false
    request:
      method: GET
      path: /public
      query:
        q: "{{q}}"
  - name: secret_status
    description: Read a route requiring the vault-backed key.
    safety: read
    input_schema:
      type: object
      properties: {}
      additionalProperties: false
    request:
      method: GET
      path: /secret
  - name: write_item
    description: Write a deterministic item.
    safety: write
    input_schema:
      type: object
      properties: {}
      additionalProperties: false
    request:
      method: POST
      path: /write
  - name: destroy_item
    description: Destructive deterministic operation.
    safety: destructive
    input_schema:
      type: object
      properties: {}
      additionalProperties: false
    request:
      method: DELETE
      path: /destroy
`, 'utf8')
    await writeFile(wrongHostConnectorPath, `
kind: connector
name: wrong_host
description: Wrong host connector drill.
type: http
base_url: http://localhost:${api.port}
credential:
  required: true
timeout_ms: 30000
max_response_bytes: 1048576
operations:
  - name: secret_status
    description: Read a route requiring the vault-backed key.
    safety: read
    input_schema:
      type: object
      properties: {}
      additionalProperties: false
    request:
      method: GET
      path: /secret
`, 'utf8')
    await writeFile(cappedConnectorPath, `
kind: connector
name: capped_api
description: Response cap connector drill.
type: http
base_url: http://127.0.0.1:${api.port}
timeout_ms: 30000
max_response_bytes: 8
operations:
  - name: large_response
    description: Return a response that exceeds the configured cap.
    safety: read
    input_schema:
      type: object
      properties: {}
      additionalProperties: false
    request:
      method: GET
      path: /large
`, 'utf8')

    const kernelBinary = await buildKernel()
    daemon = startDaemon(kernelBinary, env)
    await waitForDaemon(kernelUrl)
    client = new LocalIpcClient(kernelUrl)

    await client.send(requests.setCredentialSecretRequest(vaultKey, secretValue))
    const credential = unwrap(await client.send(requests.registerCredentialRequest(credentialPath)), 'CredentialRegistered').credential
    const connector = unwrap(await client.send(requests.registerConnectorRequest(connectorPath)), 'ConnectorRegistered').connector
    await client.send(requests.registerConnectorRequest(wrongHostConnectorPath))
    await client.send(requests.registerConnectorRequest(cappedConnectorPath))
    if (credential.id !== 'local-api' || connector.name !== 'local_api') throw new Error('registration returned wrong entries')
    const listedCredentials = unwrap(await client.send(requests.listCredentialsRequest()), 'CredentialsListed').credentials
    const listedConnectors = unwrap(await client.send(requests.listConnectorsRequest()), 'ConnectorsListed').connectors
    if (!listedCredentials.some((entry) => entry.id === 'local-api')) throw new Error('registered credential missing from list')
    if (!listedConnectors.some((entry) => entry.name === 'local_api')) throw new Error('registered connector missing from list')

    const publicResult = unwrap(await client.send(requests.testConnectorRequest('local_api', 'public_echo', { q: 'alpha' })), 'ConnectorTested').execution
    if (publicResult.response.body_json.echo !== 'alpha') throw new Error(`public connector result mismatch: ${JSON.stringify(publicResult)}`)
    const secretResult = unwrap(await client.send(requests.testConnectorRequest('local_api', 'secret_status', {}, 'local-api')), 'ConnectorTested').execution
    if (secretResult.response.body_json.authorized !== true) throw new Error(`vault-backed connector was not authorized: ${JSON.stringify(secretResult)}`)
    if (JSON.stringify(secretResult).includes(secretValue)) throw new Error('connector result leaked the vault secret')
    if (!api.seen.some((entry) => entry.pathname === '/secret' && entry.secret === secretValue)) throw new Error('API server did not receive injected secret')

    await expectReject('wrong host credential policy', () => client.send(requests.testConnectorRequest('wrong_host', 'secret_status', {}, 'local-api')), 'not allowed for host')
    await expectReject('response cap enforcement', () => client.send(requests.testConnectorRequest('capped_api', 'large_response', {})), 'max_response_bytes')
    await expectReject('write blocked by read max safety', () => client.send(requests.testConnectorRequest('local_api', 'write_item', {}, null, 'read')), 'requires Write')
    const writeResult = unwrap(await client.send(requests.testConnectorRequest('local_api', 'write_item', {}, null, 'write')), 'ConnectorTested').execution
    if (writeResult.response.body_json.route !== 'write') throw new Error('write operation failed')
    await expectReject('destructive blocked by write max safety', () => client.send(requests.testConnectorRequest('local_api', 'destroy_item', {}, null, 'write')), 'requires Destructive')
    const destroyResult = unwrap(await client.send(requests.testConnectorRequest('local_api', 'destroy_item', {}, null, 'destructive')), 'ConnectorTested').execution
    if (destroyResult.response.body_json.route !== 'destroy') throw new Error('destructive operation failed')

    const session = unwrap(await client.send(requests.createSessionRequest(workspace, workspace, 'connector-extension-drill')), 'SessionCreated').session
    const agent = unwrap(await client.send(requests.spawnAgentRequest(session.id, 'dev-stub', 'connector-agent', 'connector-profile', workspace, 'low')), 'AgentSpawned').agent
    const granted = unwrap(
      await client.send(requests.grantAgentExtensionRequest(workspace, agent.id, 'connector', 'local_api', null, {
        credential: 'local-api',
        maxSafety: 'write',
      })),
      'AgentExtensionGranted',
    ).agent
    const grant = granted.extension_grants.find((entry) => entry.kind === 'connector' && entry.name === 'local_api')
    if (!grant || grant.credential !== 'local-api' || grant.max_safety !== 'write') throw new Error(`connector grant missing metadata: ${JSON.stringify(granted.extension_grants)}`)

    await client.send(requests.removeConnectorRequest('wrong_host'))
    await client.send(requests.removeConnectorRequest('local_api'))
    await client.send(requests.removeCredentialRequest('local-api'))
    await client.send(requests.deleteCredentialSecretRequest(vaultKey))

    succeeded = true
    log('pass', { workspace, connector: connector.name, credential: credential.id })
  } finally {
    await client?.close?.().catch(() => {})
    await stopDaemon(daemon)
    if (api) await new Promise((resolve) => api.server.close(resolve))
    if (succeeded) await rm(root, { recursive: true, force: true })
    else log('artifacts-kept', { root, daemonStdout: daemon?.stdoutText?.slice(-2000), daemonStderr: daemon?.stderrText?.slice(-2000) })
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
