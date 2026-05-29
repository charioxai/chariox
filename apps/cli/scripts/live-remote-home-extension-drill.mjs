#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { access, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import http from 'node:http'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const realHomeDir = os.homedir()

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const {
  attachToSessionRequest,
  createSessionRequest,
  grantAgentExtensionRequest,
  launchProviderRunRequest,
  listRemoteMachinesRequest,
  registerConnectorAdapterRequest,
  registerConnectorRequest,
  registerEnvironmentRequest,
  registerScriptRequest,
  revokeAgentExtensionRequest,
  spawnAgentRequest,
} = await import('../../../packages/kernel-client/dist/ipc-requests.js')

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (response, key) => response?.[key] ?? response
const unwrapVariant = (response, ...keys) => keys.map((key) => response?.[key]).find((value) => value != null) ?? response

async function resolveBinary(binaryPath, manifestPath, binName) {
  try {
    await access(binaryPath)
    return binaryPath
  } catch {
    throw new Error(`missing built binary ${binaryPath}; run cargo build --manifest-path ${manifestPath} --bin ${binName} first`)
  }
}

async function resolveExecutable(command) {
  if (command.includes(path.sep)) {
    await access(command)
    return command
  }
  for (const dir of (process.env.PATH ?? '').split(path.delimiter)) {
    if (!dir) continue
    const candidate = path.join(dir, command)
    try {
      await access(candidate)
      return candidate
    } catch {}
  }
  throw new Error(`executable ${command} not found on PATH`)
}

async function waitForTcpPort(port, host = '127.0.0.1', timeoutMs = 15_000) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const connected = await new Promise((resolve) => {
      const socket = net.connect({ host, port })
      socket.once('connect', () => {
        socket.destroy()
        resolve(true)
      })
      socket.once('error', () => {
        socket.destroy()
        resolve(false)
      })
    })
    if (connected) return
    await sleep(100)
  }
  throw new Error(`TCP listener ${host}:${port} did not become reachable`)
}

async function waitForDaemon(kernelUrl) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(listRemoteMachinesRequest())
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

async function waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(relayUrl, {
      relayAuthToken: relayToken,
      targetDaemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      await Promise.race([
        client.send(listRemoteMachinesRequest()),
        sleep(2_000).then(() => { throw new Error('probe timeout') }),
      ])
      await client.close().catch(() => {})
      return
    } catch {
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`relay target ${targetDaemonAlias} did not become reachable`)
}

async function waitForRemoteMachine(client, machineRef) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const listed = unwrapVariant(await client.send(listRemoteMachinesRequest()), 'RemoteMachinesListed')
    const machines = listed.machines ?? listed.remote_machines ?? []
    if (machines.some((machine) => (
      machine.machine_id === machineRef
      || machine.machine_alias === machineRef
      || machine.alias === machineRef
      || machine.display_name === machineRef
    ))) return
    await sleep(250)
  }
  throw new Error(`remote machine ${machineRef} did not become visible`)
}

async function terminateChild(child) {
  if (!child || child.exitCode != null) return
  child.kill('SIGTERM')
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(5_000)])
  if (child.exitCode == null) child.kill('SIGKILL')
}

function daemonEnv({ rootDir, relayUrl, relayToken, daemonId, daemonAlias, machineId, machineAlias, kernelPort, mcpPort, acceptRemoteLeases, capabilityRoot, socketName }) {
  return {
    ...process.env,
    HOME: path.join(rootDir, `${daemonAlias}-home`),
    CODEX_HOME: process.env.CODEX_HOME ?? path.join(realHomeDir, '.codex'),
    OPENCODE_CONFIG_DIR: process.env.OPENCODE_CONFIG_DIR ?? path.join(realHomeDir, '.config', 'opencode'),
    XDG_CONFIG_HOME: path.join(rootDir, `${daemonAlias}-config`),
    XDG_STATE_HOME: path.join(rootDir, `${daemonAlias}-state`),
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(mcpPort),
    ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
    ARROBA_CODEX_PORT: String(kernelPort + 2001),
    ARROBA_RELAY_URL: relayUrl,
    ARROBA_RELAY_TOKEN: relayToken,
    ARROBA_DAEMON_ID: daemonId,
    ARROBA_DAEMON_ALIAS: daemonAlias,
    ARROBA_MACHINE_ID: machineId,
    ARROBA_MACHINE_ALIAS: machineAlias,
    ARROBA_ACCEPT_REMOTE_LEASES: acceptRemoteLeases ? '1' : '0',
    ARROBA_DAEMON_SOCKET: path.join(rootDir, socketName),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, `${daemonAlias}-history`),
    ARROBA_CAPABILITY_ISOLATION_ROOT: capabilityRoot,
  }
}

async function callRuntimeMcp(serverUrl, authToken, method, params = {}) {
  const response = await fetch(serverUrl, {
    method: 'POST',
    headers: {
      Authorization: `Bearer ${authToken}`,
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ jsonrpc: '2.0', id: `${Date.now()}`, method, params }),
  })
  const json = await response.json()
  if (json.error) throw new Error(`runtime MCP ${method} failed: ${JSON.stringify(json.error)}`)
  return json.result
}

async function expectRuntimeMcpReject(serverUrl, authToken, method, params = {}) {
  try {
    const result = await callRuntimeMcp(serverUrl, authToken, method, params)
    if (result?.isError) return result
    throw new Error(`runtime MCP ${method} unexpectedly succeeded: ${JSON.stringify(result)}`)
  } catch (error) {
    return { error: String(error?.message ?? error) }
  }
}

async function waitForRuntimeTool(serverUrl, authToken, name, present) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const tools = await callRuntimeMcp(serverUrl, authToken, 'tools/list')
    const found = tools.tools.some((tool) => tool.name === name)
    if (found === present) return tools
    await sleep(250)
  }
  throw new Error(`tool ${name} did not become ${present ? 'advertised' : 'revoked'}`)
}

async function main() {
  const python = await resolveExecutable(process.env.PYTHON ?? 'python3')
  const rootDir = path.join(os.tmpdir(), `arroba-remote-home-extension-${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const homeMarker = path.join(rootDir, 'home-script-marker.txt')
  const homeMcpMarker = path.join(rootDir, 'home-mcp-marker.txt')
  const homeConnectorMarker = path.join(rootDir, 'home-connector-marker.txt')
  const portsBase = 58100 + Math.floor(Math.random() * 500)
  const relayPort = portsBase
  const homeOnlyMcpPort = portsBase + 500
  const homeKernelPort = portsBase + 1000
  const workerKernelPort = portsBase + 1001
  const homeMcpPort = portsBase + 2000
  const workerMcpPort = portsBase + 2001
  const relayToken = `remote-home-extension-${process.pid}`
  const relayUrl = `ws://127.0.0.1:${relayPort}`
  const workerMachineId = `remote-home-extension-worker-machine-${process.pid}`
  const workerAlias = `remote-home-extension-worker-${process.pid}`

  let relay = null
  let home = null
  let worker = null
  let homeOnlyMcp = null
  let client = null
  let succeeded = false
  try {
    await mkdir(workspace, { recursive: true })
    const homeHomeDir = path.join(rootDir, 'home-home')
    const homeCapabilityRoot = path.join(rootDir, 'home-capabilities')
    const scriptPath = path.join(rootDir, 'home_only_lookup.py')
    await writeFile(scriptPath, `
MARKER = ${JSON.stringify(homeMarker)}

def run(query: str) -> dict[str, object]:
    """Return a deterministic home-only lookup result."""
    with open(MARKER, "w", encoding="utf-8") as handle:
        handle.write("HOME_SCRIPT_EXECUTED:" + query)
    return {"query": query, "origin": "home"}

def test_run() -> None:
    result = run("self-test")
    assert result["origin"] == "home"
`, 'utf8')
    const homeMcpDir = path.join(homeCapabilityRoot, 'user', 'mcps')
    await mkdir(homeMcpDir, { recursive: true })
    await writeFile(path.join(homeMcpDir, 'home_echo_mcp.json'), `${JSON.stringify({
      name: 'home_echo_mcp',
      transport: {
        type: 'streamable_http',
        url: `http://127.0.0.1:${homeOnlyMcpPort}/mcp`,
      },
      enabled: true,
      required: false,
      tool_timeout_sec: 10,
    }, null, 2)}\n`, 'utf8')
    homeOnlyMcp = http.createServer(async (req, res) => {
      let body = ''
      req.setEncoding('utf8')
      for await (const chunk of req) body += chunk
      const rpc = body ? JSON.parse(body) : {}
      res.setHeader('content-type', 'application/json')
      if (rpc.method === 'tools/list') {
        return res.end(JSON.stringify({
          jsonrpc: '2.0',
          id: rpc.id ?? null,
          result: {
            tools: [{
              name: 'home_echo',
              description: 'Home-only MCP echo tool.',
              inputSchema: {
                type: 'object',
                required: ['text'],
                properties: { text: { type: 'string' } },
                additionalProperties: false,
              },
            }],
          },
        }))
      }
      if (rpc.method === 'tools/call' && rpc.params?.name === 'home_echo') {
        const text = String(rpc.params?.arguments?.text ?? '')
        await writeFile(homeMcpMarker, `HOME_MCP_EXECUTED:${text}`, 'utf8')
        return res.end(JSON.stringify({
          jsonrpc: '2.0',
          id: rpc.id ?? null,
          result: {
            content: [{ type: 'text', text: JSON.stringify({ origin: 'home-mcp', text }) }],
          },
        }))
      }
      res.end(JSON.stringify({
        jsonrpc: '2.0',
        id: rpc.id ?? null,
        error: { code: -32601, message: `unsupported MCP method ${rpc.method}` },
      }))
    })
    await new Promise((resolve, reject) => {
      homeOnlyMcp.once('error', reject)
      homeOnlyMcp.listen(homeOnlyMcpPort, '127.0.0.1', resolve)
    })

    const relayBinary = await resolveBinary(path.join(repoRoot, 'apps/relay/target/debug/arroba-relay'), path.join(repoRoot, 'apps/relay/Cargo.toml'), 'arroba-relay')
    const daemonBinary = await resolveBinary(path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel'), path.join(repoRoot, 'apps/kernel/Cargo.toml'), 'arroba-kernel')
    relay = spawn(relayBinary, [], {
      cwd: repoRoot,
      env: { ...process.env, ARROBA_RELAY_HOST: '127.0.0.1', ARROBA_RELAY_PORT: String(relayPort), ARROBA_RELAY_TOKEN: relayToken },
      stdio: ['ignore', 'ignore', 'inherit'],
    })
    await waitForTcpPort(relayPort)

    home = spawn(daemonBinary, [], {
      cwd: repoRoot,
      env: daemonEnv({
        rootDir,
        relayUrl,
        relayToken,
        daemonId: `remote-home-extension-home-${process.pid}`,
        daemonAlias: 'home',
        machineId: `remote-home-extension-home-machine-${process.pid}`,
        machineAlias: `remote-home-extension-home-machine-${process.pid}`,
        kernelPort: homeKernelPort,
        mcpPort: homeMcpPort,
        acceptRemoteLeases: false,
        capabilityRoot: homeCapabilityRoot,
        socketName: 'home.sock',
      }),
      stdio: ['ignore', 'ignore', 'inherit'],
    })
    worker = spawn(daemonBinary, [], {
      cwd: repoRoot,
      env: daemonEnv({
        rootDir,
        relayUrl,
        relayToken,
        daemonId: `remote-home-extension-worker-${process.pid}`,
        daemonAlias: 'worker',
        machineId: workerMachineId,
        machineAlias: workerAlias,
        kernelPort: workerKernelPort,
        mcpPort: workerMcpPort,
        acceptRemoteLeases: true,
        capabilityRoot: path.join(rootDir, 'worker-capabilities'),
        socketName: 'worker.sock',
      }),
      stdio: ['ignore', 'ignore', 'inherit'],
    })
    const homeUrl = `ws://127.0.0.1:${homeKernelPort}`
    await waitForDaemon(homeUrl)
    await waitForDaemon(`ws://127.0.0.1:${workerKernelPort}`)
    await waitForRelayTarget(relayUrl, relayToken, 'home')
    await waitForRelayTarget(relayUrl, relayToken, 'worker')
    client = new LocalIpcClient(homeUrl)
    await waitForRemoteMachine(client, workerMachineId)

    const env = unwrap(await client.send(registerEnvironmentRequest(workspace, {
      name: 'home-python',
      runtime: { type: 'python', python },
    })), 'EnvironmentRegistered').environment
    await client.send(registerScriptRequest(workspace, scriptPath, env.name, 'home_only_lookup'))
    const connectorAdapterDir = path.join(rootDir, 'home-connector-adapter')
    await mkdir(connectorAdapterDir, { recursive: true })
    const connectorAdapterScript = path.join(connectorAdapterDir, 'home_connector_adapter.mjs')
    await writeFile(connectorAdapterScript, `
import { appendFileSync, writeFileSync } from 'node:fs'
import readline from 'node:readline'

const marker = process.argv[2]
const rl = readline.createInterface({ input: process.stdin })
rl.on('line', (line) => {
  const request = JSON.parse(line)
  if (request.type === 'shutdown') process.exit(0)
  if (request.type === 'validate') {
    console.log(JSON.stringify({ id: request.id, ok: true }))
    return
  }
  if (request.type === 'prepare') {
    console.log(JSON.stringify({ id: request.id, ok: true, result: { credential_targets: [], prepared_config: { arguments: request.arguments ?? {}, config: request.config ?? {} } } }))
    return
  }
  if (request.type === 'call') {
    const q = String(request.config?.arguments?.q ?? '')
    writeFileSync(marker, 'HOME_CONNECTOR_EXECUTED:' + q, 'utf8')
    console.log(JSON.stringify({ id: request.id, ok: true, result: { origin: 'home-connector', q } }))
    return
  }
  appendFileSync(marker + '.errors', 'unsupported request ' + request.type + '\\n')
  console.log(JSON.stringify({ id: request.id, ok: false, error: 'unsupported request ' + request.type }))
})
`, 'utf8')
    const connectorAdapterPath = path.join(connectorAdapterDir, 'adapter.yaml')
    const connectorPath = path.join(rootDir, 'home-local-api-connector.yaml')
    await writeFile(connectorAdapterPath, `
kind: connector_adapter
name: home_stub
version: 0.1.0
adapter_protocol: arroba-connector-adapter-v2
command: ${process.execPath}
args:
  - ${connectorAdapterScript}
  - ${homeConnectorMarker}
description: Home-only connector adapter for remote extension drill.
`, 'utf8')
    await writeFile(connectorPath, `
kind: connector
name: home_local_api
description: Home-only HTTP connector for remote extension drill.
adapter: home_stub
credential:
  required: false
timeout_ms: 30000
max_response_bytes: 1048576
operations:
  - name: public_echo
    description: Read home-only connector echo data.
    safety: read
    input_schema:
      type: object
      required: [q]
      properties:
        q: { type: string }
      additionalProperties: false
    config:
      marker: ${homeConnectorMarker}
`, 'utf8')
    await client.send(registerConnectorAdapterRequest(connectorAdapterPath))
    await client.send(registerConnectorRequest(connectorPath))
    const session = unwrap(await client.send(createSessionRequest(workspace, workspace, 'remote-home-extension-drill')), 'SessionCreated').session
    await client.send(attachToSessionRequest(session.id, `remote-home-extension-${process.pid}`))
    const agent = unwrap(await client.send(spawnAgentRequest(session.id, 'dev-stub', 'home-proxy-agent', 'default', workspace, 'low', undefined, undefined, workerAlias)), 'AgentSpawned').agent
    await client.send(grantAgentExtensionRequest(workspace, agent.id, 'script', 'home_only_lookup', env.name))
    await client.send(grantAgentExtensionRequest(workspace, agent.id, 'mcp', 'home_echo_mcp'))
    await client.send(grantAgentExtensionRequest(workspace, agent.id, 'connector', 'home_local_api', null, { maxSafety: 'read' }))
    const launch = unwrapVariant(await client.send(launchProviderRunRequest(
      session.id,
      'dev-stub',
      'default',
      'default',
      'low',
      agent.id,
      { nativeTui: true },
    )), 'ProviderRunLaunched', 'ProviderRunLaunchAccepted').provider_run
    if (!launch.runtime_mcp_server_url || !launch.runtime_mcp_auth_token) throw new Error(`launched run lacks runtime MCP binding: ${JSON.stringify(launch)}`)

    await waitForRuntimeTool(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, 'home_only_lookup', true)
    const call = await callRuntimeMcp(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, 'tools/call', {
      name: 'home_only_lookup',
      arguments: { query: 'remote-agent' },
    })
    if (call.isError) throw new Error(`home proxy script returned error: ${JSON.stringify(call)}`)
    const marker = await readFile(homeMarker, 'utf8')
    if (marker !== 'HOME_SCRIPT_EXECUTED:remote-agent') throw new Error(`home marker mismatch: ${JSON.stringify(marker)}`)
    const proxyUrl = launch.runtime_mcp_server_url.replace(/\/mcp\/?$/, '/mcp/proxy/home_echo_mcp')
    const mcpTools = await callRuntimeMcp(proxyUrl, launch.runtime_mcp_auth_token, 'tools/list')
    if (!mcpTools.tools.some((tool) => tool.name === 'home_echo')) throw new Error(`home MCP tool not listed: ${JSON.stringify(mcpTools)}`)
    const mcpCall = await callRuntimeMcp(proxyUrl, launch.runtime_mcp_auth_token, 'tools/call', {
      name: 'home_echo',
      arguments: { text: 'remote-mcp' },
    })
    if (mcpCall.isError) throw new Error(`home MCP returned error: ${JSON.stringify(mcpCall)}`)
    const mcpMarker = await readFile(homeMcpMarker, 'utf8')
    if (mcpMarker !== 'HOME_MCP_EXECUTED:remote-mcp') throw new Error(`home MCP marker mismatch: ${JSON.stringify(mcpMarker)}`)
    await waitForRuntimeTool(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, 'home_local_api_public_echo', true)
    const connectorCall = await callRuntimeMcp(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, 'tools/call', {
      name: 'home_local_api_public_echo',
      arguments: { q: 'remote-connector' },
    })
    if (connectorCall.isError) throw new Error(`home connector returned error: ${JSON.stringify(connectorCall)}`)
    const connectorMarker = await readFile(homeConnectorMarker, 'utf8')
    if (connectorMarker !== 'HOME_CONNECTOR_EXECUTED:remote-connector') throw new Error(`home connector marker mismatch: ${JSON.stringify(connectorMarker)}`)

    await client.send(revokeAgentExtensionRequest(agent.id, 'script', 'home_only_lookup'))
    await waitForRuntimeTool(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, 'home_only_lookup', false)
    await expectRuntimeMcpReject(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, 'tools/call', {
      name: 'home_only_lookup',
      arguments: { query: 'after-revoke' },
    })
    const afterRevokeMarker = await readFile(homeMarker, 'utf8')
    if (afterRevokeMarker !== marker) throw new Error('revoked home-proxy script executed after revoke')
    await client.send(revokeAgentExtensionRequest(agent.id, 'mcp', 'home_echo_mcp'))
    await expectRuntimeMcpReject(proxyUrl, launch.runtime_mcp_auth_token, 'tools/call', {
      name: 'home_echo',
      arguments: { text: 'after-mcp-revoke' },
    })
    const afterMcpRevokeMarker = await readFile(homeMcpMarker, 'utf8')
    if (afterMcpRevokeMarker !== mcpMarker) throw new Error('revoked home-proxy MCP executed after revoke')
    await client.send(revokeAgentExtensionRequest(agent.id, 'connector', 'home_local_api'))
    await waitForRuntimeTool(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, 'home_local_api_public_echo', false)
    await expectRuntimeMcpReject(launch.runtime_mcp_server_url, launch.runtime_mcp_auth_token, 'tools/call', {
      name: 'home_local_api_public_echo',
      arguments: { q: 'after-connector-revoke' },
    })
    const afterConnectorRevokeMarker = await readFile(homeConnectorMarker, 'utf8')
    if (afterConnectorRevokeMarker !== connectorMarker) throw new Error('revoked home-proxy connector executed after revoke')

    succeeded = true
    console.log('[remote-home-extension-drill] pass', JSON.stringify({ script: 'home_only_lookup', mcp: 'home_echo_mcp', connector: 'home_local_api', workerAlias, revoke: true }))
  } finally {
    await client?.close?.().catch(() => {})
    await terminateChild(worker)
    await terminateChild(home)
    await terminateChild(relay)
    await new Promise((resolve) => homeOnlyMcp?.close?.(resolve) ?? resolve())
    if (succeeded) await rm(rootDir, { recursive: true, force: true })
    else console.error(`[remote-home-extension-drill] artifacts kept at ${rootDir}`)
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
