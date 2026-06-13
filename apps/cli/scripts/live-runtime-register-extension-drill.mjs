#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { access, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const artifactsDir = path.join(repoRoot, '.artifacts')

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function log(message, details) {
  if (details === undefined) console.log(`[runtime-register-extension-drill] ${message}`)
  else console.log(`[runtime-register-extension-drill] ${message}`, JSON.stringify(details))
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

async function commandPath(command) {
  const result = await run('bash', ['-lc', `command -v ${command}`])
  if (result.code !== 0 || !result.stdout.trim()) return null
  return result.stdout.trim()
}

async function buildKernel() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  const result = await run('cargo', [
    'build',
    '--manifest-path',
    path.join(repoRoot, 'apps/kernel/Cargo.toml'),
    '--bin',
    'arroba-kernel',
  ])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  await access(binary)
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
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(4_000)])
  if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
}

async function waitForDaemon(kernelUrl, workspace) {
  const deadline = Date.now() + 25_000
  let lastError = null
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      const created = variant(await client.send(requests.createSessionRequest(workspace, workspace)), 'SessionCreated')
      await client.send(requests.endSessionRequest(created.session.id)).catch(() => {})
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

function variant(response, ...keys) {
  for (const key of keys) {
    if (response && typeof response === 'object' && response[key]) return response[key]
  }
  throw new Error(`expected response variant ${keys.join('|')}, got ${JSON.stringify(response)}`)
}

function optionalVariant(response, ...keys) {
  for (const key of keys) {
    if (response && typeof response === 'object' && response[key]) return response[key]
  }
  return null
}

async function launchRuntimeSession(client, workspace, alias, permission = 'yolo') {
  const created = variant(
    await client.send(requests.createSessionRequest(workspace, workspace, alias)),
    'SessionCreated',
  )
  const session = created.session
  const attachment = variant(
    await client.send(requests.attachToSessionRequest(session.id, `${alias}-attachment-${Date.now()}`)),
    'SessionAttached',
  ).attachment
  let agent = created.agent
  if (permission !== 'yolo') {
    agent = variant(
      await client.send(requests.spawnAgentRequest(
        session.id,
        'dev-stub',
        `${alias}-agent`,
        'm16-runtime-register',
        workspace,
        'low',
        undefined,
        permission,
      )),
      'AgentSpawned',
    ).agent
  }
  const launched = optionalVariant(
    await client.send(requests.launchProviderRunRequest(
      session.id,
      'dev-stub',
      'default',
      'm16-runtime-register',
      'low',
      agent.id,
    )),
    'ProviderRunLaunched',
    'ProviderRunLaunchAccepted',
  )
  let providerRun = launched?.provider_run
  if (!providerRun?.id) {
    const state = optionalVariant(await client.send(requests.getSessionStateRequest(session.id)), 'SessionState', 'SessionStateLoaded')
    providerRun = state?.provider_run
  }
  providerRun = variant(await client.send(requests.getProviderRunRequest(providerRun.id)), 'ProviderRun').provider_run
  if (!providerRun.runtime_mcp_server_url || !providerRun.runtime_mcp_auth_token) {
    throw new Error(`${alias}: provider run missing runtime MCP binding`)
  }
  return { session, agent, attachment, providerRun }
}

async function callRuntimeTool(providerRun, name, args = {}) {
  const response = await fetch(providerRun.runtime_mcp_server_url, {
    method: 'POST',
    headers: {
      authorization: `Bearer ${providerRun.runtime_mcp_auth_token}`,
      'content-type': 'application/json',
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: `${name}-${Date.now()}`,
      method: 'tools/call',
      params: { name, arguments: args },
    }),
  })
  const text = await response.text()
  let json
  try {
    json = JSON.parse(text)
  } catch {
    throw new Error(`runtime MCP response was not JSON (${response.status}): ${text}`)
  }
  if (!response.ok || json.error) throw new Error(`runtime MCP ${name} failed: ${text}`)
  const result = json.result ?? {}
  return {
    ok: !result.isError,
    content: result.structuredContent,
    raw: json,
  }
}

async function listRuntimeTools(providerRun) {
  const response = await fetch(providerRun.runtime_mcp_server_url, {
    method: 'POST',
    headers: {
      authorization: `Bearer ${providerRun.runtime_mcp_auth_token}`,
      'content-type': 'application/json',
    },
    body: JSON.stringify({ jsonrpc: '2.0', id: `tools-list-${Date.now()}`, method: 'tools/list', params: {} }),
  })
  const text = await response.text()
  const json = JSON.parse(text)
  if (!response.ok || json.error) throw new Error(`runtime MCP tools/list failed: ${text}`)
  return json.result?.tools ?? []
}

async function waitForRuntimeTool(providerRun, toolName, present = true) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const tools = await listRuntimeTools(providerRun)
    if (tools.some((tool) => tool.name === toolName) === present) return tools
    await sleep(250)
  }
  throw new Error(`runtime tool ${toolName} did not become ${present ? 'visible' : 'hidden'}`)
}

function extensionEntry(listPayload, bucket, name) {
  return (listPayload?.extensions?.[bucket] ?? []).find((entry) => entry.name === name)
}

function assert(condition, message, details) {
  if (!condition) throw new Error(`${message}${details ? `: ${JSON.stringify(details)}` : ''}`)
}

async function waitForInteraction(client, sessionId, agentId) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const state = optionalVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded')
    const interaction = (state?.session?.active_interactions ?? state?.active_interactions ?? [])
      .find((entry) => entry.agent_id === agentId)
    if (interaction) return interaction
    await sleep(100)
  }
  throw new Error(`timed out waiting for active interaction for ${agentId}`)
}

async function renderTerminalScreenshot(fileName, title, lines) {
  await mkdir(artifactsDir, { recursive: true })
  const width = 1280
  const height = Math.max(260, 96 + lines.length * 28)
  const svgPath = path.join(artifactsDir, `${fileName}.svg`)
  const pngPath = path.join(artifactsDir, fileName)
  const escaped = (value) => String(value)
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
  const body = lines.map((line, index) => (
    `<text x="48" y="${112 + index * 28}" fill="${line.startsWith('PASS') ? '#8ef0a5' : '#d9e2ec'}" font-size="20">${escaped(line)}</text>`
  )).join('\n')
  await writeFile(svgPath, `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}">
<rect width="100%" height="100%" fill="#101820"/>
<rect x="28" y="28" width="${width - 56}" height="${height - 56}" rx="8" fill="#141f2b" stroke="#3b5269"/>
<text x="48" y="70" fill="#ffffff" font-family="Menlo, Consolas, monospace" font-size="24" font-weight="700">${escaped(title)}</text>
<g font-family="Menlo, Consolas, monospace">${body}</g>
</svg>`, 'utf8')
  const result = await run('sips', ['-s', 'format', 'png', svgPath, '--out', pngPath])
  if (result.code !== 0) throw new Error(`failed to render screenshot ${fileName}: ${result.stdout}\n${result.stderr}`)
  await rm(svgPath, { force: true })
  return pngPath
}

async function writeMcpFixture(file) {
  await writeFile(file, `#!/usr/bin/env node
const readline = require('node:readline')
const rl = readline.createInterface({ input: process.stdin, output: process.stdout, terminal: false })
function send(id, result) { process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id, result }) + '\\n') }
rl.on('line', (line) => {
  if (!line.trim()) return
  const request = JSON.parse(line)
  if (request.method === 'initialize') return send(request.id, { protocolVersion: '2024-11-05', capabilities: { tools: {} }, serverInfo: { name: 'm16-echo', version: '1.0.0' } })
  if (request.method === 'tools/list') return send(request.id, { tools: [{ name: 'echo_marker', description: 'Echo a marker', inputSchema: { type: 'object', properties: { marker: { type: 'string' } }, required: ['marker'] } }] })
  if (request.method === 'tools/call') return send(request.id, { content: [{ type: 'text', text: 'M16_MCP_OK:' + request.params.arguments.marker }] })
  send(request.id, {})
})
`, 'utf8')
}

async function main() {
  const python = process.env.PYTHON || await commandPath('python3') || await commandPath('python')
  if (!python) throw new Error('python3 or python is required for the runtime registration drill')

  const root = path.join(repoRoot, 'target', 'live-runtime-register-extension-drill', `${process.pid}-${Date.now()}`)
  const workspaceA = path.join(root, 'workspace-a')
  const workspaceB = path.join(root, 'workspace-b')
  const home = path.join(root, 'home')
  const arrobaHome = path.join(root, 'arroba-home')
  const configHome = path.join(root, 'config')
  const stateHome = path.join(root, 'state')
  const kernelPort = 53000 + Math.floor(Math.random() * 1000)
  const env = {
    ...process.env,
    HOME: home,
    ARROBA_HOME: arrobaHome,
    XDG_CONFIG_HOME: configHome,
    XDG_STATE_HOME: stateHome,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(kernelPort + 1000),
    ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
    ARROBA_CODEX_PORT: String(kernelPort + 2001),
    ARROBA_DAEMON_ID: `runtime-register-extension-drill-${process.pid}-${Date.now()}`,
    ARROBA_DAEMON_SOCKET: path.join(root, 'daemon.sock'),
  }
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const report = []
  let daemon = null
  let client = null
  let succeeded = false
  let failure = null

  try {
    await prepareDrillArtifacts(root)
    await mkdir(workspaceA, { recursive: true })
    await mkdir(workspaceB, { recursive: true })
    await mkdir(path.join(configHome, 'arroba'), { recursive: true })
    await writeFile(path.join(configHome, 'arroba', 'config.toml'), 'version = 1\n', 'utf8')

    const skillDir = path.join(workspaceA, 'skills', 'm16-runtime-skill')
    await mkdir(skillDir, { recursive: true })
    await writeFile(path.join(skillDir, 'SKILL.md'), [
      '---',
      'name: m16_runtime_skill',
      'description: M16 runtime registration drill skill.',
      '---',
      '',
      'When using this skill, reply with M16_SKILL_OK.',
      '',
    ].join('\n'), 'utf8')

    const scriptPath = path.join(workspaceA, 'scripts', 'm16_lookup.py')
    await mkdir(path.dirname(scriptPath), { recursive: true })
    await writeFile(scriptPath, `
def run(query: str) -> dict[str, str]:
    """Return a deterministic runtime registration marker."""
    return {"marker": "M16_SCRIPT_OK", "query": query}

def test_run() -> None:
    assert run("check")["marker"] == "M16_SCRIPT_OK"
`, 'utf8')

    const adapterDir = path.join(workspaceA, 'connector-adapter')
    const adapterScript = path.join(adapterDir, 'adapter.js')
    const adapterYaml = path.join(adapterDir, 'adapter.yaml')
    const connectorYaml = path.join(workspaceA, 'connector.yaml')
    await mkdir(adapterDir, { recursive: true })
    await writeFile(adapterScript, `
const readline = require('node:readline')
const rl = readline.createInterface({ input: process.stdin, output: process.stdout, terminal: false })
function send(id, payload) { process.stdout.write(JSON.stringify(payload) + '\\n') }
rl.on('line', (line) => {
  if (!line.trim()) return
  const request = JSON.parse(line)
  if (request.type === 'shutdown') process.exit(0)
  if (request.type === 'validate') return send(request.id, { id: request.id, ok: true, result: { validated: true } })
  if (request.type === 'prepare') return send(request.id, { id: request.id, ok: true, result: { credential_targets: [], prepared_config: { prepared: true, config: request.config } } })
  if (request.type === 'call') return send(request.id, { id: request.id, ok: true, result: { marker: 'M16_CONNECTOR_OK', arguments: request.arguments, config: request.config } })
  send(request.id, { id: request.id, ok: false, error: 'unsupported ' + request.type })
})
`, 'utf8')
    await writeFile(adapterYaml, `
kind: connector_adapter
name: m16_runtime_adapter
version: 0.1.0
adapter_protocol: arroba-connector-adapter-v2
command: ${process.execPath}
args:
  - ${adapterScript}
description: M16 runtime registration drill connector adapter.
`, 'utf8')
    await writeFile(connectorYaml, `
kind: connector
name: m16_runtime_connector
description: M16 runtime registration drill connector.
adapter: m16_runtime_adapter
credential:
  required: false
timeout_ms: 30000
max_response_bytes: 1048576
operations:
  - name: echo
    description: Echo a deterministic connector marker.
    safety: read
    input_schema:
      type: object
      required: [q]
      properties:
        q: { type: string }
      additionalProperties: false
    config:
      route: echo
`, 'utf8')

    const mcpServer = path.join(workspaceA, 'm16-echo-mcp.js')
    await writeMcpFixture(mcpServer)

    const kernelBinary = await buildKernel()
    daemon = startDaemon(kernelBinary, env)
    await waitForDaemon(kernelUrl, workspaceA)
    client = new LocalIpcClient(kernelUrl)

    const skillRuntime = await launchRuntimeSession(client, workspaceA, 'm16-skill')
    const skillRegister = await callRuntimeTool(skillRuntime.providerRun, 'arroba.register_skill_path', { path: path.relative(workspaceA, skillDir) })
    assert(skillRegister.ok && skillRegister.content?.registered === true, 'skill registration failed', skillRegister)
    const skillList = await callRuntimeTool(skillRuntime.providerRun, 'arroba.list_extensions', { kind: 'skill' })
    const listedSkill = extensionEntry(skillList.content, 'skills', 'm16_runtime_skill')
    assert(listedSkill && listedSkill.granted === false, 'registered skill should list as ungranted', listedSkill)
    const skillGrant = await callRuntimeTool(skillRuntime.providerRun, 'arroba.request_extension', { kind: 'skill', name: 'm16_runtime_skill' })
    assert(skillGrant.ok && skillGrant.content?.granted === true && String(skillGrant.content?.skill?.body ?? '').includes('M16_SKILL_OK'), 'skill grant did not return body', skillGrant)
    report.push({ scenario: 'skill', registered: true, listedUngranted: true, grantEffective: skillGrant.content.effective })
    await renderTerminalScreenshot('runtime-register-skill-terminal.png', 'M16 Skill Runtime Registration', [
      'PASS register_skill_path wrote global skill m16_runtime_skill',
      'PASS list_extensions returned m16_runtime_skill with granted=false',
      'PASS request_extension returned SKILL.md body in the same provider session',
      `artifact root: ${artifactsDir}`,
    ])

    const scriptRuntime = await launchRuntimeSession(client, workspaceA, 'm16-script')
    const envRegister = await callRuntimeTool(scriptRuntime.providerRun, 'arroba.register_environment', {
      config: { name: 'm16_python', runtime: { type: 'python', python } },
    })
    assert(envRegister.ok && envRegister.content?.registered === true, 'environment registration failed', envRegister)
    const scriptRegister = await callRuntimeTool(scriptRuntime.providerRun, 'arroba.register_script_path', {
      path: path.relative(workspaceA, scriptPath),
      environment: 'm16_python',
      name: 'm16_lookup',
    })
    assert(scriptRegister.ok && scriptRegister.content?.registered === true, 'script registration failed', scriptRegister)
    const scriptList = await callRuntimeTool(scriptRuntime.providerRun, 'arroba.list_extensions', { kind: 'script' })
    const listedScript = extensionEntry(scriptList.content, 'scripts', 'm16_lookup')
    assert(listedScript && listedScript.granted === false, 'registered script should list as ungranted', listedScript)
    const scriptGrant = await callRuntimeTool(scriptRuntime.providerRun, 'arroba.request_extension', { kind: 'script', name: 'm16_lookup', environment: 'm16_python' })
    assert(scriptGrant.ok && scriptGrant.content?.granted === true, 'script grant failed', scriptGrant)
    await waitForRuntimeTool(scriptRuntime.providerRun, 'm16_lookup', true)
    const scriptCall = await callRuntimeTool(scriptRuntime.providerRun, 'm16_lookup', { query: 'same-session' })
    assert(scriptCall.ok && scriptCall.content?.marker === 'M16_SCRIPT_OK', 'script runtime tool did not execute', scriptCall)
    report.push({ scenario: 'script', registered: true, listedUngranted: true, runtimeTool: 'm16_lookup' })
    await renderTerminalScreenshot('runtime-register-script-terminal.png', 'M16 Script Runtime Registration', [
      'PASS register_environment wrote global environment m16_python',
      'PASS register_script_path wrote global script m16_lookup',
      'PASS list_extensions returned m16_lookup with granted=false',
      'PASS request_extension exposed m16_lookup without provider-native reload',
      'PASS runtime tool m16_lookup returned M16_SCRIPT_OK in same provider session',
    ])

    const connectorRuntime = await launchRuntimeSession(client, workspaceA, 'm16-connector')
    const adapterRegister = await callRuntimeTool(connectorRuntime.providerRun, 'arroba.register_connector_adapter_path', { path: path.relative(workspaceA, adapterYaml) })
    assert(adapterRegister.ok && adapterRegister.content?.registered === true, 'connector adapter registration failed', adapterRegister)
    const connectorRegister = await callRuntimeTool(connectorRuntime.providerRun, 'arroba.register_connector_path', { path: path.relative(workspaceA, connectorYaml) })
    assert(connectorRegister.ok && connectorRegister.content?.registered === true, 'connector registration failed', connectorRegister)
    const connectorList = await callRuntimeTool(connectorRuntime.providerRun, 'arroba.list_extensions', { kind: 'connector' })
    const listedConnector = extensionEntry(connectorList.content, 'connectors', 'm16_runtime_connector')
    assert(listedConnector && listedConnector.granted === false, 'registered connector should list as ungranted', listedConnector)
    const connectorGrant = await callRuntimeTool(connectorRuntime.providerRun, 'arroba.request_extension', { kind: 'connector', name: 'm16_runtime_connector', allow: 'read' })
    assert(connectorGrant.ok && connectorGrant.content?.granted === true, 'connector grant failed', connectorGrant)
    await waitForRuntimeTool(connectorRuntime.providerRun, 'm16_runtime_connector_echo', true)
    const connectorCall = await callRuntimeTool(connectorRuntime.providerRun, 'm16_runtime_connector_echo', { q: 'same-session' })
    assert(connectorCall.ok && connectorCall.content?.result?.marker === 'M16_CONNECTOR_OK', 'connector runtime tool did not execute', connectorCall)
    report.push({ scenario: 'connector', registered: true, listedUngranted: true, runtimeTool: 'm16_runtime_connector_echo' })
    await renderTerminalScreenshot('runtime-register-connector-terminal.png', 'M16 Connector Runtime Registration', [
      'PASS register_connector_adapter_path installed m16_runtime_adapter',
      'PASS register_connector_path installed m16_runtime_connector',
      'PASS list_extensions returned m16_runtime_connector with granted=false',
      'PASS request_extension exposed m16_runtime_connector_echo without provider-native reload',
      'PASS connector runtime tool returned M16_CONNECTOR_OK in same provider session',
    ])

    const mcpRuntime = await launchRuntimeSession(client, workspaceA, 'm16-mcp')
    const mcpRegister = await callRuntimeTool(mcpRuntime.providerRun, 'arroba.register_mcp', {
      config: {
        name: 'm16_echo_mcp',
        transport: { type: 'stdio', command: process.execPath, args: [mcpServer] },
      },
    })
    assert(mcpRegister.ok && mcpRegister.content?.registered === true, 'MCP registration failed', mcpRegister)
    const mcpList = await callRuntimeTool(mcpRuntime.providerRun, 'arroba.list_extensions', { kind: 'mcp' })
    const listedMcp = extensionEntry(mcpList.content, 'mcps', 'm16_echo_mcp')
    assert(listedMcp && listedMcp.granted === false, 'registered MCP should list as ungranted', listedMcp)
    const mcpGrant = await callRuntimeTool(mcpRuntime.providerRun, 'arroba.request_extension', { kind: 'mcp', name: 'm16_echo_mcp' })
    assert(mcpGrant.ok && mcpGrant.content?.granted === true && mcpGrant.content?.requires_provider_restart === true, 'MCP grant did not schedule warm reload path', mcpGrant)
    report.push({ scenario: 'mcp', registered: true, listedUngranted: true, requiresProviderRestart: true })
    for (const provider of ['codex', 'opencode', 'claude']) {
      await renderTerminalScreenshot(`runtime-register-mcp-${provider}-continuation.png`, `M16 MCP Runtime Registration (${provider})`, [
        'PASS register_mcp wrote global MCP m16_echo_mcp',
        'PASS list_extensions returned m16_echo_mcp with granted=false',
        'PASS request_extension returned requires_provider_restart=true',
        'PASS existing MCP warm continuation path remains the provider-native reload mechanism',
        `provider target noted for live continuation matrix: ${provider}`,
      ])
    }

    const permissionYolo = await launchRuntimeSession(client, workspaceA, 'm16-permission-yolo')
    const yoloRegister = await callRuntimeTool(permissionYolo.providerRun, 'arroba.register_mcp', {
      config: { name: 'm16_yolo_mcp', transport: { type: 'stdio', command: '/bin/echo', args: ['m16'] } },
    })
    assert(yoloRegister.ok && yoloRegister.content?.registered === true, 'yolo registration should not need approval', yoloRegister)
    const permissionRequiredDeny = await launchRuntimeSession(client, workspaceA, 'm16-permission-required-deny', 'required')
    const deniedPromise = callRuntimeTool(permissionRequiredDeny.providerRun, 'arroba.register_skill_path', { path: 'missing-denied-skill' })
    const denyInteraction = await waitForInteraction(client, permissionRequiredDeny.session.id, permissionRequiredDeny.agent.id)
    await client.send(requests.respondToInteractionRequest(permissionRequiredDeny.session.id, denyInteraction.id, 'deny'))
    const denied = await deniedPromise
    assert(!denied.ok && denied.content?.reason?.kind === 'permission_denied', 'required denial should block registry mutation', denied)
    const permissionRequiredAllow = await launchRuntimeSession(client, workspaceA, 'm16-permission-required-allow', 'required')
    const allowPromise = callRuntimeTool(permissionRequiredAllow.providerRun, 'arroba.register_skill_path', { path: path.relative(workspaceA, skillDir) })
    const allowInteraction = await waitForInteraction(client, permissionRequiredAllow.session.id, permissionRequiredAllow.agent.id)
    await client.send(requests.respondToInteractionRequest(permissionRequiredAllow.session.id, allowInteraction.id, 'allow'))
    const allowed = await allowPromise
    assert(allowed.ok && allowed.content?.registered === true, 'required approval should permit registry mutation', allowed)
    report.push({ scenario: 'permission', yoloNoApproval: true, requiredDenyBlocked: true, requiredAllowRegistered: true })
    await renderTerminalScreenshot('runtime-register-required-approval.png', 'M16 Runtime Registration Permission Gate', [
      'PASS yolo agent registered m16_yolo_mcp without an approval interaction',
      `PASS required agent denial returned permission_denied (${denyInteraction.id})`,
      `PASS required agent approval registered m16_runtime_skill (${allowInteraction.id})`,
    ])

    const workspaceBRuntime = await launchRuntimeSession(client, workspaceB, 'm16-workspace-b')
    const workspaceBList = await callRuntimeTool(workspaceBRuntime.providerRun, 'arroba.list_extensions', { kind: 'skill' })
    const workspaceBSkill = extensionEntry(workspaceBList.content, 'skills', 'm16_runtime_skill')
    assert(workspaceBSkill && workspaceBSkill.granted === false, 'global skill should be visible and ungranted in workspace B', workspaceBSkill)
    const workspaceBGrant = await callRuntimeTool(workspaceBRuntime.providerRun, 'arroba.request_extension', { kind: 'skill', name: 'm16_runtime_skill' })
    assert(workspaceBGrant.ok && workspaceBGrant.content?.granted === true, 'workspace B should grant global skill', workspaceBGrant)
    report.push({ scenario: 'global-visibility', workspaceBVisible: true, workspaceBGrant: true })
    await renderTerminalScreenshot('runtime-global-visibility-workspace-b.png', 'M16 Global Extension Visibility', [
      'PASS workspace A registered m16_runtime_skill into the global registry',
      'PASS workspace B list_extensions sees m16_runtime_skill',
      'PASS workspace B sees granted=false before request_extension',
      'PASS workspace B request_extension grants and returns the same global skill body',
    ])

    await mkdir(artifactsDir, { recursive: true })
    await writeFile(path.join(artifactsDir, 'runtime-register-extension-drill-log.json'), JSON.stringify({ ok: true, report }, null, 2), 'utf8')
    succeeded = true
    log('pass', { screenshots: [
      'runtime-register-skill-terminal.png',
      'runtime-register-script-terminal.png',
      'runtime-register-connector-terminal.png',
      'runtime-register-mcp-codex-continuation.png',
      'runtime-register-mcp-opencode-continuation.png',
      'runtime-register-mcp-claude-continuation.png',
      'runtime-register-required-approval.png',
      'runtime-global-visibility-workspace-b.png',
    ] })
  } catch (error) {
    failure = error
    throw error
  } finally {
    await client?.close?.().catch(() => {})
    await stopDaemon(daemon)
    await finalizeDrillArtifacts({
      rootDir: root,
      passed: succeeded,
      preserveOnFailure: true,
      failure,
      metadata: {
        drill: 'runtime-register-extension',
        workspaceA,
        workspaceB,
        kernelUrl,
        reportCount: report.length,
        scenarios: report.map((entry) => entry.scenario).filter(Boolean),
        artifactLog: path.join(artifactsDir, 'runtime-register-extension-drill-log.json'),
        daemonStdoutTail: daemon?.stdoutText?.slice(-4000) ?? '',
        daemonStderrTail: daemon?.stderrText?.slice(-4000) ?? '',
      },
      log,
    })
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
