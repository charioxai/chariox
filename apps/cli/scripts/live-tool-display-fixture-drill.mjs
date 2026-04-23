#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { access, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const DEFAULT_PROVIDERS = ['codex', 'opencode']
const DEFAULT_MODEL = 'gpt-5.4'
const DEFAULT_TIMEOUT_MS = 360_000
const DEFAULT_POLL_MS = 1_000

function parseArgs(argv) {
  const options = {
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
    providerModels: {},
    allModels: false,
    maxModelsPerProvider: 1,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    keepArtifactsOnFailure: false,
    outDir: null,
    listTargets: false,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--provider') options.providers = [argv[++i]]
    else if (arg === '--providers') options.providers = argv[++i].split(',').map((value) => value.trim()).filter(Boolean)
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++i].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    } else if (arg === '--all-models') options.allModels = true
    else if (arg === '--max-models-per-provider') options.maxModelsPerProvider = Number(argv[++i])
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--out-dir') options.outDir = argv[++i]
    else if (arg === '--list-targets') options.listTargets = true
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-tool-display-fixture-drill.mjs [options]',
    '',
    'Runs disposable provider sessions and captures raw provider_tool events for tool-display fixtures.',
    'By default it samples one model per provider. Use --all-models to expand from the provider catalog.',
    '',
    'Options:',
    '  --provider PROVIDER',
    `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
    `  --model ${DEFAULT_MODEL}`,
    '  --provider-model PROVIDER=MODEL',
    '  --all-models',
    '  --max-models-per-provider 1',
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    `  --poll-ms ${DEFAULT_POLL_MS}`,
    '  --out-dir PATH',
    '  --list-targets',
    '  --keep-artifacts-on-failure',
  ].join('\n'))
}

async function loadCliModules(runtimeDir) {
  const [{ transformAsync }, tsPreset] = await Promise.all([
    import('@babel/core'),
    import('@babel/preset-typescript'),
  ])
  for (const rel of ['src/ipc.ts', 'src/ipc-requests.ts']) {
    const sourcePath = path.join(cliRoot, rel)
    const outPath = path.join(runtimeDir, path.basename(rel).replace(/\.tsx?$/, '.js'))
    const code = await readFile(sourcePath, 'utf8')
    const transformed = await transformAsync(code, {
      filename: sourcePath,
      presets: [[tsPreset.default ?? tsPreset]],
      sourceMaps: false,
    })
    await writeFile(outPath, transformed?.code ?? '', 'utf8')
  }
  const ipcUrl = new URL(`file://${path.join(runtimeDir, 'ipc.js')}`).href
  const requestsUrl = new URL(`file://${path.join(runtimeDir, 'ipc-requests.js')}`).href
  const { LocalIpcClient } = await import(ipcUrl)
  const requests = await import(requestsUrl)
  return { LocalIpcClient, requests }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (resp, key) => resp?.[key] ?? resp
const unwrapVariant = (resp, ...keys) => keys.map((key) => resp?.[key]).find((value) => value != null) ?? resp

function makePorts() {
  const base = 59000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort: base,
    mcpPort: base + 1000,
    opencodePort: base + 2000,
    codexPort: base + 2001,
  }
}

async function resolveBinary(binaryPath, manifestPath, binName) {
  try {
    await access(binaryPath)
    return binaryPath
  } catch {
    throw new Error(`missing built binary ${binaryPath}; run cargo build --manifest-path ${manifestPath} --bin ${binName} first`)
  }
}

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    sleep(5_000),
  ])
  if (child.exitCode == null) {
    child.kill('SIGKILL')
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      sleep(2_000),
    ])
  }
}

async function waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl)
    try {
      const session = unwrap(await probe.send(createSessionRequest(workspace, workspace)), 'SessionCreated').session
      await probe.send(endSessionRequest(session.id)).catch(() => {})
      await probe.close()
      return
    } catch {
      await probe.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error('daemon did not become ready')
}

function modelForProvider(provider, options) {
  const explicit = options.providerModels[provider]
  if (explicit) return explicit
  if (provider === 'opencode' && !options.model.includes('/')) return `openai/${opencodeCodexModel(options.model)}`
  if (provider === 'codex' && !options.model.includes('/')) return opencodeCodexModel(options.model)
  return options.model
}

function opencodeCodexModel(model) {
  if (model.endsWith('-codex')) return model
  if (/^gpt-5\.[23]$/.test(model)) return `${model}-codex`
  return model
}

function drillTargets(catalog, options) {
  if (!options.allModels) {
    return options.providers.map((provider) => ({ provider, model: modelForProvider(provider, options) }))
  }
  const connected = new Set(catalog.connected ?? [])
  const targets = []
  for (const provider of catalog.all ?? []) {
    if (!options.providers.includes(provider.id) || !connected.has(provider.id)) continue
    const models = Object.values(provider.models ?? {})
      .filter((model) => model.status !== 'deprecated')
      .slice(0, options.maxModelsPerProvider)
    for (const model of models) {
      targets.push({ provider: provider.id, model: model.id })
    }
  }
  return targets
}

async function waitForAgentIdle({ client, requests, sessionId, attachmentId, agentId, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send(requests.pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    const state = unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const session = state.session ?? state
    const agent = (session.agents ?? []).find((candidate) => candidate.id === agentId)
    const promptState = (session.prompt_states ?? {})[agentId] ?? {}
    if (agent && !agent.is_processing && agent.state !== 'Working' && promptState.active_prompt == null && (promptState.queued_prompts ?? []).length === 0) {
      return
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${agentId} to become idle`)
}

async function collectProviderToolEvents(historyDir, agentId) {
  const events = []
  const files = await listJsonlFiles(historyDir)
  for (const file of files) {
    const lines = (await readFile(file, 'utf8').catch(() => '')).split(/\r?\n/)
    for (const line of lines) {
      if (!line.trim()) continue
      let entry
      try {
        entry = JSON.parse(line)
      } catch {
        continue
      }
      if (entry.agent_id && entry.agent_id !== agentId) continue
      if (entry.kind === 'provider_tool' && typeof entry.text === 'string') {
        events.push(entry.text)
      }
    }
  }
  return [...new Set(events)]
}

async function listJsonlFiles(root) {
  const files = []
  const entries = await readdir(root, { withFileTypes: true }).catch(() => [])
  for (const entry of entries) {
    const fullPath = path.join(root, entry.name)
    if (entry.isDirectory()) {
      files.push(...await listJsonlFiles(fullPath))
    } else if (entry.isFile() && entry.name.endsWith('.jsonl')) {
      files.push(fullPath)
    }
  }
  return files
}

function fixtureFileName(provider, model) {
  return `${provider}-${model.replace(/[^a-zA-Z0-9_.-]+/g, '_')}.jsonl`
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }

  const ports = makePorts()
  const runtimeDir = path.join(cliRoot, '.tmp-live-tool-display-fixture-drill')
  const rootDir = path.join(cliRoot, 'target', 'live-tool-display-fixture-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const historyDir = path.join(rootDir, 'history')
  const outDir = options.outDir ? path.resolve(options.outDir) : path.join(repoRoot, 'target', 'tool-display-fixtures')
  await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(runtimeDir, { recursive: true })
  await mkdir(workspace, { recursive: true })
  await mkdir(outDir, { recursive: true })
  await writeFile(path.join(workspace, 'seed.txt'), 'TOOL_DISPLAY_FIXTURE_SEED\n', 'utf8')
  await writeFile(path.join(workspace, 'patch-target.txt'), 'before\n', 'utf8')

  const { LocalIpcClient, requests } = await loadCliModules(runtimeDir)
  const daemonBinary = await resolveBinary(
    path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel'),
    path.join(repoRoot, 'apps/kernel/Cargo.toml'),
    'arroba-kernel',
  )
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const daemonChild = spawn(daemonBinary, [], {
    cwd: repoRoot,
    env: {
      ...process.env,
      ARROBA_KERNEL_PORT: String(ports.kernelPort),
      ARROBA_MCP_PORT: String(ports.mcpPort),
      ARROBA_OPENCODE_PORT: String(ports.opencodePort),
      ARROBA_CODEX_PORT: String(ports.codexPort),
      ARROBA_DAEMON_ID: `tool-display-fixtures-${process.pid}-${Date.now()}`,
      ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
      ARROBA_SESSION_HISTORY_DIR: historyDir,
    },
    stdio: ['ignore', 'ignore', 'inherit'],
  })

  let succeeded = false
  let client = null
  let sessionId = null
  try {
    await waitForLocalDaemon(LocalIpcClient, kernelUrl, requests.createSessionRequest, requests.endSessionRequest, workspace)
    client = new LocalIpcClient(kernelUrl)
    const catalog = unwrapVariant(await client.send(requests.getProviderCatalogRequest()), 'ProviderCatalog').catalog
    const targets = drillTargets(catalog, options)
    if (targets.length === 0) throw new Error('no provider/model targets selected')
    if (options.listTargets) {
      for (const target of targets) {
        console.log(`[tool-display-fixture] target ${target.provider} ${target.model}`)
      }
      succeeded = true
      return
    }

    const session = unwrap(await client.send(requests.createSessionRequest(workspace, workspace)), 'SessionCreated').session
    sessionId = session.id
    const attachment = unwrap(await client.send(requests.attachToSessionRequest(session.id, `tool-display-fixtures-${Date.now()}`)), 'SessionAttached').attachment

    for (const target of targets) {
      const agent = unwrapVariant(
        await client.send(requests.spawnAgentRequest(
          session.id,
          target.provider,
          `${target.provider}-tool-display-fixture`,
          target.model,
          workspace,
          'low',
        )),
        'AgentSpawned',
      ).agent
      await client.send(requests.submitPromptRequest(session.id, attachment.id, agent.id, [
        'This is an Arroba tool-display fixture drill.',
        'Use each available tool at most once and keep changes inside this disposable workspace.',
        '1. Read seed.txt.',
        '2. Search for TOOL_DISPLAY_FIXTURE_SEED in this workspace.',
        '3. Run a harmless shell command that prints TOOL_DISPLAY_SHELL_OK.',
        '4. Use apply_patch to change patch-target.txt from before to after.',
        'Then reply with TOOL_DISPLAY_FIXTURE_DONE.',
      ].join('\n'), []))
      await waitForAgentIdle({
        client,
        requests,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentId: agent.id,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
      const events = await collectProviderToolEvents(historyDir, agent.id)
      if (events.length === 0) {
        throw new Error(`${target.provider} ${target.model} produced no provider_tool events`)
      }
      const targetPath = path.join(outDir, fixtureFileName(target.provider, target.model))
      await writeFile(targetPath, `${events.join('\n')}\n`, 'utf8')
      console.log(`[tool-display-fixture] ${target.provider} ${target.model}: ${events.length} tool events -> ${targetPath}`)
    }

    await client.send(requests.endSessionRequest(session.id)).catch(() => {})
    succeeded = true
  } finally {
    await client?.close?.().catch(() => {})
    await terminateChild(daemonChild)
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
      await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
    } else {
      console.error(`[tool-display-fixture] kept artifacts at ${rootDir}`)
    }
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack || error.message : String(error))
  process.exitCode = 1
})
