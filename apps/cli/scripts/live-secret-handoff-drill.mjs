#!/usr/bin/env node
import http from 'node:http'
import { spawn } from 'node:child_process'
import { mkdir, rm, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

function log(name, details) {
  if (details === undefined) console.log(`[secret-drill] ${name}`)
  else console.log(`[secret-drill] ${name}`, JSON.stringify(details))
}

function makePorts() {
  const base = 52000 + Math.floor(Math.random() * 1000)
  return {
    localKernel: base,
    workerKernel: base + 1,
    localMcp: base + 1000,
    workerMcp: base + 1001,
    localOpenCode: base + 2000,
    workerOpenCode: base + 2001,
    localCodex: base + 3000,
    workerCodex: base + 3001,
  }
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
  const exists = await stat(binary).then((info) => info.isFile()).catch(() => false)
  if (exists) return binary
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'arroba-kernel'])
  if (result.code !== 0) {
    throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  }
  return binary
}

async function buildKernelClient() {
  const result = await run('pnpm', ['--filter', '@arroba/kernel-client', 'build'])
  if (result.code !== 0) {
    throw new Error(`kernel-client build failed\n${result.stdout}\n${result.stderr}`)
  }
}

async function terminate(child) {
  if (!child || child.exitCode != null) return
  child.kill('SIGTERM')
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    new Promise((resolve) => setTimeout(resolve, 5_000)),
  ])
  if (child.exitCode == null) {
    child.kill('SIGKILL')
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      new Promise((resolve) => setTimeout(resolve, 2_000)),
    ])
  }
}

async function startVerifierServer(expectedToken) {
  const requests = []
  const server = http.createServer((request, response) => {
    const auth = request.headers.authorization ?? ''
    requests.push({ url: request.url, auth })
    const ok = auth === `Bearer ${expectedToken}`
    response.writeHead(ok ? 200 : 401, { 'content-type': 'application/json' })
    response.end(JSON.stringify({ ok, saw_auth: ok }))
  })
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      server.off('error', reject)
      resolve()
    })
  })
  return {
    server,
    requests,
    host: `127.0.0.1:${server.address().port}`,
    close: () => new Promise((resolve) => server.close(resolve)),
  }
}

async function waitForDaemon(LocalIpcClient, requests, kernelUrl, workspace) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      const created = variant(await client.send(requests.createSessionRequest(workspace, workspace)), 'SessionCreated')
      await client.send(requests.endSessionRequest(created.session.id)).catch(() => {})
      await client.close()
      return
    } catch {
      await client.close().catch(() => {})
      await new Promise((resolve) => setTimeout(resolve, 250))
    }
  }
  throw new Error(`daemon at ${kernelUrl} did not become ready`)
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

async function launchDevStub(client, requests, workspace, label, model = 'secret-drill-model') {
  const created = variant(await client.send(requests.createSessionRequest(workspace, workspace)), 'SessionCreated')
  const session = created.session
  const agent = created.agent
  const attachment = variant(
    await client.send(requests.attachToSessionRequest(session.id, `${label}-${Date.now()}`)),
    'SessionAttached',
  ).attachment
  const launched = optionalVariant(
    await client.send(requests.launchProviderRunRequest(session.id, 'dev-stub', 'default', model, 'low', agent.id)),
    'ProviderRunLaunched',
    'ProviderRunLaunchAccepted',
  )
  let providerRun = launched?.provider_run
  if (!providerRun) {
    const state = optionalVariant(await client.send(requests.getSessionStateRequest(session.id)), 'SessionState', 'SessionStateLoaded')
    providerRun = state?.provider_run
  }
  if (!providerRun?.id) throw new Error(`${label}: provider run did not launch`)
  providerRun = variant(await client.send(requests.getProviderRunRequest(providerRun.id)), 'ProviderRun').provider_run
  if (!providerRun.runtime_mcp_server_url || !providerRun.runtime_mcp_auth_token) {
    throw new Error(`${label}: provider run missing runtime MCP binding`)
  }
  return { session, agent, attachment, providerRun }
}

async function mcpToolCall(providerRun, name, args) {
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
      params: { name, arguments: args ?? {} },
    }),
  })
  const text = await response.text()
  let payload
  try {
    payload = JSON.parse(text)
  } catch {
    throw new Error(`runtime MCP response was not JSON (${response.status}): ${text}`)
  }
  if (!response.ok) {
    throw new Error(`runtime MCP HTTP ${response.status}: ${text}`)
  }
  if (payload.error) {
    return { ok: false, error: payload.error }
  }
  return {
    ok: !payload.result?.isError,
    content: payload.result?.structuredContent,
    raw: payload,
  }
}

async function pumpTerminalContains(client, requests, sessionId, attachmentId, expected) {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const response = optionalVariant(
      await client.send(requests.pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => null),
      'TerminalOutput',
    )
    const records = response?.records ?? []
    const text = records
      .map((record) => Buffer.from(record.bytes ?? []).toString('utf8'))
      .join('')
    if (text.includes(expected)) return true
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  return false
}

async function writeConfig(configRoot, verifierHost, credentialId, tokenEnv, terminalId, terminalEnv) {
  await mkdir(path.join(configRoot, 'arroba'), { recursive: true })
  await writeFile(path.join(configRoot, 'arroba', 'config.toml'), [
    'version = 1',
    '',
    '[[credentials]]',
    `id = "${credentialId}"`,
    `source = { type = "env", name = "${tokenEnv}" }`,
    `allowed_hosts = ["${verifierHost}"]`,
    'allowed_uses = ["http"]',
    'injection = { kind = "header", name = "authorization", value = "Bearer ${secret}" }',
    '',
    '[[credentials]]',
    `id = "${terminalId}"`,
    `source = { type = "env", name = "${terminalEnv}" }`,
    'allowed_uses = ["pty"]',
    'injection = { kind = "pty" }',
    '',
  ].join('\n'), 'utf8')
}

function daemonEnv({ baseEnv, rootDir, configRoot, homeDir, kernelPort, mcpPort, opencodePort, codexPort, daemonId, tokenEnv, token, terminalEnv, terminalSecret }) {
  return {
    ...baseEnv,
    XDG_CONFIG_HOME: configRoot,
    HOME: homeDir,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(mcpPort),
    ARROBA_OPENCODE_PORT: String(opencodePort),
    ARROBA_CODEX_PORT: String(codexPort),
    ARROBA_DAEMON_ID: daemonId,
    ARROBA_MACHINE_ID: `${daemonId}-machine`,
    ARROBA_DAEMON_SOCKET: path.join(rootDir, `${daemonId}.sock`),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, `${daemonId}-history`),
    [tokenEnv]: token,
    [terminalEnv]: terminalSecret,
  }
}

async function main() {
  const keepArtifacts = process.argv.includes('--keep-artifacts-on-failure')
  const rootDir = path.join(repoRoot, 'target', 'live-secret-handoff-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const localConfig = path.join(rootDir, 'local-config')
  const workerConfig = path.join(rootDir, 'worker-config')
  const localHome = path.join(rootDir, 'local-home')
  const workerHome = path.join(rootDir, 'worker-home')
  const ports = makePorts()
  const localToken = `local-token-${process.pid}`
  const workerToken = `worker-token-${process.pid}`
  const terminalSecret = `terminal-secret-${process.pid}`
  const workerTerminalSecret = `worker-terminal-secret-${process.pid}`
  const localTokenEnv = 'ARROBA_SECRET_DRILL_LOCAL_TOKEN'
  const workerTokenEnv = 'ARROBA_SECRET_DRILL_WORKER_TOKEN'
  const localTerminalEnv = 'ARROBA_SECRET_DRILL_LOCAL_TERMINAL'
  const workerTerminalEnv = 'ARROBA_SECRET_DRILL_WORKER_TERMINAL'
  let localDaemon = null
  let workerDaemon = null
  let localClient = null
  let workerClient = null
  let localVerifier = null
  let workerVerifier = null
  let succeeded = false

  try {
    await mkdir(workspace, { recursive: true })
    const [kernelBinary] = await Promise.all([buildKernel(), buildKernelClient()])
    const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
    const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')

    localVerifier = await startVerifierServer(localToken)
    workerVerifier = await startVerifierServer(workerToken)
    await writeConfig(localConfig, localVerifier.host, 'local-api', localTokenEnv, 'local-terminal', localTerminalEnv)
    await writeConfig(workerConfig, workerVerifier.host, 'worker-api', workerTokenEnv, 'worker-terminal', workerTerminalEnv)

    localDaemon = spawn(kernelBinary, [], {
      cwd: repoRoot,
      env: daemonEnv({
        baseEnv: process.env,
        rootDir,
        configRoot: localConfig,
        homeDir: localHome,
        kernelPort: ports.localKernel,
        mcpPort: ports.localMcp,
        opencodePort: ports.localOpenCode,
        codexPort: ports.localCodex,
        daemonId: `secret-drill-local-${process.pid}`,
        tokenEnv: localTokenEnv,
        token: localToken,
        terminalEnv: localTerminalEnv,
        terminalSecret,
      }),
      stdio: ['ignore', 'ignore', 'inherit'],
    })
    workerDaemon = spawn(kernelBinary, [], {
      cwd: repoRoot,
      env: daemonEnv({
        baseEnv: process.env,
        rootDir,
        configRoot: workerConfig,
        homeDir: workerHome,
        kernelPort: ports.workerKernel,
        mcpPort: ports.workerMcp,
        opencodePort: ports.workerOpenCode,
        codexPort: ports.workerCodex,
        daemonId: `secret-drill-worker-${process.pid}`,
        tokenEnv: workerTokenEnv,
        token: workerToken,
        terminalEnv: workerTerminalEnv,
        terminalSecret: workerTerminalSecret,
      }),
      stdio: ['ignore', 'ignore', 'inherit'],
    })

    const localKernelUrl = `ws://127.0.0.1:${ports.localKernel}`
    const workerKernelUrl = `ws://127.0.0.1:${ports.workerKernel}`
    await waitForDaemon(LocalIpcClient, requests, localKernelUrl, workspace)
    await waitForDaemon(LocalIpcClient, requests, workerKernelUrl, workspace)
    localClient = new LocalIpcClient(localKernelUrl)
    workerClient = new LocalIpcClient(workerKernelUrl)

    const local = await launchDevStub(localClient, requests, workspace, 'local')
    const worker = await launchDevStub(workerClient, requests, workspace, 'worker')

    if (!local.providerRun.pty_env_remove?.includes(localTokenEnv)) {
      throw new Error(`local provider env removal did not include ${localTokenEnv}`)
    }
    if (!worker.providerRun.pty_env_remove?.includes(workerTokenEnv)) {
      throw new Error(`worker provider env removal did not include ${workerTokenEnv}`)
    }
    log('provider_env_scrub_ok')

    const localHandles = await mcpToolCall(local.providerRun, 'list_credential_handles', {})
    const workerHandles = await mcpToolCall(worker.providerRun, 'list_credential_handles', {})
    const localIds = new Set((localHandles.content?.credentials ?? []).map((credential) => credential.id))
    const workerIds = new Set((workerHandles.content?.credentials ?? []).map((credential) => credential.id))
    if (!localIds.has('local-api') || localIds.has('worker-api')) {
      throw new Error(`local credential discovery was not local-only: ${JSON.stringify([...localIds])}`)
    }
    if (!workerIds.has('worker-api') || workerIds.has('local-api')) {
      throw new Error(`worker credential discovery was not worker-only: ${JSON.stringify([...workerIds])}`)
    }
    log('credential_discovery_locality_ok')

    const localHttp = await mcpToolCall(local.providerRun, 'http_request_with_credential', {
      credential_id: 'local-api',
      method: 'GET',
      url: `http://${localVerifier.host}/local`,
    })
    if (!localHttp.ok || localHttp.content?.status !== 200 || localHttp.content?.body_json?.ok !== true) {
      throw new Error(`local HTTP credential call failed: ${JSON.stringify(localHttp)}`)
    }
    const workerHttp = await mcpToolCall(worker.providerRun, 'http_request_with_credential', {
      credential_id: 'worker-api',
      method: 'GET',
      url: `http://${workerVerifier.host}/worker`,
    })
    if (!workerHttp.ok || workerHttp.content?.status !== 200 || workerHttp.content?.body_json?.ok !== true) {
      throw new Error(`worker HTTP credential call failed: ${JSON.stringify(workerHttp)}`)
    }
    const serializedHttp = JSON.stringify({ localHttp, workerHttp })
    if (serializedHttp.includes(localToken) || serializedHttp.includes(workerToken)) {
      throw new Error('HTTP credential result leaked a token')
    }
    log('http_credential_calls_ok')

    const wrongHost = await mcpToolCall(local.providerRun, 'http_request_with_credential', {
      credential_id: 'local-api',
      method: 'GET',
      url: `http://${workerVerifier.host}/wrong-host`,
    })
    const wrongHostMessage = JSON.stringify(wrongHost)
    if (wrongHost.ok || !wrongHostMessage.includes('not allowed for host')) {
      throw new Error(`wrong-host denial did not surface expected error: ${wrongHostMessage}`)
    }
    if (wrongHostMessage.includes(localToken) || wrongHostMessage.includes(localTokenEnv)) {
      throw new Error('wrong-host denial leaked token material')
    }
    const missingWorkerLocal = await mcpToolCall(worker.providerRun, 'http_request_with_credential', {
      credential_id: 'local-api',
      method: 'GET',
      url: `http://${localVerifier.host}/missing`,
    })
    if (missingWorkerLocal.ok || !JSON.stringify(missingWorkerLocal).includes('unknown credential `local-api`')) {
      throw new Error(`worker missing-handle denial failed: ${JSON.stringify(missingWorkerLocal)}`)
    }
    log('denials_ok')

    const terminal = await mcpToolCall(local.providerRun, 'send_secret_to_terminal', {
      credential_id: 'local-terminal',
      append_newline: true,
    })
    if (!terminal.ok || terminal.content?.submitted !== true) {
      throw new Error(`terminal secret handoff failed: ${JSON.stringify(terminal)}`)
    }
    const terminalEchoed = await pumpTerminalContains(localClient, requests, local.session.id, local.attachment.id, terminalSecret)
    if (!terminalEchoed) {
      throw new Error('terminal handoff did not reach provider PTY')
    }
    log('terminal_handoff_ok')

    await localClient.send(requests.endSessionRequest(local.session.id)).catch(() => {})
    await workerClient.send(requests.endSessionRequest(worker.session.id)).catch(() => {})
    succeeded = true
    log('passed', {
      localVerifierRequests: localVerifier.requests.length,
      workerVerifierRequests: workerVerifier.requests.length,
    })
  } finally {
    await localClient?.close?.().catch(() => {})
    await workerClient?.close?.().catch(() => {})
    await terminate(localDaemon)
    await terminate(workerDaemon)
    await localVerifier?.close?.().catch(() => {})
    await workerVerifier?.close?.().catch(() => {})
    if (succeeded || !keepArtifacts) {
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    } else {
      console.error(`[secret-drill] kept artifacts at ${rootDir}`)
    }
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack || error.message : String(error))
  process.exitCode = 1
})
