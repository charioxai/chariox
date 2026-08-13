#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const {
  attachToSessionRequest,
  createSessionRequest,
  grantAgentExtensionRequest,
  listEnvironmentsRequest,
  listScriptsRequest,
  listSessionsRequest,
  registerEnvironmentRequest,
  registerScriptRequest,
  spawnAgentRequest,
  validateScriptRequest,
} = await import('../../../packages/kernel-client/dist/ipc-requests.js')

function log(message, details) {
  if (details === undefined) console.log(`[script-extension-drill] ${message}`)
  else console.log(`[script-extension-drill] ${message}`, JSON.stringify(details))
}

function parseArgs(argv) {
  const options = { keepArtifactsOnFailure: false }
  for (const arg of argv) {
    if (arg === '--') continue
    if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help' || arg === '-h') {
      console.log('Usage: node apps/cli/scripts/live-script-extension-drill.mjs [--keep-artifacts-on-failure]')
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

async function commandPath(command) {
  const result = await run('bash', ['-lc', `command -v ${command}`])
  if (result.code !== 0 || !result.stdout.trim()) return null
  return result.stdout.trim()
}

async function buildKernel() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/chariox-kernel')
  const result = await run('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'chariox-kernel'])
  if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
  return binary
}

function startDaemon(binary, env) {
  const child = spawn(binary, [], {
    cwd: repoRoot,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  child.stdoutText = ''
  child.stderrText = ''
  child.stdout.on('data', (chunk) => { child.stdoutText += chunk.toString() })
  child.stderr.on('data', (chunk) => { child.stderrText += chunk.toString() })
  return child
}

async function stopDaemon(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  child.kill('SIGTERM')
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    new Promise((resolve) => setTimeout(resolve, 3000)),
  ])
  if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
}

async function waitForDaemon(kernelUrl) {
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(listSessionsRequest())
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await new Promise((resolve) => setTimeout(resolve, 250))
    }
  }
  throw new Error(`daemon did not become ready: ${lastError?.message ?? String(lastError)}`)
}

function variant(response, key) {
  if (!response?.[key]) throw new Error(`expected ${key}, got ${JSON.stringify(response)}`)
  return response[key]
}

function hasExtensionGrant(agent, kind, name, environment = null) {
  return Array.isArray(agent?.extension_grants) && agent.extension_grants.some((grant) => (
    grant.kind === kind && grant.name === name && (environment === null || grant.environment === environment)
  ))
}

async function main() {
  const keepArtifacts = process.argv.slice(2).includes('--keep-artifacts-on-failure')
  const root = path.join(repoRoot, 'target', 'live-script-extension-drill', `${process.pid}-${Date.now()}`)
  let options = { keepArtifactsOnFailure: keepArtifacts }
  let failure = null
  let environmentName = null
  let scriptName = null
  let sessionId = null
  let agentId = null
  let python = null

  const workspace = path.join(root, 'workspace')
  const home = path.join(root, 'home')
  const configHome = path.join(root, 'config')
  const stateHome = path.join(root, 'state')
  const scriptPath = path.join(root, 'vector_lookup.py')
  const kernelPort = 49000 + Math.floor(Math.random() * 1000)
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const env = {
    ...process.env,
    HOME: home,
    XDG_CONFIG_HOME: configHome,
    XDG_STATE_HOME: stateHome,
    CHARIOX_KERNEL_PORT: String(kernelPort),
    CHARIOX_MCP_PORT: String(kernelPort + 1000),
    CHARIOX_OPENCODE_PORT: String(kernelPort + 2000),
    CHARIOX_CODEX_PORT: String(kernelPort + 2001),
    CHARIOX_DAEMON_ID: `script-extension-drill-${process.pid}-${Date.now()}`,
    CHARIOX_DAEMON_SOCKET: path.join(root, 'daemon.sock'),
  }

  let daemon = null
  let client = null
  let succeeded = false
  try {
    await prepareDrillArtifacts(root)
    options = parseArgs(process.argv.slice(2))
    python = process.env.PYTHON || await commandPath('python3') || await commandPath('python')
    if (!python) throw new Error('python3 or python is required for the script extension drill')
    await mkdir(workspace, { recursive: true })
    await mkdir(path.join(configHome, 'chariox'), { recursive: true })
    await writeFile(path.join(configHome, 'chariox', 'config.toml'), 'version = 1\n', 'utf8')
    await writeFile(scriptPath, `
def run(query: str, limit: int = 1) -> dict[str, object]:
    """Lookup deterministic rows from a vector database fixture."""
    print("VECTOR_LOOKUP_LOG")
    return {"query": query, "limit": limit, "ids": ["doc-1"]}

def test_run() -> None:
    result = run("demo", 2)
    assert result["query"] == "demo"
    assert result["limit"] == 2
`, 'utf8')

    const kernelBinary = await buildKernel()
    daemon = startDaemon(kernelBinary, env)
    await waitForDaemon(kernelUrl)
    client = new LocalIpcClient(kernelUrl)

    const environment = variant(await client.send(registerEnvironmentRequest(workspace, {
      name: 'py-drill',
      runtime: { type: 'python', python },
    })), 'EnvironmentRegistered').environment
    environmentName = environment.name
    const validated = variant(await client.send(validateScriptRequest(workspace, scriptPath, environment.name, 'vector_lookup')), 'ScriptValidated').script
    const registered = variant(await client.send(registerScriptRequest(workspace, scriptPath, environment.name, 'vector_lookup')), 'ScriptRegistered').script
    scriptName = registered.name
    const environments = variant(await client.send(listEnvironmentsRequest(workspace)), 'EnvironmentsListed').environments
    const scripts = variant(await client.send(listScriptsRequest(workspace)), 'ScriptsListed').scripts

    if (validated.name !== 'vector_lookup' || registered.name !== 'vector_lookup') throw new Error('script validation/registration returned wrong script')
    if (!environments.some((entry) => entry.name === environment.name)) throw new Error('registered environment missing from list')
    if (!scripts.some((entry) => entry.name === 'vector_lookup')) throw new Error('registered script missing from list')

    const session = variant(await client.send(createSessionRequest(workspace, workspace, 'script-extension-drill')), 'SessionCreated').session
    sessionId = session.id
    await client.send(attachToSessionRequest(session.id, `script-extension-drill-${process.pid}`))
    const agent = variant(
      await client.send(spawnAgentRequest(session.id, 'dev-stub', 'script-agent', 'script-profile', workspace, 'low')),
      'AgentSpawned',
    ).agent
    agentId = agent.id
    const granted = variant(
      await client.send(grantAgentExtensionRequest(workspace, agent.id, 'script', 'vector_lookup', environment.name)),
      'AgentExtensionGranted',
    ).agent
    if (!hasExtensionGrant(granted, 'script', 'vector_lookup', environment.name)) {
      throw new Error('script grant was accepted but missing from the agent')
    }

    succeeded = true
    log('pass', { workspace, script: registered.name, environment: environment.name })
  } catch (error) {
    failure = error
    throw error
  } finally {
    await client?.close?.().catch(() => {})
    await stopDaemon(daemon)
    await finalizeDrillArtifacts({
      rootDir: root,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: 'script-extension',
        workspace,
        scriptPath,
        kernelUrl,
        python,
        environmentName,
        scriptName,
        sessionId,
        agentId,
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
