#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { access, mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const artifactsDir = path.join(repoRoot, '.artifacts')

const DEFAULT_MODEL = 'gpt-5.2'
const DEFAULT_PROVIDERS = ['opencode']
const DEFAULT_TIMEOUT_MS = 420_000
const DEFAULT_POLL_MS = 1_000

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrapVariant = (resp, ...keys) => keys.map((key) => resp?.[key]).find((value) => value != null) ?? resp

function parseArgs(argv) {
  const options = {
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
    providerModels: {},
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    keepArtifactsOnFailure: false,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--') continue
    else if (arg === '--provider') options.providers = [argv[++i]]
    else if (arg === '--providers') options.providers = argv[++i].split(',').map((value) => value.trim()).filter(Boolean)
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++i].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    } else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help') {
      console.log([
        'Usage: node apps/cli/scripts/live-runtime-register-mcp-use-drill.mjs [options]',
        '',
        'Prompts live Arroba agents to register, grant, and use the public Playwright MCP.',
        '',
        'Options:',
        `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
        `  --model ${DEFAULT_MODEL}`,
        '  --provider-model PROVIDER=MODEL',
        `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
        `  --poll-ms ${DEFAULT_POLL_MS}`,
        '  --keep-artifacts-on-failure',
      ].join('\n'))
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  if (options.providers.length === 0) throw new Error('at least one provider is required')
  return options
}

function modelForProvider(provider, options) {
  const explicit = options.providerModels[provider]
  if (explicit) return explicit
  if (provider === 'opencode' && !options.model.includes('/')) return `opencode/${options.model}`
  return options.model
}

async function run(command, args, options = {}) {
  return await new Promise((resolve) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => { stdout += chunk.toString() })
    child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
    child.on('error', (error) => resolve({ code: -1, stdout, stderr: `${stderr}${error.message}` }))
    child.on('close', (code, signal) => resolve({ code: code ?? -1, signal, stdout, stderr }))
  })
}

async function resolveKernelBinary() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  try {
    await access(binary)
    return binary
  } catch {
    const result = await run('cargo', [
      'build',
      '--manifest-path',
      path.join(repoRoot, 'apps/kernel/Cargo.toml'),
      '--bin',
      'arroba-kernel',
    ])
    if (result.code !== 0) throw new Error(`kernel build failed\n${result.stdout}\n${result.stderr}`)
    return binary
  }
}

async function terminateChild(child) {
  if (!child || child.exitCode != null) return
  child.kill('SIGTERM')
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(5_000)])
  if (child.exitCode == null) {
    child.kill('SIGKILL')
    await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(2_000)])
  }
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

async function waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace) {
  let lastError = null
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl)
    try {
      const created = unwrapVariant(
        await probe.send(createSessionRequest(
          workspace,
          workspace,
          `m16-mcp-probe-${attempt}`,
          undefined,
          undefined,
          'unrestricted',
        )),
        'SessionCreated',
      )
      await probe.send(endSessionRequest(created.session.id)).catch(() => {})
      await probe.close()
      return
    } catch (error) {
      lastError = error
      await probe.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`daemon did not become ready: ${lastError?.message ?? String(lastError)}`)
}

async function waitForActiveProviderRun({ client, sessionId, providerRunId, getSessionStateRequest, getProviderRunRequest, timeoutMs, pollMs }) {
  const started = Date.now()
  let lastActiveRunId = null
  while (Date.now() - started < timeoutMs) {
    const runResponse = await client.send(getProviderRunRequest(providerRunId)).catch(() => null)
    const run = unwrapVariant(runResponse, 'ProviderRun')?.provider_run
    const runState = String(run?.state ?? '').toLowerCase()
    if (runState === 'ended' || runState === 'failed' || runState === 'error') {
      throw new Error(`provider run ${providerRunId} ended before becoming active`)
    }
    const state = unwrapVariant(await client.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const session = state.session ?? state
    lastActiveRunId = session.active_provider_run_id ?? null
    if (lastActiveRunId === providerRunId) return run
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for provider run ${providerRunId}; last active run was ${lastActiveRunId ?? 'none'}`)
}

async function launchProviderRunAndWait({
  client,
  sessionId,
  agentId,
  provider,
  model,
  launchProviderRunRequest,
  getSessionStateRequest,
  getProviderRunRequest,
  timeoutMs,
  pollMs,
}) {
  const accepted = unwrapVariant(
    await client.send(launchProviderRunRequest(sessionId, provider, 'default', model, 'low', agentId)),
    'ProviderRunLaunchAccepted',
    'ProviderRunLaunched',
  )
  const run = accepted.provider_run
  await waitForActiveProviderRun({
    client,
    sessionId,
    providerRunId: run.id,
    getSessionStateRequest,
    getProviderRunRequest,
    timeoutMs: Math.min(timeoutMs, 90_000),
    pollMs,
  })
  return run
}

async function waitForAgentPromptIdle({ client, sessionId, agentId, getSessionStateRequest, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const state = unwrapVariant(await client.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const session = state.session ?? state
    const promptState = session.prompt_states?.[agentId]
    const activePrompt = promptState?.active_prompt ?? (session.active_prompt?.target_agent_id === agentId ? session.active_prompt : null)
    const queuedPrompts = promptState?.queued_prompts ?? (session.queued_prompts ?? []).filter((prompt) => prompt.target_agent_id === agentId)
    if (!activePrompt && queuedPrompts.length === 0) return
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for agent ${agentId} prompt queue to become idle`)
}

async function waitForHistoryToolCall({ historyDir, agentId, sinceMs, timeoutMs, pollMs, predicate }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const files = (await readdir(historyDir).catch(() => []))
      .filter((file) => file.endsWith('.jsonl'))
      .map((file) => path.join(historyDir, file))
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
        if ((entry.timestamp_ms ?? 0) < sinceMs || entry.agent_id !== agentId) continue
        if (entry.kind !== 'provider_tool' || typeof entry.text !== 'string') continue
        let update
        try {
          update = JSON.parse(entry.text)
        } catch {
          continue
        }
        if (predicate(update)) return update
      }
    }
    await sleep(pollMs)
  }
  throw new Error('timed out waiting for expected provider tool call in history')
}

async function providerTranscript({ historyDir, agentId, sinceMs }) {
  const entries = []
  const files = (await readdir(historyDir).catch(() => []))
    .filter((file) => file.endsWith('.jsonl'))
    .map((file) => path.join(historyDir, file))
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
      if ((entry.timestamp_ms ?? 0) < sinceMs || entry.agent_id !== agentId) continue
      entries.push(entry)
    }
  }
  entries.sort((a, b) => (a.timestamp_ms ?? 0) - (b.timestamp_ms ?? 0))
  return entries
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

function toolName(update) {
  return String(update?.tool ?? '').toLowerCase()
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const [{ LocalIpcClient }, requests] = await Promise.all([
    import('../../../packages/kernel-client/dist/ipc.js'),
    import('../../../packages/kernel-client/dist/ipc-requests.js'),
  ])

  const rootDir = path.join(repoRoot, 'target', 'live-runtime-register-mcp-use-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const historyDir = path.join(rootDir, 'history')
  const capabilityRoot = path.join(rootDir, 'capabilities')
  const outputsDir = path.join(workspace, 'outputs')
  await mkdir(outputsDir, { recursive: true })
  await writeFile(path.join(workspace, 'README.md'), '# M16 external MCP live drill workspace\n', 'utf8')

  const {
    attachToSessionRequest,
    createSessionRequest,
    endSessionRequest,
    getProviderRunRequest,
    getSessionStateRequest,
    launchProviderRunRequest,
    listProviderProcessesRequest,
    spawnAgentRequest,
    submitPromptRequest,
    teardownProviderProcessesRequest,
  } = requests

  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const kernel = await resolveKernelBinary()
  const daemon = spawn(kernel, [], {
    cwd: repoRoot,
    env: {
      ...process.env,
      ARROBA_KERNEL_PORT: String(ports.kernelPort),
      ARROBA_MCP_PORT: String(ports.mcpPort),
      ARROBA_OPENCODE_PORT: String(ports.opencodePort),
      ARROBA_CODEX_PORT: String(ports.codexPort),
      ARROBA_DAEMON_ID: `m16-runtime-mcp-use-${process.pid}-${Date.now()}`,
      ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
      ARROBA_SESSION_HISTORY_DIR: historyDir,
      ARROBA_CAPABILITY_ISOLATION_ROOT: capabilityRoot,
    },
    stdio: ['ignore', 'ignore', 'inherit'],
  })

  const results = []
  let client = null
  let succeeded = false
  try {
    await waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace)
    client = new LocalIpcClient(kernelUrl)
    const created = unwrapVariant(
      await client.send(createSessionRequest(
        workspace,
        workspace,
        `m16-external-mcp-${Date.now()}`,
        undefined,
        undefined,
        'unrestricted',
      )),
      'SessionCreated',
    )
    const session = created.session
    const attachment = unwrapVariant(
      await client.send(attachToSessionRequest(session.id, `m16-external-mcp-${Date.now()}`)),
      'SessionAttached',
    ).attachment

    for (const provider of options.providers) {
      const agent = unwrapVariant(
        await client.send(spawnAgentRequest(
          session.id,
          provider,
          `${provider}-m16-register-playwright-mcp`,
          modelForProvider(provider, options),
          workspace,
          'low',
        )),
        'AgentSpawned',
      ).agent
      await launchProviderRunAndWait({
        client,
        sessionId: session.id,
        agentId: agent.id,
        provider,
        model: modelForProvider(provider, options),
        launchProviderRunRequest,
        getSessionStateRequest,
        getProviderRunRequest,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })

      const startedAt = Date.now()
      await client.send(submitPromptRequest(session.id, attachment.id, agent.id, [
        'This is an M16 live external MCP validation drill.',
        'You must install a third-party MCP yourself through Arroba runtime MCP.',
        'First call `arroba.register_mcp` with this exact JSON argument:',
        '{"config":{"name":"m16_playwright_external","transport":{"type":"stdio","command":"npx","args":["-y","@playwright/mcp@latest"]},"enabled":true,"required":true,"startup_timeout_sec":45,"tool_timeout_sec":45}}',
        'Then call `arroba.request_extension` with {"kind":"mcp","name":"m16_playwright_external"}.',
        'After Arroba reloads the provider conversation and sends the continuation, use the provider-native Playwright/browser MCP tool, not Arroba request_extension.',
        'The Playwright tool may be named `playwright_browser_snapshot`, `mcp__m16_playwright_external__browser_snapshot`, `browser_snapshot`, `playwright_browser_navigate`, or similar.',
        'A non-mutating snapshot/title/text call is enough. Navigating to https://example.com is optional.',
        'When a provider-native Playwright/browser MCP call completes successfully, reply exactly M16_EXTERNAL_PLAYWRIGHT_MCP_DONE.',
        'If the MCP remains unavailable after continuation, reply exactly M16_EXTERNAL_PLAYWRIGHT_MCP_UNAVAILABLE.',
      ].join('\n'), []))

      const registerCall = await waitForHistoryToolCall({
        historyDir,
        agentId: agent.id,
        sinceMs: startedAt,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        predicate: (update) => toolName(update).endsWith('register_mcp') &&
          update.status === 'completed' &&
          update.input?.config?.name === 'm16_playwright_external',
      })
      const requestCall = await waitForHistoryToolCall({
        historyDir,
        agentId: agent.id,
        sinceMs: startedAt,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        predicate: (update) => toolName(update).endsWith('request_extension') &&
          update.status === 'completed' &&
          update.input?.kind === 'mcp' &&
          update.input?.name === 'm16_playwright_external',
      })
      const playwrightCall = await waitForHistoryToolCall({
        historyDir,
        agentId: agent.id,
        sinceMs: startedAt,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        predicate: (update) => {
          const name = toolName(update)
          return update.status === 'completed' &&
            !name.includes('arroba') &&
            (name.includes('playwright') || name.includes('browser'))
        },
      })
      await waitForAgentPromptIdle({
        client,
        sessionId: session.id,
        agentId: agent.id,
        getSessionStateRequest,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      }).catch(() => {})
      const transcript = await providerTranscript({ historyDir, agentId: agent.id, sinceMs: startedAt })
      await writeFile(
        path.join(artifactsDir, `runtime-register-external-mcp-${provider}-history.json`),
        JSON.stringify(transcript, null, 2),
        'utf8',
      )
      await renderTerminalScreenshot(`runtime-register-external-mcp-${provider}.png`, `M16 External MCP Live Use (${provider})`, [
        'PASS provider agent called arroba.register_mcp for @playwright/mcp',
        'PASS provider agent called arroba.request_extension for the registered MCP',
        `PASS request_extension completed with tool ${requestCall.tool}`,
        `PASS provider-native MCP tool completed: ${playwrightCall.tool}`,
        'PASS external credentialless MCP was installed, granted, reloaded, and used',
      ])
      results.push({
        provider,
        agentId: agent.id,
        registerTool: registerCall.tool,
        requestTool: requestCall.tool,
        providerNativeTool: playwrightCall.tool,
      })
      await client.send(teardownProviderProcessesRequest(provider, true)).catch(() => {})
      const deadline = Date.now() + 60_000
      while (Date.now() < deadline) {
        const processes = unwrapVariant(await client.send(listProviderProcessesRequest(provider)), 'ProviderProcessesListed').processes ?? []
        if (!processes.some((process) => process.provider === provider)) break
        await sleep(options.pollMs)
      }
    }

    await writeFile(
      path.join(artifactsDir, 'runtime-register-external-mcp-live-use.json'),
      JSON.stringify({ ok: true, kernelUrl, workspace, results }, null, 2),
      'utf8',
    )
    console.log(JSON.stringify({ ok: true, kernelUrl, workspace, results }, null, 2))
    succeeded = true
    await client.send(endSessionRequest(session.id)).catch(() => {})
  } finally {
    if (client) await client.close().catch(() => {})
    await terminateChild(daemon)
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    } else {
      console.error(`Keeping drill artifacts for debugging: ${rootDir}`)
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
