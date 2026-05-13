#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { access, mkdir, readFile, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const DEFAULT_MODEL = 'sonnet'
const DEFAULT_EFFORT = 'low'
const DEFAULT_TIMEOUT_MS = 180_000
const DEFAULT_POLL_MS = 1_000

function parseArgs(argv) {
  const options = {
    kernel: null,
    workspace: null,
    worktree: null,
    model: DEFAULT_MODEL,
    effort: DEFAULT_EFFORT,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    spawnDaemon: true,
    keepArtifactsOnFailure: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--kernel') options.kernel = argv[++index]
    else if (arg === '--workspace') options.workspace = argv[++index]
    else if (arg === '--worktree') options.worktree = argv[++index]
    else if (arg === '--model') options.model = argv[++index]
    else if (arg === '--effort') options.effort = argv[++index]
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++index])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++index])
    else if (arg === '--no-spawn-daemon') options.spawnDaemon = false
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help' || arg === '-h') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-claude-provider-drill.mjs [options]',
    '',
    'Launches a real Claude Code provider through the kernel, submits one prompt,',
    'and verifies streamed output plus assistant completion through session history.',
    '',
    'Options:',
    `  --model ${DEFAULT_MODEL}`,
    `  --effort ${DEFAULT_EFFORT}`,
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    '  --kernel ws://127.0.0.1:43284',
    `  --workspace ${repoRoot}`,
    `  --worktree ${repoRoot}`,
    '  --no-spawn-daemon',
    '  --keep-artifacts-on-failure',
  ].join('\n'))
}

function log(name, details) {
  if (details === undefined) console.log(`[claude-provider-drill] ${name}`)
  else console.log(`[claude-provider-drill] ${name}`, JSON.stringify(details))
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

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
    await import('node:fs/promises').then((fs) => fs.writeFile(outPath, transformed?.code ?? '', 'utf8'))
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

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(5000)])
  if (child.exitCode == null) {
    child.kill('SIGKILL')
    await Promise.race([new Promise((resolve) => child.once('exit', resolve)), sleep(1000)])
  }
}

async function waitForHistory(client, requests, sessionId, attachmentId, expected, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastText = ''
  let lastState = null
  while (Date.now() < deadline) {
    await client.send(requests.pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    const history = unwrap(
      await client.send(requests.getSessionHistoryRequest(sessionId, 200, 100_000, null, null)),
      'SessionHistory',
    )
    const stateResponse = await client.send(requests.getSessionStateRequest(sessionId))
    lastState = stateResponse.SessionState?.session ?? stateResponse.SessionStateLoaded?.session ?? null
    const entries = history.entries.map((entry) => entry.entry).filter(Boolean)
    lastText = entries.map((entry) => entry.text ?? '').join('\n')
    const completed = !lastState?.active_prompt
    if (lastText.includes(expected) && completed) return { entries, text: lastText }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for Claude output marker ${expected}\nlastState=${JSON.stringify(lastState)}\n${lastText.slice(-4000)}`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }

  const runtimeDir = path.join(cliRoot, '.tmp-live-claude-provider-drill')
  const rootDir = path.join(os.tmpdir(), `arroba-claude-provider-${process.pid}-${Date.now()}`)
  const workspace = options.workspace ?? path.join(rootDir, 'workspace')
  const worktree = options.worktree ?? workspace
  await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(runtimeDir, { recursive: true })
  await mkdir(worktree, { recursive: true })

  const marker = `ARROBA_CLAUDE_DRILL_${process.pid}_${Date.now()}`
  const prompt = [
    'Respond with exactly this marker and no extra prose:',
    marker,
  ].join('\n')
  const { LocalIpcClient, requests } = await loadCliModules(runtimeDir)

  let daemon = null
  let client = null
  let succeeded = false
  const kernelPort = 53500 + Math.floor(Math.random() * 1000)
  const kernelUrl = options.kernel ?? `ws://127.0.0.1:${kernelPort}`
  try {
    if (options.spawnDaemon) {
      const kernelBinary = await resolveKernelBinary()
      daemon = spawn(kernelBinary, [], {
        cwd: repoRoot,
        env: {
          ...process.env,
          HOME: process.env.HOME,
          ARROBA_KERNEL_PORT: String(kernelPort),
          ARROBA_MCP_PORT: String(kernelPort + 1000),
          ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
          ARROBA_CODEX_PORT: String(kernelPort + 2001),
          ARROBA_DAEMON_ID: `claude-provider-drill-${process.pid}-${Date.now()}`,
          ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
          ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
        },
        stdio: ['ignore', 'ignore', 'inherit'],
      })
    }
    await waitForKernel(LocalIpcClient, requests.listSessionsRequest, kernelUrl)
    client = new LocalIpcClient(kernelUrl)
    const session = unwrap(
      await client.send(requests.createSessionRequest(workspace, worktree)),
      'SessionCreated',
    ).session
    const attachment = unwrap(
      await client.send(requests.attachToSessionRequest(session.id, `claude-provider-drill-${process.pid}`)),
      'SessionAttached',
    ).attachment
    const launchResponse = await client.send(requests.launchProviderRunRequest(
        session.id,
        'claude',
        'default',
        options.model,
        options.effort,
        null,
        null,
      ))
    const launchPayload = launchResponse.ProviderRunLaunched ?? launchResponse.ProviderRunLaunchAccepted
    if (!launchPayload?.provider_run) throw new Error(`unexpected launch response: ${JSON.stringify(launchResponse)}`)
    const launched = await waitForProviderRunReady(
      client,
      requests,
      launchPayload.provider_run.id,
      options.timeoutMs,
    )
    log('provider-launched', {
      providerRunId: launched.id,
      model: launched.model,
      variant: launched.variant,
      endpointMode: launched.endpoint_mode,
    })
    await client.send(requests.submitPromptRequest(session.id, attachment.id, null, prompt, []))
    const history = await waitForHistory(
      client,
      requests,
      session.id,
      attachment.id,
      marker,
      options.timeoutMs,
      options.pollMs,
    )
    log('verified', { marker, historyEntries: history.entries.length })
    await client.send(requests.endSessionRequest(session.id)).catch(() => {})
    succeeded = true
  } finally {
    await client?.close?.().catch(() => {})
    await terminateChild(daemon)
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
    } else {
      log('kept-artifacts', { runtimeDir, rootDir })
    }
  }
}

main().catch((error) => {
  console.error(error?.stack ?? error)
  process.exit(1)
})
