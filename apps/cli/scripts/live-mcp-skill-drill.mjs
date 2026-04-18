import { spawn } from 'node:child_process'
import { access, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const DEFAULT_KERNEL = 'ws://127.0.0.1:43284'
const DEFAULT_MODEL = 'gpt-5.2'
const DEFAULT_PROVIDERS = ['opencode', 'codex']
const DEFAULT_TIMEOUT_MS = 360_000
const DEFAULT_POLL_MS = 1_000
const WEB_SKILL_REPO = 'https://github.com/vercel-labs/agent-skills.git'

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

function parseArgs(argv) {
  const options = {
    kernel: DEFAULT_KERNEL,
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
    providerModels: {},
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    spawnDaemon: true,
    historyDir: null,
    keepArtifactsOnFailure: false,
    skipLiveProvider: false,
    liveMcpUse: false,
    requireWebSkill: false,
    includeGithubMcp: false,
    webSkillRepo: WEB_SKILL_REPO,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--kernel') options.kernel = argv[++i]
    else if (arg === '--provider') options.providers = [argv[++i]]
    else if (arg === '--providers') options.providers = argv[++i].split(',').map((value) => value.trim()).filter(Boolean)
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--provider-model') {
      const [provider, model] = argv[++i].split('=', 2)
      if (!provider || !model) throw new Error('--provider-model must use provider=model')
      options.providerModels[provider] = model
    }
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--no-spawn-daemon') options.spawnDaemon = false
    else if (arg === '--history-dir') options.historyDir = argv[++i]
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--skip-live-provider') options.skipLiveProvider = true
    else if (arg === '--live-mcp-use') options.liveMcpUse = true
    else if (arg === '--skip-live-mcp-use') options.liveMcpUse = false
    else if (arg === '--require-web-skill') options.requireWebSkill = true
    else if (arg === '--include-github-mcp') options.includeGithubMcp = true
    else if (arg === '--web-skill-repo') options.webSkillRepo = argv[++i]
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-mcp-skill-drill.mjs [options]',
    '',
    'Runs a local M7 MCP/skill drill with isolated daemon/session/workspace lifecycle:',
    '- installs real MCPs into the Arroba registry, including Playwright by default',
    '- optionally installs GitHub MCP when --include-github-mcp is set and a GitHub token env var exists',
    '- installs a public web skill repo into an isolated Arroba skill root when reachable',
    '- verifies per-agent MCP/skill grants and same-turn skill request bodies',
    '- optionally prompts local Codex/OpenCode agents to use granted skills and Playwright MCP',
    '',
    'Options:',
    `  --kernel ${DEFAULT_KERNEL}`,
    '  --provider PROVIDER',
    `  --providers ${DEFAULT_PROVIDERS.join(',')}`,
    `  --model ${DEFAULT_MODEL}`,
    '  --provider-model PROVIDER=MODEL (for example opencode=openai/gpt-5.2)',
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    `  --poll-ms ${DEFAULT_POLL_MS}`,
    '  --no-spawn-daemon',
    '  --history-dir PATH (required when using --no-spawn-daemon and live provider checks)',
    '  --keep-artifacts-on-failure',
    '  --skip-live-provider (registry/runtime checks only)',
    '  --live-mcp-use (also require a live provider-native Playwright tool call after relaunching the provider run with granted MCPs)',
    '  --require-web-skill (fail if the public skill repo cannot be cloned/imported)',
    '  --include-github-mcp (install GitHub MCP when GITHUB_PERSONAL_ACCESS_TOKEN or GITHUB_TOKEN is set)',
    `  --web-skill-repo ${WEB_SKILL_REPO}`,
  ].join('\n'))
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

function modelForProvider(provider, options) {
  const explicit = options.providerModels[provider]
  if (explicit) return explicit
  if (provider === 'opencode' && !options.model.includes('/')) return `openai/${options.model}`
  return options.model
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

async function runCommand(command, args, options = {}) {
  return await new Promise((resolve) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => { stdout += String(chunk) })
    child.stderr.on('data', (chunk) => { stderr += String(chunk) })
    child.on('error', (error) => resolve({ code: -1, stdout, stderr: `${stderr}${error.message}` }))
    child.on('exit', (code) => resolve({ code: code ?? -1, stdout, stderr }))
  })
}

async function createDeterministicSkill(rootDir) {
  const skillDir = path.join(rootDir, 'local-skill-source', 'm7-drill-marker')
  await mkdir(skillDir, { recursive: true })
  await writeFile(path.join(skillDir, 'SKILL.md'), [
    '---',
    'name: m7-drill-marker',
    'description: Use when asked to prove Arroba same-turn skill loading or M7 skill request behavior. Writes deterministic marker files through Arroba managed I/O.',
    '---',
    '',
    '# M7 Drill Marker',
    '',
    'When this skill is active, use Arroba managed I/O tools only.',
    '',
    'If asked for the same-turn marker, write `outputs/same-turn-skill.txt` with exactly:',
    '',
    '```text',
    'M7_SAME_TURN_SKILL_OK',
    '```',
    '',
    'If asked for the pre-granted marker, write `outputs/pregranted-skill.txt` with exactly:',
    '',
    '```text',
    'M7_PREGRANTED_SKILL_OK',
    '```',
    '',
  ].join('\n'), 'utf8')
  return skillDir
}

async function clonePublicSkillRepo(rootDir, repoUrl, requireWebSkill) {
  const cloneDir = path.join(rootDir, 'web-skill-repo')
  const result = await runCommand('git', ['clone', '--depth', '1', repoUrl, cloneDir])
  if (result.code !== 0) {
    if (requireWebSkill) {
      throw new Error(`failed to clone public skill repo ${repoUrl}: ${result.stderr || result.stdout}`)
    }
    return { sourceDirs: [], skippedReason: result.stderr || result.stdout || `git clone exited ${result.code}` }
  }
  const skillDirs = await findSkillDirs(cloneDir)
  if (skillDirs.length === 0) {
    if (requireWebSkill) throw new Error(`public skill repo ${repoUrl} did not contain any SKILL.md files`)
    return { sourceDirs: [], skippedReason: 'no SKILL.md files found' }
  }
  skillDirs.sort((a, b) => a.localeCompare(b))
  return { sourceDirs: skillDirs, skippedReason: null }
}

async function findSkillDirs(root) {
  const found = []
  async function walk(dir, depth) {
    if (depth > 5) return
    if (await fileExists(path.join(dir, 'SKILL.md'))) {
      found.push(dir)
      return
    }
    const entries = await readdir(dir, { withFileTypes: true }).catch(() => [])
    for (const entry of entries) {
      if (!entry.isDirectory()) continue
      if (entry.name === '.git' || entry.name === 'node_modules') continue
      await walk(path.join(dir, entry.name), depth + 1)
    }
  }
  await walk(root, 0)
  return found
}

function playwrightMcpConfig() {
  return {
    name: 'playwright',
    transport: {
      type: 'stdio',
      command: 'npx',
      args: ['-y', '@playwright/mcp@latest'],
    },
    enabled: true,
    required: false,
    startup_timeout_sec: 45,
    tool_timeout_sec: 45,
  }
}

function githubMcpConfig() {
  return {
    name: 'github',
    transport: {
      type: 'stdio',
      command: 'npx',
      args: ['-y', '@modelcontextprotocol/server-github'],
      env_vars: ['GITHUB_PERSONAL_ACCESS_TOKEN', 'GITHUB_TOKEN'],
    },
    enabled: true,
    required: false,
    startup_timeout_sec: 45,
    tool_timeout_sec: 45,
  }
}

async function waitForFiles({ files, timeoutMs, pollMs }) {
  const started = Date.now()
  let missing = files
  while (Date.now() - started < timeoutMs) {
    missing = []
    for (const file of files) {
      if (!(await fileExists(file))) missing.push(file)
    }
    if (missing.length === 0) return
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for files: ${missing.join(', ')}`)
}

async function waitForCompletionCount({ client, sessionId, attachmentId, events, expectedCompletionCount, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    const completed = events.filter((event) => event.event === 'assistant_message_completed')
    if (completed.length >= expectedCompletionCount) return completed
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${expectedCompletionCount} assistant completions`)
}

async function waitForHistoryToolCall({ historyDir, predicate, timeoutMs, pollMs }) {
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

async function waitForActiveProviderRun({ client, sessionId, getSessionStateRequest, getProviderRunRequest, providerRunId, timeoutMs, pollMs }) {
  const started = Date.now()
  let lastActiveRunId = null
  while (Date.now() - started < timeoutMs) {
    if (getProviderRunRequest) {
      const run = unwrapVariant(await client.send(getProviderRunRequest(providerRunId)).catch(() => null), 'ProviderRun')?.provider_run
      const runState = String(run?.state ?? '').toLowerCase()
      if (runState === 'ended' || runState === 'error' || runState === 'failed') {
        throw new Error(`provider run ${providerRunId} ended before it became active`)
      }
    }
    const state = unwrapVariant(await client.send(getSessionStateRequest(sessionId)), 'SessionStateLoaded', 'SessionState')
    const session = state.session ?? state
    lastActiveRunId = session.active_provider_run_id ?? null
    if (lastActiveRunId === providerRunId) return session
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for active provider run ${providerRunId}; last active run was ${lastActiveRunId ?? 'none'}`)
}

async function launchProviderRunAndWait({
  client,
  sessionId,
  agentId,
  provider,
  model,
  effort,
  launchProviderRunRequest,
  getSessionStateRequest,
  getProviderRunRequest,
  timeoutMs,
  pollMs,
}) {
  let lastError = null
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    const run = unwrapVariant(
      await client.send(launchProviderRunRequest(sessionId, provider, 'default', model, effort, agentId)),
      'ProviderRunLaunchAccepted',
      'ProviderRunLaunched',
    ).provider_run
    try {
      await waitForActiveProviderRun({
        client,
        sessionId,
        getSessionStateRequest,
        getProviderRunRequest,
        providerRunId: run.id,
        timeoutMs: Math.min(timeoutMs, 45_000),
        pollMs,
      })
      return run
    } catch (error) {
      lastError = error
      if (attempt < 3) await sleep(3_000 * attempt)
    }
  }
  throw lastError ?? new Error(`failed to launch ${provider} provider run`)
}

async function waitForNoProviderProcesses({ client, provider, listProviderProcessesRequest, timeoutMs, pollMs }) {
  const started = Date.now()
  let remaining = []
  while (Date.now() - started < timeoutMs) {
    const listed = unwrapVariant(
      await client.send(listProviderProcessesRequest(provider)),
      'ProviderProcessesListed',
    ).processes ?? []
    remaining = listed.filter((process) => process.provider === provider)
    if (remaining.length === 0) return
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${provider} provider processes to exit; remaining=${remaining.map((process) => process.process_id).join(',')}`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  if (options.providers.length === 0) throw new Error('at least one provider is required')
  if (!options.spawnDaemon && !options.skipLiveProvider && !options.historyDir) {
    throw new Error('--history-dir is required with --no-spawn-daemon unless --skip-live-provider is set')
  }

  const runtimeDir = path.join(cliRoot, '.tmp-live-mcp-skill-drill')
  const rootDir = path.join(cliRoot, 'target', 'live-mcp-skill-drill', `${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const outputsDir = path.join(workspace, 'outputs')
  const historyDir = options.historyDir ?? path.join(rootDir, 'history')
  await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(runtimeDir, { recursive: true })
  await mkdir(outputsDir, { recursive: true })
  await writeFile(path.join(workspace, 'README.md'), '# M7 MCP/skill live drill workspace\n', 'utf8')

  const { LocalIpcClient, requests } = await loadCliModules(runtimeDir)
  const {
    attachToSessionRequest,
    createSessionRequest,
    endSessionRequest,
    getProviderRunRequest,
    getSessionStateRequest,
    grantAgentCapabilityRequest,
    installMcpServerRequest,
    installSkillRequest,
    launchProviderRunRequest,
    listMcpServersRequest,
    listProviderProcessesRequest,
    listSkillsRequest,
    spawnAgentRequest,
    submitPromptRequest,
    teardownProviderProcessesRequest,
  } = requests

  let daemonChild = null
  let kernelUrl = options.kernel
  const startedAt = Date.now()
  let sessionId = null
  let succeeded = false
  let webSkill = { installed: false, name: null, source: null, skippedReason: null }
  try {
    if (options.spawnDaemon) {
      const ports = makePorts()
      kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
      const daemonBinary = await resolveBinary(
        path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel'),
        path.join(repoRoot, 'apps/kernel/Cargo.toml'),
        'arroba-kernel',
      )
      daemonChild = spawn(daemonBinary, [], {
        cwd: repoRoot,
        env: {
          ...process.env,
          ARROBA_KERNEL_PORT: String(ports.kernelPort),
          ARROBA_MCP_PORT: String(ports.mcpPort),
          ARROBA_OPENCODE_PORT: String(ports.opencodePort),
          ARROBA_CODEX_PORT: String(ports.codexPort),
          ARROBA_DAEMON_ID: `mcp-skill-drill-${process.pid}-${Date.now()}`,
          ARROBA_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
          ARROBA_SESSION_HISTORY_DIR: historyDir,
        },
        stdio: ['ignore', 'ignore', 'inherit'],
      })
      await waitForLocalDaemon(LocalIpcClient, kernelUrl, createSessionRequest, endSessionRequest, workspace, workspace)
    }

    const client = new LocalIpcClient(kernelUrl)
    const events = []
    try {
      const session = unwrap(await client.send(createSessionRequest(workspace, workspace, 'm7-mcp-skill-drill')), 'SessionCreated').session
      sessionId = session.id
      const attachment = unwrap(
        await client.send(attachToSessionRequest(session.id, `mcp-skill-drill-${Date.now()}`)),
        'SessionAttached',
      ).attachment
      client.onKernelEvent((event) => events.push({ ...event, observed_at_ms: Date.now() }))
      await client.subscribeToKernelEvents(session.id, attachment.id)

      const markerSkillSource = await createDeterministicSkill(rootDir)
      const markerInstall = unwrapVariant(
        await client.send(installSkillRequest(workspace, markerSkillSource)),
        'SkillInstalled',
      )
      const markerSkillName = markerInstall.skill.name

      const cloned = await clonePublicSkillRepo(rootDir, options.webSkillRepo, options.requireWebSkill)
      if (cloned.sourceDirs.length > 0) {
        const installErrors = []
        for (const sourceDir of cloned.sourceDirs) {
          try {
            const installed = unwrapVariant(
              await client.send(installSkillRequest(workspace, sourceDir)),
              'SkillInstalled',
            )
            webSkill = { installed: true, name: installed.skill.name, source: sourceDir, skippedReason: null }
            break
          } catch (error) {
            installErrors.push(`${sourceDir}: ${error.message ?? error}`)
          }
        }
        if (!webSkill.installed) {
          if (options.requireWebSkill) {
            throw new Error(`no valid public skills could be installed from ${options.webSkillRepo}: ${installErrors.join('; ')}`)
          }
          webSkill = { installed: false, name: null, source: null, skippedReason: installErrors.join('; ') }
        }
      } else {
        webSkill = { installed: false, name: null, source: null, skippedReason: cloned.skippedReason }
      }

      const installedMcps = []
      installedMcps.push(unwrapVariant(
        await client.send(installMcpServerRequest(workspace, playwrightMcpConfig())),
        'McpServerInstalled',
      ).mcp)
      if (options.includeGithubMcp) {
        if (process.env.GITHUB_PERSONAL_ACCESS_TOKEN || process.env.GITHUB_TOKEN) {
          installedMcps.push(unwrapVariant(
            await client.send(installMcpServerRequest(workspace, githubMcpConfig())),
            'McpServerInstalled',
          ).mcp)
        } else {
          console.warn('Skipping GitHub MCP install: set GITHUB_PERSONAL_ACCESS_TOKEN or GITHUB_TOKEN and pass --include-github-mcp to enable it.')
        }
      }

      const listedMcps = unwrapVariant(await client.send(listMcpServersRequest(workspace)), 'McpServersListed').mcps ?? []
      const listedSkills = unwrapVariant(await client.send(listSkillsRequest(workspace)), 'SkillsListed').skills ?? []
      if (!listedMcps.some((mcp) => mcp.name === 'playwright')) throw new Error('Playwright MCP was not listed after install')
      if (!listedSkills.some((skill) => skill.name === markerSkillName)) throw new Error('marker skill was not listed after install')
      if (webSkill.installed && !listedSkills.some((skill) => skill.name === webSkill.name)) {
        throw new Error(`web skill ${webSkill.name} was not listed after install`)
      }

      const agents = []
      if (!options.skipLiveProvider) {
        for (let index = 0; index < options.providers.length; index += 1) {
          const provider = options.providers[index]
          const agent = unwrapVariant(
            await client.send(spawnAgentRequest(
              session.id,
              provider,
              `${provider}-m7-capability-${index + 1}`,
              modelForProvider(provider, options),
              workspace,
              'low',
            )),
            'AgentSpawned',
          ).agent
          agents.push({ provider, agent })
        }
        if (agents.length === 0) throw new Error('no live provider agents were spawned')

        const grantedAgent = agents[0]
        await client.send(grantAgentCapabilityRequest(workspace, grantedAgent.agent.id, 'mcp', 'playwright'))
        await client.send(grantAgentCapabilityRequest(workspace, grantedAgent.agent.id, 'skill', markerSkillName))

        const stateAfterGrant = unwrapVariant(await client.send(getSessionStateRequest(session.id)), 'SessionStateLoaded', 'SessionState')
        const grantedState = (stateAfterGrant.session ?? stateAfterGrant).agents.find((agent) => agent.id === grantedAgent.agent.id)
        if (!grantedState?.mcp_grants?.includes?.('playwright')) throw new Error('granted agent is missing playwright MCP grant')
        if (!grantedState?.skill_grants?.includes?.(markerSkillName)) throw new Error('granted agent is missing marker skill grant')

        const pregrantBefore = events.filter((event) => event.event === 'assistant_message_completed').length
        await client.send(submitPromptRequest(session.id, attachment.id, grantedAgent.agent.id, [
          `Use the ${markerSkillName} skill.`,
          'Follow the pre-granted marker instruction from that skill.',
          'Use only Arroba managed I/O to write the required marker file.',
          'When done, reply exactly M7_PREGRANTED_SKILL_DONE.',
        ].join('\n'), []))
        await waitForCompletionCount({
          client,
          sessionId: session.id,
          attachmentId: attachment.id,
          events,
          expectedCompletionCount: pregrantBefore + 1,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
        })
        await waitForFiles({
          files: [path.join(outputsDir, 'pregranted-skill.txt')],
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
        })
        const pregrantedContent = (await readFile(path.join(outputsDir, 'pregranted-skill.txt'), 'utf8')).trim()
        if (pregrantedContent !== 'M7_PREGRANTED_SKILL_OK') {
          throw new Error(`unexpected pregranted skill file content: ${JSON.stringify(pregrantedContent)}`)
        }

        const requestAgent = agents[1] ?? agents[0]
        if (requestAgent.agent.id === grantedAgent.agent.id) {
          await client.send(grantAgentCapabilityRequest(workspace, requestAgent.agent.id, 'skill', markerSkillName))
        }
        const sameTurnBefore = events.filter((event) => event.event === 'assistant_message_completed').length
        await client.send(submitPromptRequest(session.id, attachment.id, requestAgent.agent.id, [
          'This is an Arroba runtime capability request drill.',
          `First call \`list_capabilities\` with {"kind":"skill"} and find ${JSON.stringify(markerSkillName)}.`,
          `Then call \`request_capability\` with {"kind":"skill","name":${JSON.stringify(markerSkillName)}}.`,
          'Read the returned skill.body and follow its same-turn marker instruction immediately.',
          'Use only Arroba managed I/O to write the required marker file.',
          'When done, reply exactly M7_SAME_TURN_SKILL_DONE.',
        ].join('\n'), []))
        await waitForCompletionCount({
          client,
          sessionId: session.id,
          attachmentId: attachment.id,
          events,
          expectedCompletionCount: sameTurnBefore + 1,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
        })
        await waitForFiles({
          files: [path.join(outputsDir, 'same-turn-skill.txt')],
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
        })
        const sameTurnContent = (await readFile(path.join(outputsDir, 'same-turn-skill.txt'), 'utf8')).trim()
        if (sameTurnContent !== 'M7_SAME_TURN_SKILL_OK') {
          throw new Error(`unexpected same-turn skill file content: ${JSON.stringify(sameTurnContent)}`)
        }
        await waitForHistoryToolCall({
          historyDir,
          timeoutMs: options.timeoutMs,
          pollMs: options.pollMs,
          predicate: (update) => String(update.tool ?? '').endsWith('request_capability') &&
            update.status === 'completed' &&
            update.input?.kind === 'skill' &&
            update.input?.name === markerSkillName,
        })

        if (options.liveMcpUse) {
          const mcpDrillAgent = unwrapVariant(
            await client.send(spawnAgentRequest(
              session.id,
              grantedAgent.provider,
              `${grantedAgent.provider}-m7-mcp-live`,
              modelForProvider(grantedAgent.provider, options),
              workspace,
              'low',
            )),
            'AgentSpawned',
          ).agent
          agents.push({ provider: grantedAgent.provider, agent: mcpDrillAgent })
          await client.send(grantAgentCapabilityRequest(workspace, mcpDrillAgent.id, 'mcp', 'playwright'))

          await client.send(teardownProviderProcessesRequest(grantedAgent.provider, true))
          await waitForNoProviderProcesses({
            client,
            provider: grantedAgent.provider,
            listProviderProcessesRequest,
            timeoutMs: options.timeoutMs,
            pollMs: options.pollMs,
          })

          await launchProviderRunAndWait({
            client,
            sessionId: session.id,
            agentId: mcpDrillAgent.id,
            provider: grantedAgent.provider,
            model: modelForProvider(grantedAgent.provider, options),
            effort: 'low',
            launchProviderRunRequest,
            getSessionStateRequest,
            getProviderRunRequest,
            timeoutMs: options.timeoutMs,
            pollMs: options.pollMs,
          })
          await sleep(1_500)

          const mcpBefore = events.filter((event) => event.event === 'assistant_message_completed').length
          await client.send(submitPromptRequest(session.id, attachment.id, mcpDrillAgent.id, [
            'This is a live MCP grant drill.',
            'Use the provider-native Playwright MCP tool that is available to this agent, not Arroba list_capabilities/request_capability.',
            'The tool is usually named `mcp__playwright__browser_navigate`, `mcp__playwright__browser_snapshot`, `browser_navigate`, or similar.',
            'Prefer a non-mutating Playwright browser snapshot/title/text tool first; navigating to https://example.com is optional and may require approval.',
            'After any Playwright/browser MCP tool call completes successfully, use Arroba managed I/O to write `outputs/playwright-mcp.txt` with exactly `M7_PLAYWRIGHT_MCP_OK`.',
            'Then reply exactly M7_PLAYWRIGHT_MCP_DONE.',
            'If Playwright MCP is unavailable, reply exactly M7_PLAYWRIGHT_MCP_UNAVAILABLE and do not write the marker file.',
          ].join('\n'), []))
          await waitForCompletionCount({
            client,
            sessionId: session.id,
            attachmentId: attachment.id,
            events,
            expectedCompletionCount: mcpBefore + 1,
            timeoutMs: options.timeoutMs,
            pollMs: options.pollMs,
          })
          await waitForHistoryToolCall({
            historyDir,
            timeoutMs: options.timeoutMs,
            pollMs: options.pollMs,
            predicate: (update) => {
              const tool = String(update.tool ?? '').toLowerCase()
              return update.status === 'completed' &&
                !tool.includes('arroba') &&
                (tool.includes('playwright') || tool.includes('browser'))
            },
          })
          await waitForFiles({
            files: [path.join(outputsDir, 'playwright-mcp.txt')],
            timeoutMs: options.timeoutMs,
            pollMs: options.pollMs,
          })
          const playwrightContent = (await readFile(path.join(outputsDir, 'playwright-mcp.txt'), 'utf8')).trim()
          if (playwrightContent !== 'M7_PLAYWRIGHT_MCP_OK') {
            throw new Error(`unexpected Playwright MCP marker content: ${JSON.stringify(playwrightContent)}`)
          }
        }
      }

      const processes = unwrapVariant(await client.send(listProviderProcessesRequest()), 'ProviderProcessesListed').processes || []
      console.log(JSON.stringify({
        status: 'ok',
        mode: 'mcp-skill-live-drill',
        kernelUrl,
        workspace,
        providers: options.providers,
        model: options.model,
        durationMs: Date.now() - startedAt,
        installedMcps: installedMcps.map((mcp) => mcp.name),
        installedSkills: listedSkills.map((skill) => skill.name),
        webSkill,
        liveProviderChecks: !options.skipLiveProvider,
        liveMcpUse: !options.skipLiveProvider && options.liveMcpUse,
        agents: agents.map(({ provider, agent }) => ({ provider, id: agent.id, alias: agent.alias })),
        completionCount: events.filter((event) => event.event === 'assistant_message_completed').length,
        providerProcesses: processes.map((process) => ({
          processId: process.process_id,
          provider: process.provider,
          pid: process.pid ?? null,
          ownerRunIds: process.owner_provider_run_ids || [],
        })),
      }, null, 2))
      succeeded = true
      await client.send(endSessionRequest(session.id)).catch(() => {})
      await client.close()
    } catch (error) {
      await client.close().catch(() => {})
      throw error
    }
  } finally {
    if (sessionId && !succeeded) {
      // Best-effort: client may already be closed or daemon may be gone.
    }
    await terminateChild(daemonChild)
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
