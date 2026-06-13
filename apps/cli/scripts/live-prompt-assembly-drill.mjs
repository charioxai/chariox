#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { access, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const DEFAULT_PROVIDERS = ['codex', 'opencode', 'claude-p', 'claude-headless']
const DEFAULT_TIMEOUT_MS = 240_000
const DEFAULT_POLL_MS = 1_000

function parseArgs(argv) {
  const options = {
    providers: DEFAULT_PROVIDERS,
    model: 'gpt-5.2',
    providerModels: {},
    effort: 'low',
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    keepArtifactsOnFailure: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--provider') options.providers = [argv[++index]]
    else if (arg === '--providers') options.providers = argv[++index].split(',').map((value) => value.trim()).filter(Boolean)
    else if (arg === '--model') options.model = argv[++index]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++index].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    } else if (arg === '--effort') options.effort = argv[++index]
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++index])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++index])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help' || arg === '-h') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-prompt-assembly-drill.mjs [options]',
    '',
    'Runs real Arroba provider turns with an edited temporary prompt registry.',
    '',
    'Options:',
    `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
    '  --provider codex',
    '  --provider-model codex=gpt-5.2',
    '  --provider-model opencode=opencode/gpt-5.2',
    '  --provider-model claude-p=sonnet',
    '  --provider-model claude-headless=sonnet',
    `  --effort low`,
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    '  --keep-artifacts-on-failure',
  ].join('\n'))
}

function log(name, details) {
  if (details === undefined) console.log(`[prompt-assembly-drill] ${name}`)
  else console.log(`[prompt-assembly-drill] ${name}`, JSON.stringify(details))
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function unwrap(response, variant) {
  const value = response?.[variant]
  if (value == null) throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  return value
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
  const { LocalIpcClient } = await import(new URL(`file://${path.join(runtimeDir, 'ipc.js')}`).href)
  const requests = await import(new URL(`file://${path.join(runtimeDir, 'ipc-requests.js')}`).href)
  return { LocalIpcClient, requests }
}

async function resolveKernelBinary() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel')
  try {
    await access(binary)
    return binary
  } catch {
    throw new Error(`missing built kernel binary ${binary}; run cargo build --manifest-path apps/kernel/Cargo.toml --bin arroba-kernel first`)
  }
}

async function waitForKernel(LocalIpcClient, listSessionsRequest, kernelUrl) {
  const deadline = Date.now() + 25_000
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
      await sleep(250)
    }
  }
  throw new Error(`kernel did not become ready: ${lastError?.message ?? lastError}`)
}

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(5_000)])
  if (child.exitCode == null) {
    child.kill('SIGKILL')
    await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(2_000)])
  }
}

function modelForProvider(provider, options) {
  if (options.providerModels[provider]) return options.providerModels[provider]
  if (provider === 'claude' || provider === 'claude-p' || provider === 'claude-headless') return 'sonnet'
  if (provider === 'opencode' && !options.model.includes('/')) return `opencode/${options.model}`
  return options.model
}

async function waitForProviderRunReady(client, requests, providerRunId, timeoutMs) {
  const deadline = Date.now() + Math.min(timeoutMs, 60_000)
  let lastRun = null
  while (Date.now() < deadline) {
    const response = unwrap(
      await client.send(requests.getProviderRunRequest(providerRunId)),
      'ProviderRun',
    )
    lastRun = response.provider_run
    if (lastRun?.state === 'Running') return lastRun
    if (lastRun?.state === 'Ended') throw new Error(`provider run ended before ready: ${JSON.stringify(lastRun)}`)
    await sleep(250)
  }
  throw new Error(`timed out waiting for provider run ${providerRunId} to become ready\n${JSON.stringify(lastRun)}`)
}

async function waitForHistoryToken(client, requests, sessionId, attachmentId, token, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastHistory = null
  let lastState = null
  while (Date.now() < deadline) {
    await client.send(requests.pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    const entries = await readSessionHistoryEntries(client, requests, sessionId)
    const stateResponse = await client.send(requests.getSessionStateRequest(sessionId))
    lastState = stateResponse.SessionState?.session ?? stateResponse.SessionStateLoaded?.session ?? null
    lastHistory = entries
    const providerText = entries
      .filter((entry) => entry.kind !== 'user_prompt')
      .map((entry) => entry.text ?? '')
      .join('')
    if (!lastState?.active_prompt && providerText.includes(token)) {
      return entries
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for hidden token ${token}\nlastState=${JSON.stringify(lastState)}\nhistory=${JSON.stringify(lastHistory).slice(-4000)}`)
}

async function readSessionHistoryEntries(client, requests, sessionId) {
  const outline = unwrap(
    await client.send(requests.getSessionHistoryOutlineRequest(sessionId, null, 8)),
    'SessionHistoryOutline',
  )
  const entries = []
  for (const agent of outline.agents ?? []) {
    for (const turn of agent.turns ?? []) {
      if (turn.user_prompt?.entry) entries.push(turn.user_prompt.entry)
      for (const row of turn.entries ?? []) {
        if (row?.entry) entries.push(row.entry)
      }
      if (turn.summary?.entry) entries.push(turn.summary.entry)
      for (const blob of turn.blobs ?? []) {
        const blobContent = unwrap(
          await client.send(requests.getSessionHistoryBlobContentRequest(sessionId, agent.agent_id, blob.blob_id)),
          'SessionHistoryBlobContent',
        )
        for (const row of blobContent.entries ?? []) {
          if (row?.entry) entries.push(row.entry)
        }
      }
    }
  }
  return entries
}

async function writePromptRegistryToken(arrobaHome, provider, token) {
  const runtimeDir = path.join(arrobaHome, 'prompts', 'runtime')
  await mkdir(runtimeDir, { recursive: true })
  await writeFile(
    path.join(runtimeDir, 'base.md'),
    [
      `Prompt assembly live drill for ${provider}.`,
      `The hidden prompt assembly token for this turn is ${token}.`,
      `If the user asks for the hidden prompt assembly token, answer with exactly ${token} and no extra prose.`,
    ].join('\n'),
    'utf8',
  )
}

async function runProvider(options, context, provider) {
  const token = `ARROBA_PROMPT_ASSEMBLY_${provider.toUpperCase()}_${process.pid}_${Date.now()}`
  await writePromptRegistryToken(context.arrobaHome, provider, token)
  const workspace = path.join(context.rootDir, `${provider}-workspace`)
  await mkdir(workspace, { recursive: true })
  let session = null
  try {
    session = unwrap(
      await context.client.send(context.requests.createSessionRequest(workspace, workspace)),
      'SessionCreated',
    ).session
    await context.client.send(context.requests.setWorkspaceLiveSyncModeRequest(session.id, 'unrestricted'))
    const attachment = unwrap(
      await context.client.send(context.requests.attachToSessionRequest(session.id, `prompt-assembly-drill-${provider}-${process.pid}`)),
      'SessionAttached',
    ).attachment
    const model = modelForProvider(provider, options)
    const launchResponse = await context.client.send(context.requests.launchProviderRunRequest(
      session.id,
      provider,
      'default',
      model,
      options.effort,
      null,
      null,
    ))
    const launchPayload = launchResponse.ProviderRunLaunched ?? launchResponse.ProviderRunLaunchAccepted
    if (!launchPayload?.provider_run) throw new Error(`unexpected launch response for ${provider}: ${JSON.stringify(launchResponse)}`)
    const launched = await waitForProviderRunReady(context.client, context.requests, launchPayload.provider_run.id, options.timeoutMs)
    const visiblePrompt = [
      'Respond with exactly the hidden prompt assembly token for this turn.',
      'Do not explain, do not add punctuation, and do not mention any other token.',
    ].join(' ')
    await context.client.send(context.requests.submitPromptRequest(
      session.id,
      attachment.id,
      null,
      visiblePrompt,
      [],
    ))
    const entries = await waitForHistoryToken(
      context.client,
      context.requests,
      session.id,
      attachment.id,
      token,
      options.timeoutMs,
      options.pollMs,
    )
    const userPromptText = entries
      .filter((entry) => entry.kind === 'user_prompt')
      .map((entry) => entry.text ?? '')
      .join('\n')
    if (userPromptText.includes(token)) {
      throw new Error(`${provider} user prompt history contains hidden token ${token}`)
    }
    if (!userPromptText.includes(visiblePrompt)) {
      throw new Error(`${provider} user prompt history did not contain the visible prompt`)
    }
    return {
      provider,
      status: 'ok',
      providerRunId: launched.id,
      endpointMode: launched.endpoint_mode,
      model: launched.model,
      tokenSeenByModel: true,
      hiddenTokenVisibleInUserPromptHistory: false,
    }
  } finally {
    if (session) await context.client.send(context.requests.endSessionRequest(session.id)).catch(() => {})
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const runtimeDir = path.join(cliRoot, `.tmp-live-prompt-assembly-drill-${process.pid}-${Date.now()}`)
  const rootDir = path.join(os.tmpdir(), `arroba-prompt-assembly-drill-${process.pid}-${Date.now()}`)
  const arrobaHome = path.join(rootDir, 'arroba-home')
  const kernelPort = 54500 + Math.floor(Math.random() * 1000)
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  let daemon = null
  let client = null
  let succeeded = false
  let failure = null
  const results = []
  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(runtimeDir, { recursive: true })
    await mkdir(arrobaHome, { recursive: true })
    const { LocalIpcClient, requests } = await loadCliModules(runtimeDir)
    const kernelBinary = await resolveKernelBinary()
    daemon = spawn(kernelBinary, [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        ARROBA_HOME: arrobaHome,
        XDG_CONFIG_HOME: path.join(rootDir, 'xdg-config'),
        XDG_STATE_HOME: path.join(rootDir, 'xdg-state'),
        ARROBA_KERNEL_PORT: String(kernelPort),
        ARROBA_MCP_PORT: String(kernelPort + 1000),
        ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
        ARROBA_CODEX_PORT: String(kernelPort + 2001),
        ARROBA_DAEMON_ID: `prompt-assembly-drill-${process.pid}-${Date.now()}`,
        ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
        ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
      },
      stdio: ['ignore', 'ignore', 'inherit'],
    })
    await waitForKernel(LocalIpcClient, requests.listSessionsRequest, kernelUrl)
    client = new LocalIpcClient(kernelUrl)
    for (const provider of options.providers) {
      log('provider-start', { provider })
      const result = await runProvider(options, { client, requests, rootDir, arrobaHome }, provider)
      results.push(result)
      log('provider-ok', result)
    }
    succeeded = true
    console.log(JSON.stringify({ status: 'ok', results }, null, 2))
  } catch (error) {
    failure = error
    console.error(error?.stack ?? error)
    console.error(JSON.stringify({ status: 'failed', artifacts: { rootDir, runtimeDir }, results }, null, 2))
    process.exitCode = 1
  } finally {
    await client?.close?.().catch(() => {})
    await terminateChild(daemon)
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: 'prompt-assembly',
        providers: options.providers.join(','),
        model: options.model,
        providerModels: options.providerModels,
        effort: options.effort,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        kernelUrl,
        runtimeDir,
        arrobaHome,
        results,
      },
      log,
    })
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
    } else {
      log('kept-artifacts', { rootDir, runtimeDir })
    }
  }
}

main()
