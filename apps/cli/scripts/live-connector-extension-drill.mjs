#!/usr/bin/env node
import http from 'node:http'
import { spawn } from 'node:child_process'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function log(message, details) {
  if (details === undefined) console.log(`[connector-extension-drill] ${message}`)
  else console.log(`[connector-extension-drill] ${message}`, JSON.stringify(details))
}

function parseArgs(argv) {
  const options = { keepArtifactsOnFailure: false }
  for (const arg of argv) {
    if (arg === '--') continue
    if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help' || arg === '-h') {
      console.log('Usage: node apps/cli/scripts/live-connector-extension-drill.mjs [--keep-artifacts-on-failure]')
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  return options
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

async function buildHttpAdapter() {
  const binary = path.join(repoRoot, 'adapters/rust/target/debug/arroba-adapter-http')
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'adapters/rust/Cargo.toml'), '--bin', 'arroba-adapter-http'])
  if (result.code !== 0) throw new Error(`HTTP adapter build failed\n${result.stdout}\n${result.stderr}`)
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
  const options = parseArgs(process.argv.slice(2))
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
    ARROBA_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT: '1',
  }

  let daemon = null
  let client = null
  let api = null
  let succeeded = false
  let failure = null
  const completedChecks = []
  let sessionId = null
  let agentId = null
  let connectorName = null
  let credentialId = null
  let httpAdapterBinary = null
  try {
    await prepareDrillArtifacts(root)
    await mkdir(workspace, { recursive: true })
    await mkdir(path.join(configHome, 'arroba'), { recursive: true })
    await writeFile(path.join(configHome, 'arroba', 'config.toml'), [
      'version = 1',
      '',
      '[credential_vault]',
      'backend = "process_memory"',
      `service = "connector-extension-drill-${process.pid}"`,
      '',
    ].join('\n'), 'utf8')
    api = await startApiServer(secretValue)
    const credentialPath = path.join(root, 'credential.yaml')
    const adapterDir = path.join(root, 'http-adapter')
    const adapterPath = path.join(adapterDir, 'adapter.yaml')
    const pyAdapterDir = path.join(root, 'python-adapter')
    const pyAdapterPath = path.join(pyAdapterDir, 'adapter.yaml')
    const tsAdapterDir = path.join(root, 'typescript-adapter')
    const tsAdapterPath = path.join(tsAdapterDir, 'adapter.yaml')
    const connectorPath = path.join(root, 'connector.yaml')
    const pyConnectorPath = path.join(root, 'python-connector.yaml')
    const tsConnectorPath = path.join(root, 'typescript-connector.yaml')
    const wrongHostConnectorPath = path.join(root, 'wrong-host.yaml')
    const cappedConnectorPath = path.join(root, 'capped.yaml')
    httpAdapterBinary = await buildHttpAdapter()
    await mkdir(adapterDir, { recursive: true })
    await writeFile(adapterPath, `
kind: connector_adapter
name: http
version: 0.1.0
adapter_protocol: arroba-connector-adapter-v2
command: ${httpAdapterBinary}
description: HTTP adapter drill build.
`, 'utf8')
    await mkdir(pyAdapterDir, { recursive: true })
    await writeFile(path.join(pyAdapterDir, 'adapter.py'), `#!/usr/bin/env python3
import json
import sys

counter = 0
for line in sys.stdin:
    if not line.strip():
        continue
    request = json.loads(line)
    if request.get("type") == "shutdown":
        break
    if request.get("type") == "prepare":
        config = request.get("config") or {}
        print(json.dumps({"id": request["id"], "ok": True, "result": {
            "credential_targets": [{"kind": "host", "host": config.get("target_host"), "port": config.get("target_port")}],
            "prepared_config": {
                "arguments": request.get("arguments") or {},
                "config": config,
            },
        }}), flush=True)
        continue
    counter += 1
    credential = request.get("credential") or {}
    prepared_config = request.get("config") or {}
    print(json.dumps({"id": request["id"], "ok": True, "result": {
        "language": "python",
        "call_count": counter,
        "operation": request.get("operation"),
        "arguments": prepared_config.get("arguments"),
        "config": prepared_config.get("config"),
        "credential_id": credential.get("id"),
        "has_secret": bool(credential.get("secret")),
    }}), flush=True)
`, 'utf8')
    await writeFile(pyAdapterPath, `
kind: connector_adapter
name: python_echo
version: 0.1.0
adapter_protocol: arroba-connector-adapter-v2
command: /usr/bin/python3
args:
  - ${path.join(pyAdapterDir, 'adapter.py')}
description: Python adapter drill.
`, 'utf8')
    await mkdir(tsAdapterDir, { recursive: true })
    await writeFile(tsAdapterPath, `
kind: connector_adapter
name: typescript_echo
version: 0.1.0
adapter_protocol: arroba-connector-adapter-v2
command: ${process.execPath}
args:
  - ${path.join(tsAdapterDir, 'adapter.mjs')}
description: TypeScript-style Node adapter drill.
`, 'utf8')
    await writeFile(path.join(tsAdapterDir, 'adapter.mjs'), `#!/usr/bin/env node
import readline from 'node:readline'
const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity })
let counter = 0
for await (const line of rl) {
  if (!line.trim()) continue
  const request = JSON.parse(line)
  if (request.type === 'shutdown') break
  if (request.type === 'prepare') {
    const config = request.config ?? {}
    process.stdout.write(JSON.stringify({ id: request.id, ok: true, result: {
      credential_targets: [{ kind: 'host', host: config.target_host, port: config.target_port }],
      prepared_config: {
        arguments: request.arguments ?? {},
        config,
      },
    } }) + '\\n')
    continue
  }
  counter += 1
  const preparedConfig = request.config ?? {}
  process.stdout.write(JSON.stringify({ id: request.id, ok: true, result: {
    language: 'typescript',
    call_count: counter,
    operation: request.operation,
    arguments: preparedConfig.arguments,
    config: preparedConfig.config,
    credential_id: request.credential?.id ?? null,
    has_secret: Boolean(request.credential?.secret),
  } }) + '\\n')
}
`, 'utf8')
    await writeFile(credentialPath, `
id: local-api
description: Local API drill key
source:
  type: vault
  key: ${vaultKey}
allowed_hosts:
  - 127.0.0.1:${api.port}
allowed_uses:
  - connector
injection:
  kind: query
  name: key
`, 'utf8')
    await writeFile(connectorPath, `
kind: connector
name: local_api
description: Local HTTP API connector drill.
adapter: http
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
    config:
      base_url: http://127.0.0.1:${api.port}
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
    config:
      base_url: http://127.0.0.1:${api.port}
      method: GET
      path: /secret
  - name: write_item
    description: Write a deterministic item.
    safety: write
    input_schema:
      type: object
      properties: {}
      additionalProperties: false
    config:
      base_url: http://127.0.0.1:${api.port}
      method: POST
      path: /write
  - name: destroy_item
    description: Destructive deterministic operation.
    safety: destructive
    input_schema:
      type: object
      properties: {}
      additionalProperties: false
    config:
      base_url: http://127.0.0.1:${api.port}
      method: DELETE
      path: /destroy
`, 'utf8')
    await writeFile(wrongHostConnectorPath, `
kind: connector
name: wrong_host
description: Wrong host connector drill.
adapter: http
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
    config:
      base_url: http://localhost:${api.port}
      method: GET
      path: /secret
`, 'utf8')
    await writeFile(cappedConnectorPath, `
kind: connector
name: capped_api
description: Response cap connector drill.
adapter: http
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
    config:
      base_url: http://127.0.0.1:${api.port}
      method: GET
      path: /large
`, 'utf8')
    await writeFile(pyConnectorPath, `
kind: connector
name: python_echo_connector
description: Python adapter connector drill.
adapter: python_echo
credential:
  required: true
timeout_ms: 30000
max_response_bytes: 1048576
operations:
  - name: inspect
    description: Inspect Python adapter inputs.
    safety: read
    input_schema:
      type: object
      required: [value]
      properties:
        value: { type: string }
      additionalProperties: false
    config:
      adapter_kind: python
      target_host: 127.0.0.1
      target_port: ${api.port}
`, 'utf8')
    await writeFile(tsConnectorPath, `
kind: connector
name: typescript_echo_connector
description: TypeScript adapter connector drill.
adapter: typescript_echo
credential:
  required: true
timeout_ms: 30000
max_response_bytes: 1048576
operations:
  - name: inspect
    description: Inspect TypeScript adapter inputs.
    safety: read
    input_schema:
      type: object
      required: [value]
      properties:
        value: { type: string }
      additionalProperties: false
    config:
      adapter_kind: typescript
      target_host: 127.0.0.1
      target_port: ${api.port}
`, 'utf8')

    const kernelBinary = await buildKernel()
    daemon = startDaemon(kernelBinary, env)
    await waitForDaemon(kernelUrl)
    client = new LocalIpcClient(kernelUrl)

    await client.send(requests.setCredentialSecretRequest(vaultKey, secretValue))
    const adapter = unwrap(await client.send(requests.registerConnectorAdapterRequest(adapterPath)), 'ConnectorAdapterRegistered').adapter
    const pyAdapter = unwrap(await client.send(requests.registerConnectorAdapterRequest(pyAdapterPath)), 'ConnectorAdapterRegistered').adapter
    const tsAdapter = unwrap(await client.send(requests.registerConnectorAdapterRequest(tsAdapterPath)), 'ConnectorAdapterRegistered').adapter
    const credential = unwrap(await client.send(requests.registerCredentialRequest(credentialPath)), 'CredentialRegistered').credential
    const connector = unwrap(await client.send(requests.registerConnectorRequest(connectorPath)), 'ConnectorRegistered').connector
    await client.send(requests.registerConnectorRequest(pyConnectorPath))
    await client.send(requests.registerConnectorRequest(tsConnectorPath))
    await client.send(requests.registerConnectorRequest(wrongHostConnectorPath))
    await client.send(requests.registerConnectorRequest(cappedConnectorPath))
    if (adapter.name !== 'http' || pyAdapter.name !== 'python_echo' || tsAdapter.name !== 'typescript_echo' || credential.id !== 'local-api' || connector.name !== 'local_api') throw new Error('registration returned wrong entries')
    connectorName = connector.name
    credentialId = credential.id
    const listedCredentials = unwrap(await client.send(requests.listCredentialsRequest()), 'CredentialsListed').credentials
    const listedConnectors = unwrap(await client.send(requests.listConnectorsRequest()), 'ConnectorsListed').connectors
    const listedAdapters = unwrap(await client.send(requests.listConnectorAdaptersRequest()), 'ConnectorAdaptersListed').adapters
    if (!listedCredentials.some((entry) => entry.id === 'local-api')) throw new Error('registered credential missing from list')
    if (!listedConnectors.some((entry) => entry.name === 'local_api')) throw new Error('registered connector missing from list')
    if (!listedAdapters.some((entry) => entry.name === 'http')) throw new Error('registered adapter missing from list')
    if (!listedAdapters.some((entry) => entry.name === 'python_echo')) throw new Error('registered Python adapter missing from list')
    if (!listedAdapters.some((entry) => entry.name === 'typescript_echo')) throw new Error('registered TypeScript adapter missing from list')
    completedChecks.push('registry entries listed')

    const publicResult = unwrap(await client.send(requests.testConnectorRequest('local_api', 'public_echo', { q: 'alpha' })), 'ConnectorTested').execution
    if (publicResult.result.body_json.echo !== 'alpha') throw new Error(`public connector result mismatch: ${JSON.stringify(publicResult)}`)
    const secretResult = unwrap(await client.send(requests.testConnectorRequest('local_api', 'secret_status', {}, 'local-api')), 'ConnectorTested').execution
    if (secretResult.result.body_json.authorized !== true) throw new Error(`vault-backed connector was not authorized: ${JSON.stringify(secretResult)}`)
    if (JSON.stringify(secretResult).includes(secretValue)) throw new Error('connector result leaked the vault secret')
    if (!api.seen.some((entry) => entry.pathname === '/secret' && entry.secret === secretValue)) throw new Error('API server did not receive injected secret')
    const pyResult = unwrap(await client.send(requests.testConnectorRequest('python_echo_connector', 'inspect', { value: 'py-alpha' }, 'local-api')), 'ConnectorTested').execution
    if (pyResult.result.language !== 'python' || pyResult.result.arguments.value !== 'py-alpha' || pyResult.result.has_secret !== true) throw new Error(`Python adapter result mismatch: ${JSON.stringify(pyResult)}`)
    const tsResult = unwrap(await client.send(requests.testConnectorRequest('typescript_echo_connector', 'inspect', { value: 'ts-alpha' }, 'local-api')), 'ConnectorTested').execution
    if (tsResult.result.language !== 'typescript' || tsResult.result.arguments.value !== 'ts-alpha' || tsResult.result.has_secret !== true) throw new Error(`TypeScript adapter result mismatch: ${JSON.stringify(tsResult)}`)
    completedChecks.push('connector calls executed through http python and typescript adapters')

    await expectReject('wrong host credential policy', () => client.send(requests.testConnectorRequest('wrong_host', 'secret_status', {}, 'local-api')), 'not allowed for adapter-declared target')
    await expectReject('response cap enforcement', () => client.send(requests.testConnectorRequest('capped_api', 'large_response', {})), 'exceeded')
    await expectReject('write blocked by read max safety', () => client.send(requests.testConnectorRequest('local_api', 'write_item', {}, null, 'read')), 'requires Write')
    const writeResult = unwrap(await client.send(requests.testConnectorRequest('local_api', 'write_item', {}, null, 'write')), 'ConnectorTested').execution
    if (writeResult.result.body_json.route !== 'write') throw new Error('write operation failed')
    await expectReject('destructive blocked by write max safety', () => client.send(requests.testConnectorRequest('local_api', 'destroy_item', {}, null, 'write')), 'requires Destructive')
    const destroyResult = unwrap(await client.send(requests.testConnectorRequest('local_api', 'destroy_item', {}, null, 'destructive')), 'ConnectorTested').execution
    if (destroyResult.result.body_json.route !== 'destroy') throw new Error('destructive operation failed')
    completedChecks.push('safety and credential denials enforced')

    const session = unwrap(await client.send(requests.createSessionRequest(workspace, workspace, 'connector-extension-drill')), 'SessionCreated').session
    sessionId = session.id
    const agent = unwrap(await client.send(requests.spawnAgentRequest(session.id, 'dev-stub', 'connector-agent', 'connector-profile', workspace, 'low')), 'AgentSpawned').agent
    agentId = agent.id
    const granted = unwrap(
      await client.send(requests.grantAgentExtensionRequest(workspace, agent.id, 'connector', 'local_api', null, {
        credential: 'local-api',
        maxSafety: 'write',
      })),
      'AgentExtensionGranted',
    ).agent
    const grant = granted.extension_grants.find((entry) => entry.kind === 'connector' && entry.name === 'local_api')
    if (!grant || grant.credential !== 'local-api' || grant.max_safety !== 'write') throw new Error(`connector grant missing metadata: ${JSON.stringify(granted.extension_grants)}`)
    completedChecks.push('connector grant preserved credential and max safety metadata')

    await client.send(requests.removeConnectorRequest('wrong_host'))
    await client.send(requests.removeConnectorRequest('python_echo_connector'))
    await client.send(requests.removeConnectorRequest('typescript_echo_connector'))
    await client.send(requests.removeConnectorRequest('local_api'))
    await client.send(requests.removeConnectorAdapterRequest('http'))
    await client.send(requests.removeConnectorAdapterRequest('python_echo'))
    await client.send(requests.removeConnectorAdapterRequest('typescript_echo'))
    await client.send(requests.removeCredentialRequest('local-api'))
    await client.send(requests.deleteCredentialSecretRequest(vaultKey))

    succeeded = true
    log('pass', { workspace, connector: connector.name, credential: credential.id })
  } catch (error) {
    failure = error
    throw error
  } finally {
    await client?.close?.().catch(() => {})
    await stopDaemon(daemon)
    if (api) await new Promise((resolve) => api.server.close(resolve))
    await finalizeDrillArtifacts({
      rootDir: root,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: 'connector-extension',
        workspace,
        kernelUrl,
        connectorName,
        credentialId,
        sessionId,
        agentId,
        httpAdapterBinary,
        completedChecks,
        apiRequestCount: api?.seen?.length ?? null,
        daemonStdoutTail: daemon?.stdoutText?.slice(-2000) ?? '',
        daemonStderrTail: daemon?.stderrText?.slice(-2000) ?? '',
      },
      log,
    })
    if (!succeeded && options.keepArtifactsOnFailure) {
      log('artifacts-kept', { root })
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
