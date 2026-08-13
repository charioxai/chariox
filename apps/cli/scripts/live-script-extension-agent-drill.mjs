#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { access, mkdir, readFile, writeFile } from 'node:fs/promises'
import { homedir, tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const DEFAULT_PROVIDERS = ['codex', 'opencode', 'claude-p', 'claude-headless']
const DEFAULT_MODEL = 'gpt-5.2'
const DEFAULT_TIMEOUT_MS = 420_000
const DEFAULT_POLL_MS = 1_000
const realHomeDir = process.env.HOME || homedir()

const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')

function parseArgs(argv) {
  const options = {
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
    providerModels: {},
    effort: 'low',
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    keepArtifactsOnFailure: false,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--') continue
    if (arg === '--provider') options.providers = [argv[++i]]
    else if (arg === '--providers') options.providers = argv[++i].split(',').map((value) => value.trim()).filter(Boolean)
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++i].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    } else if (arg === '--effort') options.effort = argv[++i]
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown option: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-script-extension-agent-drill.mjs [options]',
    '',
    'Runs a provider-backed script extension drill:',
    '- registers Python and TypeScript script extensions',
    '- grants both scripts to real provider agents',
    '- prompts each agent to call both script tools with fixed inputs',
    '- verifies hidden tokens returned by plain run return values',
    '- verifies the agent writes the observed values through Workspace Live Sync',
    '',
    `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
    '  --provider PROVIDER',
    `  --model ${DEFAULT_MODEL}`,
    '  --provider-model PROVIDER=MODEL',
    `  --effort low`,
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    '  --keep-artifacts-on-failure',
  ].join('\n'))
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function log(message, details) {
  if (details === undefined) console.log(`[script-agent-drill] ${message}`)
  else console.log(`[script-agent-drill] ${message}`, JSON.stringify(details))
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
  return result.code === 0 ? result.stdout.trim() : ''
}

async function resolveBinary(binaryPath, manifestPath, binName) {
  const result = await run('cargo', ['build', '--manifest-path', manifestPath, '--bin', binName])
  if (result.code !== 0) throw new Error(`${binName} build failed\n${result.stdout}\n${result.stderr}`)
  await access(binaryPath)
  return binaryPath
}

function makePorts() {
  const base = 61000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort: base,
    mcpPort: base + 1000,
    opencodePort: base + 2000,
    codexPort: base + 2001,
  }
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
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(5000)])
  if (child.exitCode === null && child.signalCode === null) {
    child.kill('SIGKILL')
    await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(1000)])
  }
}

async function waitForDaemon(kernelUrl, workspace) {
  const deadline = Date.now() + 30_000
  let lastError = null
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      const created = await client.send(requests.createSessionRequest(workspace, workspace, `script-agent-smoke-${Date.now()}`))
      const session = unwrap(created, 'SessionCreated').session
      await client.send(requests.endSessionRequest(session.id)).catch(() => {})
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

function unwrapOne(response, ...keys) {
  for (const key of keys) {
    if (response?.[key]) return response[key]
  }
  throw new Error(`expected one of ${keys.join(', ')}, got ${JSON.stringify(response)}`)
}

function modelForProvider(provider, options) {
  const explicit = options.providerModels[provider]
  if (explicit) return explicit
  if (provider === 'claude' || provider === 'claude-p' || provider === 'claude-headless') return 'sonnet'
  if (provider === 'opencode' && !options.model.includes('/')) return `opencode/${options.model}`
  return options.model
}

async function createScripts(root, tokens) {
  const pythonScript = path.join(root, 'python_vector_lookup.py')
  const tsScript = path.join(root, 'ts_sdk_lookup.ts')
  await writeFile(pythonScript, `
def run(query: str, limit: int = 2) -> dict[str, object]:
    """Lookup deterministic Python vector DB rows for the query."""
    return {
        "source": "python",
        "token": ${JSON.stringify(tokens.python)},
        "query": query,
        "limit": limit,
        "matches": [
            {"id": "py-doc-1", "score": 0.91, "text": "alpha python result"},
            {"id": "py-doc-2", "score": 0.82, "text": "beta python result"},
        ][:limit],
    }

def test_run() -> None:
    result = run("alpha", 1)
    assert result["source"] == "python"
    assert result["token"] == ${JSON.stringify(tokens.python)}
    assert result["matches"][0]["id"] == "py-doc-1"
`, 'utf8')
  await writeFile(tsScript, `
/**
 * Call a deterministic TypeScript SDK fixture and return account metadata.
 */
export async function run(accountId: string, multiplier: number = 3): Promise<Record<string, unknown>> {
  return {
    source: "typescript",
    token: ${JSON.stringify(tokens.typescript)},
    accountId,
    multiplied: multiplier * 7,
    records: [
      { id: "ts-rec-1", label: "gamma typescript result" },
      { id: "ts-rec-2", label: "delta typescript result" }
    ]
  }
}

export async function test_run(): Promise<void> {
  const result = await run("acct-demo", 2)
  if (result.source !== "typescript") throw new Error("bad source")
  if (result.token !== ${JSON.stringify(tokens.typescript)}) throw new Error("bad token")
  if (result.multiplied !== 14) throw new Error("bad multiplier")
}
`, 'utf8')
  return { pythonScript, tsScript }
}

async function prepareNodePackage(packageRoot) {
  await mkdir(packageRoot, { recursive: true })
  await writeFile(path.join(packageRoot, 'package.json'), '{"type":"module","dependencies":{"tsx":"latest"}}\n', 'utf8')
  const result = await run('npm', ['install', '--silent'], { cwd: packageRoot })
  if (result.code !== 0) throw new Error(`npm install tsx failed\n${result.stdout}\n${result.stderr}`)
}

async function registerScripts(client, workspace, root, tokens, scriptNames) {
  const python = process.env.PYTHON || await commandPath('python3') || await commandPath('python')
  const node = process.env.NODE || await commandPath('node')
  if (!python) throw new Error('python3 or python is required')
  if (!node) throw new Error('node is required')
  const packageRoot = path.join(root, 'node-env')
  await prepareNodePackage(packageRoot)
  const { pythonScript, tsScript } = await createScripts(root, tokens)

  const pyEnv = unwrap(await client.send(requests.registerEnvironmentRequest(workspace, {
    name: 'py-agent-drill',
    runtime: { type: 'python', python },
  })), 'EnvironmentRegistered').environment
  const nodeEnv = unwrap(await client.send(requests.registerEnvironmentRequest(workspace, {
    name: 'node-agent-drill',
    runtime: { type: 'node', node, package_root: packageRoot },
  })), 'EnvironmentRegistered').environment
  const pyScript = unwrap(
    await client.send(requests.registerScriptRequest(workspace, pythonScript, pyEnv.name, scriptNames.python)),
    'ScriptRegistered',
  ).script
  const tsScriptMeta = unwrap(
    await client.send(requests.registerScriptRequest(workspace, tsScript, nodeEnv.name, scriptNames.typescript)),
    'ScriptRegistered',
  ).script
  return { pyEnv, nodeEnv, pyScript, tsScript: tsScriptMeta }
}

async function waitForProviderRunReady(client, providerRunId, timeoutMs) {
  const deadline = Date.now() + Math.min(timeoutMs, 90_000)
  let lastRun = null
  while (Date.now() < deadline) {
    lastRun = unwrap(await client.send(requests.getProviderRunRequest(providerRunId)), 'ProviderRun').provider_run
    if (lastRun?.state === 'Running') return lastRun
    if (lastRun?.state === 'Ended') throw new Error(`provider run ended before ready: ${JSON.stringify(lastRun)}`)
    await sleep(500)
  }
  throw new Error(`timed out waiting for provider run ready\n${JSON.stringify(lastRun)}`)
}

async function waitForPromptDone(client, sessionId, attachmentId, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastState = null
  while (Date.now() < deadline) {
    await client.send(requests.pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    const response = await client.send(requests.getSessionStateRequest(sessionId))
    lastState = response.SessionState?.session ?? response.SessionStateLoaded?.session ?? null
    if (!lastState?.active_prompt) return lastState
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for prompt completion\n${JSON.stringify(lastState)}`)
}

async function waitForFile(file, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      return await readFile(file, 'utf8')
    } catch {
      await sleep(pollMs)
    }
  }
  throw new Error(`timed out waiting for ${file}`)
}

async function runProviderScenario({ client, session, attachment, workspace, outputsDir, provider, options, pyEnv, nodeEnv, tokens, scriptNames }) {
  const model = modelForProvider(provider, options)
  const agent = unwrap(
    await client.send(requests.spawnAgentRequest(session.id, provider, `${provider}-script-extension`, model, workspace, options.effort)),
    'AgentSpawned',
  ).agent
  await client.send(requests.grantAgentExtensionRequest(workspace, agent.id, 'script', scriptNames.python, pyEnv.name))
  await client.send(requests.grantAgentExtensionRequest(workspace, agent.id, 'script', scriptNames.typescript, nodeEnv.name))
  const launch = unwrapOne(
    await client.send(requests.launchProviderRunRequest(session.id, provider, 'default', model, options.effort, agent.id)),
    'ProviderRunLaunched',
    'ProviderRunLaunchAccepted',
  ).provider_run
  await waitForProviderRunReady(client, launch.id, options.timeoutMs)
  const outputRel = `outputs/${provider}-script-extension-result.json`
  const outputPath = path.join(outputsDir, `${provider}-script-extension-result.json`)
  const prompt = [
    'This is a Chariox script extension end-to-end drill.',
    `You have exactly two relevant script tools: \`${scriptNames.python}\` and \`${scriptNames.typescript}\`.`,
    `Call \`${scriptNames.python}\` with {"query":"alpha","limit":1}.`,
    `Call \`${scriptNames.typescript}\` with {"accountId":"acct-42","multiplier":3}.`,
    `Then write ${outputRel} using Chariox workspace live sync as one JSON object with these keys:`,
    'python_source, python_query, python_token, python_first_id, typescript_source, typescript_account, typescript_token, typescript_multiplied, typescript_first_id.',
    'Use only values returned by the script tools. Do not use placeholders and do not guess token values. Reply exactly SCRIPT_EXTENSION_AGENT_DRILL_DONE.',
  ].join('\n')
  await client.send(requests.submitPromptRequest(session.id, attachment.id, agent.id, prompt, []))
  await waitForPromptDone(client, session.id, attachment.id, options.timeoutMs, options.pollMs)
  const fileText = await waitForFile(outputPath, options.timeoutMs, options.pollMs)
  const parsed = JSON.parse(fileText)
  const expected = {
    python_source: 'python',
    python_query: 'alpha',
    python_token: tokens.python,
    python_first_id: 'py-doc-1',
    typescript_source: 'typescript',
    typescript_account: 'acct-42',
    typescript_token: tokens.typescript,
    typescript_multiplied: 21,
    typescript_first_id: 'ts-rec-1',
  }
  const mismatches = Object.entries(expected).filter(([key, value]) => parsed?.[key] !== value)
  const extraKeys = Object.keys(parsed ?? {}).filter((key) => !(key in expected))
  if (mismatches.length > 0 || extraKeys.length > 0) {
    throw new Error(`${provider} wrote unexpected result\nexpected=${JSON.stringify(expected)}\nactual=${JSON.stringify(parsed)}`)
  }
  log('provider-pass', { provider, agentId: agent.id, output: outputRel })
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  if (options.providers.length === 0) throw new Error('at least one provider is required')

  const root = path.join(tmpdir(), 'chariox-live-script-extension-agent-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(root, 'workspace')
  const outputsDir = path.join(workspace, 'outputs')
  const historyDir = path.join(root, 'history')
  const configHome = path.join(root, 'config')
  const stateHome = path.join(root, 'state')
  const tokens = {
    python: `py-token-${process.pid}-${Date.now()}`,
    typescript: `ts-token-${process.pid}-${Date.now()}`,
  }
  const scriptSuffix = `${process.pid}_${Date.now()}`
  const scriptNames = {
    python: `python_vector_lookup_${scriptSuffix}`,
    typescript: `ts_sdk_lookup_${scriptSuffix}`,
  }
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  let daemon = null
  let client = null
  let succeeded = false
  let failure = null
  let sessionId = null
  let attachmentId = null
  const completedProviders = []
  try {
    await prepareDrillArtifacts(root)
    await mkdir(outputsDir, { recursive: true })
    await mkdir(path.join(configHome, 'chariox'), { recursive: true })
    await writeFile(path.join(configHome, 'chariox', 'config.toml'), 'version = 1\n', 'utf8')
    await writeFile(path.join(workspace, 'README.md'), '# script extension agent drill\n', 'utf8')
    const kernelBinary = await resolveBinary(
      path.join(repoRoot, 'apps/kernel/target/debug/chariox-kernel'),
      path.join(repoRoot, 'apps/kernel/Cargo.toml'),
      'chariox-kernel',
    )
    daemon = startDaemon(kernelBinary, {
      ...process.env,
      HOME: realHomeDir,
      CODEX_HOME: process.env.CODEX_HOME ?? path.join(realHomeDir, '.codex'),
      OPENCODE_CONFIG_DIR: process.env.OPENCODE_CONFIG_DIR ?? path.join(realHomeDir, '.config', 'opencode'),
      XDG_DATA_HOME: process.env.XDG_DATA_HOME ?? path.join(realHomeDir, '.local', 'share'),
      XDG_CACHE_HOME: process.env.XDG_CACHE_HOME ?? path.join(realHomeDir, '.cache'),
      XDG_CONFIG_HOME: configHome,
      XDG_STATE_HOME: stateHome,
      CHARIOX_KERNEL_PORT: String(ports.kernelPort),
      CHARIOX_MCP_PORT: String(ports.mcpPort),
      CHARIOX_OPENCODE_PORT: String(ports.opencodePort),
      CHARIOX_CODEX_PORT: String(ports.codexPort),
      CHARIOX_DAEMON_ID: `script-agent-drill-${process.pid}-${Date.now()}`,
      CHARIOX_DAEMON_SOCKET: path.join(root, 'daemon.sock'),
      CHARIOX_SESSION_HISTORY_DIR: historyDir,
    })
    await waitForDaemon(kernelUrl, workspace)
    client = new LocalIpcClient(kernelUrl)
    const { pyEnv, nodeEnv } = await registerScripts(client, workspace, root, tokens, scriptNames)
    const session = unwrap(
      await client.send(requests.createSessionRequest(workspace, workspace, 'script-extension-agent-drill')),
      'SessionCreated',
    ).session
    sessionId = session.id
    const attachment = unwrap(
      await client.send(requests.attachToSessionRequest(session.id, `script-agent-drill-${process.pid}`)),
      'SessionAttached',
    ).attachment
    attachmentId = attachment.id
    for (const provider of options.providers) {
      await runProviderScenario({ client, session, attachment, workspace, outputsDir, provider, options, pyEnv, nodeEnv, tokens, scriptNames })
      completedProviders.push(provider)
    }
    succeeded = true
    log('pass', { providers: options.providers, workspace })
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
        drill: 'script-extension-agent',
        providers: options.providers,
        completedProviders,
        workspace,
        kernelUrl,
        sessionId,
        attachmentId,
        scriptNames,
        daemonStdoutTail: daemon?.stdoutText?.slice(-2000) ?? '',
        daemonStderrTail: daemon?.stderrText?.slice(-4000) ?? '',
      },
      log,
    })
    if (!succeeded && options.keepArtifactsOnFailure) log('artifacts-kept', { root })
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
