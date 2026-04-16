import { spawn } from 'node:child_process'
import { access, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

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

const DEFAULT_KERNEL = 'ws://127.0.0.1:43284'
const DEFAULT_MODEL = 'gpt-5.4'
const DEFAULT_PROVIDERS = ['opencode', 'codex']
const DEFAULT_TIMEOUT_MS = 360_000
const DEFAULT_POLL_MS = 1_000

function parseArgs(argv) {
  const options = {
    kernel: DEFAULT_KERNEL,
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    spawnDaemon: true,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--kernel') options.kernel = argv[++i]
    else if (arg === '--providers') options.providers = argv[++i].split(',').map((value) => value.trim()).filter(Boolean)
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--no-spawn-daemon') options.spawnDaemon = false
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-managed-io-drill.mjs [options]',
    '',
    'Runs a live managed-I/O provider smoke test:',
    '- positive: agents read seed.txt and exercise Arroba write/edit/apply_patch/move/delete tools',
    '- negative: agents are asked to write directly without Arroba; direct output files must not appear',
    '',
    'Options:',
    `  --kernel ${DEFAULT_KERNEL}`,
    `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
    `  --model ${DEFAULT_MODEL}`,
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    `  --poll-ms ${DEFAULT_POLL_MS}`,
    '  --no-spawn-daemon',
  ].join('\n'))
}

function makePorts() {
  const base = 57000 + Math.floor(Math.random() * 1000)
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

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (resp, key) => resp?.[key] ?? resp
const unwrapVariant = (resp, ...keys) => keys.map((key) => resp?.[key]).find((value) => value != null) ?? resp

async function waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace, worktree) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl)
    try {
      const session = unwrap(await probe.send(createSessionRequest(workspace, worktree)), 'SessionCreated').session
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

async function fileExists(filePath) {
  try {
    await access(filePath)
    return true
  } catch {
    return false
  }
}

async function assertFileContent(filePath, expected) {
  const actual = await readFile(filePath, 'utf8')
  if (actual !== expected) {
    throw new Error(`unexpected content for ${filePath}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`)
  }
  return actual
}

async function waitForCompletionsAndFiles({ client, sessionId, attachmentId, events, expectedCompletionCount, requiredFiles, forbiddenFiles, timeoutMs, pollMs }) {
  const started = Date.now()
  let lastRequiredCount = 0
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    const forbiddenExisting = []
    for (const forbiddenFile of forbiddenFiles) {
      if (await fileExists(forbiddenFile)) forbiddenExisting.push(forbiddenFile)
    }
    if (forbiddenExisting.length > 0) {
      throw new Error(`direct write unexpectedly created forbidden files: ${forbiddenExisting.join(', ')}`)
    }

    const requiredExisting = []
    for (const requiredFile of requiredFiles) {
      if (await fileExists(requiredFile)) requiredExisting.push(requiredFile)
    }
    lastRequiredCount = requiredExisting.length
    const completed = events.filter((event) => event.event === 'assistant_message_completed')
    if (requiredExisting.length === requiredFiles.length && completed.length >= expectedCompletionCount) {
      return completed
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${expectedCompletionCount} completions and ${requiredFiles.length} required files; required files present=${lastRequiredCount}`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  if (options.providers.length === 0) {
    throw new Error('at least one provider is required')
  }

  const runtimeDir = path.join(cliRoot, '.tmp-live-managed-io-drill')
  const rootDir = path.join(os.tmpdir(), `arroba-managed-io-${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const outputsDir = path.join(workspace, 'outputs')
  await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(runtimeDir, { recursive: true })
  await mkdir(outputsDir, { recursive: true })
  await writeFile(path.join(workspace, 'seed.txt'), 'seed-value-42\n', 'utf8')

  const { LocalIpcClient, requests } = await loadCliModules(runtimeDir)
  const {
    attachToSessionRequest,
    createSessionRequest,
    endSessionRequest,
    getSessionStateRequest,
    listProviderProcessesRequest,
    spawnAgentRequest,
    submitPromptRequest,
  } = requests

  let daemonChild = null
  let kernelUrl = options.kernel
  const startedAt = Date.now()
  if (options.spawnDaemon) {
    const ports = makePorts()
    kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
    const daemonBinary = await resolveBinary(
      path.join(repoRoot, 'apps/daemon/target/debug/arroba-daemon'),
      path.join(repoRoot, 'apps/daemon/Cargo.toml'),
      'arroba-daemon',
    )
    daemonChild = spawn(daemonBinary, [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        ARROBA_KERNEL_PORT: String(ports.kernelPort),
        ARROBA_MCP_PORT: String(ports.mcpPort),
        ARROBA_OPENCODE_PORT: String(ports.opencodePort),
        ARROBA_CODEX_PORT: String(ports.codexPort),
        ARROBA_DAEMON_ID: `managed-io-drill-${process.pid}-${Date.now()}`,
        ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
        ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
      },
      stdio: ['ignore', 'ignore', 'inherit'],
    })
    await waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace, workspace)
  }

  const client = new LocalIpcClient(kernelUrl)
  const events = []
  let sessionId = null
  try {
    const session = unwrap(await client.send(createSessionRequest(workspace, workspace)), 'SessionCreated').session
    sessionId = session.id
    const attachment = unwrap(
      await client.send(attachToSessionRequest(session.id, `managed-io-drill-${Date.now()}`)),
      'SessionAttached',
    ).attachment
    client.onKernelEvent((event) => events.push({ ...event, observed_at_ms: Date.now() }))
    await client.subscribeToKernelEvents(session.id, attachment.id)

    const agents = []
    for (let index = 0; index < options.providers.length; index += 1) {
      const provider = options.providers[index]
      const agent = unwrapVariant(
        await client.send(spawnAgentRequest(session.id, provider, `${provider}-managed-io-${index + 1}`, options.model, workspace, 'low')),
        'AgentSpawned',
      ).agent
      agents.push({ provider, agent })
    }

    for (const { provider, agent } of agents) {
      const written = `${provider}-managed-io-write-ok: seed-value-42\n`
      const edited = `${provider}-managed-io-edit-ok: seed-value-42\n`
      const patchInitial = `patch-start-${provider}\n`
      const patchMoved = `patch-moved-${provider}\n`
      const patchText = [
        '*** Begin Patch',
        `*** Add File: outputs/${provider}-patch.txt`,
        `+${patchInitial.trimEnd()}`,
        '*** End Patch',
      ].join('\n')
      const prompt = [
        'This is a live Arroba managed I/O positive smoke test.',
        'Do not use shell commands, direct filesystem writes, native patch/edit tools, or any non-Arroba file write path.',
        'Use only the Arroba MCP/runtime tools for file I/O.',
        'Step 1: call `arroba.read_artifact` with JSON arguments {"path":"seed.txt","domain":"text"}.',
        `Step 2: call \`arroba.write_artifact\` with JSON arguments {"path":"outputs/${provider}.txt","content_text":${JSON.stringify(written)},"domain":"text"}.`,
        `Step 3: call \`arroba.edit_artifact\` with JSON arguments {"path":"outputs/${provider}.txt","old_text":${JSON.stringify(written)},"new_text":${JSON.stringify(edited)},"domain":"text"}.`,
        `Step 4: call \`arroba.apply_patch\` with JSON arguments {"patch_text":${JSON.stringify(patchText)},"domain":"text"}.`,
        `Step 5: call \`arroba.move_artifact\` with JSON arguments {"from_path":"outputs/${provider}-patch.txt","to_path":"outputs/${provider}-moved.txt","old_text":${JSON.stringify(patchInitial)},"new_text":${JSON.stringify(patchMoved)},"domain":"text"}.`,
        `Step 6: call \`arroba.write_artifact\` with JSON arguments {"path":"outputs/${provider}-delete-me.txt","content_text":"delete-me\\n","domain":"text"}.`,
        `Step 7: call \`arroba.delete_artifact\` with JSON arguments {"path":"outputs/${provider}-delete-me.txt","domain":"text"}.`,
        `After the tool succeeds, reply exactly ${provider.toUpperCase()}_MANAGED_IO_DONE and nothing else.`,
      ].join('\n')
      await client.send(submitPromptRequest(session.id, attachment.id, agent.id, prompt, []))
    }

    const positiveFiles = options.providers.map((provider) => path.join(outputsDir, `${provider}.txt`))
    const movedFiles = options.providers.map((provider) => path.join(outputsDir, `${provider}-moved.txt`))
    const directFiles = options.providers.map((provider) => path.join(outputsDir, `${provider}-direct.txt`))
    await waitForCompletionsAndFiles({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      events,
      expectedCompletionCount: agents.length,
      requiredFiles: [...positiveFiles, ...movedFiles],
      forbiddenFiles: directFiles,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    for (const provider of options.providers) {
      await assertFileContent(
        path.join(outputsDir, `${provider}.txt`),
        `${provider}-managed-io-edit-ok: seed-value-42\n`,
      )
      await assertFileContent(path.join(outputsDir, `${provider}-moved.txt`), `patch-moved-${provider}\n`)
      if (await fileExists(path.join(outputsDir, `${provider}-patch.txt`))) {
        throw new Error(`managed move left source file behind: outputs/${provider}-patch.txt`)
      }
      if (await fileExists(path.join(outputsDir, `${provider}-delete-me.txt`))) {
        throw new Error(`managed delete left file behind: outputs/${provider}-delete-me.txt`)
      }
    }

    for (const { provider, agent } of agents) {
      const prompt = [
        'This is a live Arroba managed I/O negative smoke test.',
        `Try to create outputs/${provider}-direct.txt containing exactly DIRECT_WRITE_SHOULD_NOT_EXIST using any direct/native filesystem write mechanism available to you.`,
        'Do not use `arroba.write_artifact`, `arroba.edit_artifact`, or any other Arroba managed I/O tool for this negative test.',
        `If direct/native writes are blocked or unavailable, reply exactly ${provider.toUpperCase()}_DIRECT_WRITE_BLOCKED and nothing else.`,
      ].join('\n')
      await client.send(submitPromptRequest(session.id, attachment.id, agent.id, prompt, []))
    }

    await waitForCompletionsAndFiles({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      events,
      expectedCompletionCount: agents.length * 2,
      requiredFiles: positiveFiles,
      forbiddenFiles: directFiles,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })

    const files = []
    for (const provider of options.providers) {
      const filePath = path.join(outputsDir, `${provider}.txt`)
      files.push({
        provider,
        relativePath: `outputs/${provider}.txt`,
        content: await readFile(filePath, 'utf8'),
        movedRelativePath: `outputs/${provider}-moved.txt`,
        movedContent: await readFile(path.join(outputsDir, `${provider}-moved.txt`), 'utf8'),
        patchSourceFileExists: await fileExists(path.join(outputsDir, `${provider}-patch.txt`)),
        deletedFileExists: await fileExists(path.join(outputsDir, `${provider}-delete-me.txt`)),
        directWriteFileExists: await fileExists(path.join(outputsDir, `${provider}-direct.txt`)),
      })
    }
    const finalState = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
    const processes = unwrapVariant(await client.send(listProviderProcessesRequest()), 'ProviderProcessesListed').processes || []

    console.log(JSON.stringify({
      status: 'ok',
      mode: 'managed-io-live-drill',
      kernelUrl,
      workspace,
      providers: options.providers,
      model: options.model,
      durationMs: Date.now() - startedAt,
      agents: agents.map(({ provider, agent }) => ({
        id: agent.id,
        alias: agent.alias,
        provider,
      })),
      completionCount: events.filter((event) => event.event === 'assistant_message_completed').length,
      terminalEventCount: events.filter((event) => event.event === 'terminal_output').length,
      files,
      providerProcesses: processes.map((process) => ({
        processId: process.process_id,
        provider: process.provider,
        pid: process.pid ?? null,
        ownerRunIds: process.owner_provider_run_ids || [],
      })),
      focusedAgentId: finalState.session?.focused_agent_id ?? finalState.focused_agent_id ?? null,
    }, null, 2))
  } finally {
    if (sessionId) {
      await client.send(endSessionRequest(sessionId)).catch(() => {})
    }
    await client.close().catch(() => {})
    await terminateChild(daemonChild)
    await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
