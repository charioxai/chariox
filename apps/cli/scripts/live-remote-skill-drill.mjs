import { spawn } from 'node:child_process'
import { access, mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')
const repoRoot = path.resolve(cliRoot, '..', '..')

const DEFAULT_PROVIDER = 'opencode'
const DEFAULT_MODEL = 'openai/gpt-5.2'
const DEFAULT_TIMEOUT_MS = 300_000
const DEFAULT_POLL_MS = 1_000

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
    provider: DEFAULT_PROVIDER,
    model: DEFAULT_MODEL,
    effort: 'low',
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    skipLiveProvider: false,
    keepArtifactsOnFailure: false,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === '--provider') options.provider = argv[++i]
    else if (arg === '--model') options.model = argv[++i]
    else if (arg === '--effort') options.effort = argv[++i]
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++i])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++i])
    else if (arg === '--skip-live-provider') options.skipLiveProvider = true
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help') options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    'Usage: node apps/cli/scripts/live-remote-skill-drill.mjs [options]',
    '',
    'Runs a remote M7 skill drill with isolated relay/home/worker daemons:',
    '- creates a home session and worker leased agent',
    '- installs an Arroba-owned skill with assets',
    '- verifies local-only grants do not materialize on the worker',
    '- verifies local-to-remote move synchronizes existing skill grants',
    '- verifies remote-first grant-time materialization on the worker',
    '- verifies prompt-time repair after worker materialization is removed',
    '- optionally verifies same-turn remote request_extension use',
    '',
    `  --provider ${DEFAULT_PROVIDER}`,
    `  --model ${DEFAULT_MODEL}`,
    '  --effort low',
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    `  --poll-ms ${DEFAULT_POLL_MS}`,
    '  --skip-live-provider',
    '  --keep-artifacts-on-failure',
  ].join('\n'))
}

function makePorts() {
  const base = 57000 + Math.floor(Math.random() * 1000)
  return {
    relayPort: base,
    homeKernelPort: base + 1000,
    workerKernelPort: base + 1001,
    homeMcpPort: base + 2000,
    workerMcpPort: base + 2001,
    homeOpenCodePort: base + 3000,
    workerOpenCodePort: base + 3001,
    homeCodexPort: base + 3002,
    workerCodexPort: base + 3003,
  }
}

function daemonEnv({
  ports,
  rootDir,
  relayToken,
  daemonId,
  daemonAlias,
  machineId,
  machineAlias,
  acceptRemoteLeases,
  kernelPort,
  mcpPort,
  opencodePort,
  codexPort,
  socketName,
  historyDir,
}) {
  return {
    ...process.env,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(mcpPort),
    ARROBA_OPENCODE_PORT: String(opencodePort),
    ARROBA_CODEX_PORT: String(codexPort),
    ARROBA_RELAY_URL: `ws://127.0.0.1:${ports.relayPort}`,
    ARROBA_RELAY_TOKEN: relayToken,
    ARROBA_DAEMON_ID: daemonId,
    ARROBA_DAEMON_ALIAS: daemonAlias,
    ARROBA_MACHINE_ID: machineId,
    ARROBA_MACHINE_ALIAS: machineAlias,
    ARROBA_ACCEPT_REMOTE_LEASES: acceptRemoteLeases ? '1' : '0',
    ARROBA_DAEMON_SOCKET: path.join(rootDir, socketName),
    ARROBA_SESSION_HISTORY_DIR: historyDir,
  }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (resp, key) => resp?.[key] ?? resp
const unwrapVariant = (resp, ...keys) => keys.map((key) => resp?.[key]).find((value) => value != null) ?? resp

async function waitForTcpPort(port, host = '127.0.0.1', timeoutMs = 15_000) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const connected = await new Promise((resolve) => {
      const socket = net.connect({ host, port })
      socket.once('connect', () => {
        socket.destroy()
        resolve(true)
      })
      socket.once('error', () => {
        socket.destroy()
        resolve(false)
      })
    })
    if (connected) return
    await sleep(100)
  }
  throw new Error(`TCP listener ${host}:${port} did not become reachable`)
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
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const probe = new LocalIpcClient(kernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
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
  throw new Error('home daemon did not become ready')
}

async function waitForRelayTarget(LocalIpcClient, listRemoteMachinesRequest, relayUrl, relayToken, targetDaemonAlias) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(relayUrl, {
      relayAuthToken: relayToken,
      targetDaemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      await Promise.race([
        client.send(listRemoteMachinesRequest()),
        sleep(2_000).then(() => { throw new Error('probe timeout') }),
      ])
      await client.close().catch(() => {})
      return
    } catch {
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`relay target ${targetDaemonAlias} did not become reachable`)
}

async function waitForRemoteMachine(client, listRemoteMachinesRequest, machineRef) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const machines = unwrapVariant(await client.send(listRemoteMachinesRequest()), 'RemoteMachinesListed').machines || []
    if (machines.some((machine) => machine.machine_id === machineRef || machine.machine_alias === machineRef || machine.display_name === machineRef)) return
    await sleep(500)
  }
  throw new Error(`remote machine ${machineRef} did not become visible`)
}

function spawnRemoteAgentRequest(sessionId, provider, alias, model, worktreeId, effort, kernelRef) {
  return {
    SpawnAgent: {
      session_id: sessionId,
      provider,
      alias,
      model,
      effort,
      worktree_id: worktreeId,
      kernel_ref: kernelRef,
    },
  }
}

async function findMaterializedSkill(workspace, homeKernelId, skillName, timeoutMs, pollMs) {
  const root = path.join(workspace, '.arroba', 'remote', 'skills', homeKernelId, skillName)
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    try {
      const versions = await readdir(root)
      for (const version of versions) {
        const candidate = path.join(root, version)
        const skillMd = path.join(candidate, 'SKILL.md')
        const asset = path.join(candidate, 'assets', 'checklist.txt')
        await access(skillMd)
        await access(asset)
        return { root: candidate, version, assetText: await readFile(asset, 'utf8') }
      }
    } catch {
      await sleep(pollMs)
    }
  }
  throw new Error(`timed out waiting for materialized skill ${skillName}`)
}

async function materializedSkillExists(workspace, homeKernelId, skillName) {
  const root = path.join(workspace, '.arroba', 'remote', 'skills', homeKernelId, skillName)
  try {
    const versions = await readdir(root)
    return versions.length > 0
  } catch {
    return false
  }
}

async function clearMaterializedSkill(workspace, homeKernelId, skillName) {
  await rm(path.join(workspace, '.arroba', 'remote', 'skills', homeKernelId, skillName), {
    recursive: true,
    force: true,
  })
}

async function waitForCompletion({ client, sessionId, attachmentId, events, count, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    const completed = events.filter((event) => event.event === 'assistant_message_completed')
    if (completed.length >= count) return completed
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${count} assistant completions`)
}

async function waitForHistoryText({ historyDir, requiredText, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const files = (await readdir(historyDir).catch(() => []))
      .map((file) => path.join(historyDir, file))
      .sort()
    for (const file of files) {
      const lines = (await readFile(file, 'utf8').catch(() => '')).split(/\r?\n/).filter(Boolean)
      for (const line of lines) {
        try {
          const entry = JSON.parse(line)
          if (entry.kind === 'provider_output' && typeof entry.text === 'string' && entry.text.includes(requiredText)) return entry
        } catch {}
      }
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for history text ${requiredText}`)
}

async function waitForFileText({ filePath, requiredText, timeoutMs, pollMs }) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    const text = await readFile(filePath, 'utf8').catch(() => '')
    if (text.includes(requiredText)) return text
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for ${requiredText} in ${filePath}`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const ports = makePorts()
  const rootDir = path.join(os.tmpdir(), `arroba-remote-skill-${process.pid}-${Date.now()}`)
  const workspace = path.join(rootDir, 'workspace')
  const cliRuntimeDir = path.join(cliRoot, '.tmp-live-remote-skill-drill')
  await mkdir(workspace, { recursive: true })
  await rm(cliRuntimeDir, { recursive: true, force: true })
  await mkdir(cliRuntimeDir, { recursive: true })
  const { LocalIpcClient, requests } = await loadCliModules(cliRuntimeDir)
  const {
    attachToSessionRequest,
    createSessionRequest,
    endSessionRequest,
    grantAgentExtensionRequest,
    installSkillRequest,
    listRemoteMachinesRequest,
    moveAgentToRemoteRequest,
    spawnAgentRequest,
    submitPromptRequest,
  } = requests

  const relayToken = `remote-skill-token-${process.pid}`
  const relayBinary = await resolveBinary(
    path.join(repoRoot, 'apps/relay/target/debug/arroba-relay'),
    path.join(repoRoot, 'apps/relay/Cargo.toml'),
    'arroba-relay',
  )
  const daemonBinary = await resolveBinary(
    path.join(repoRoot, 'apps/kernel/target/debug/arroba-kernel'),
    path.join(repoRoot, 'apps/kernel/Cargo.toml'),
    'arroba-kernel',
  )
  const relayEnv = {
    ...process.env,
    ARROBA_RELAY_HOST: '127.0.0.1',
    ARROBA_RELAY_PORT: String(ports.relayPort),
    ARROBA_RELAY_TOKEN: relayToken,
  }
  const relayUrl = `ws://127.0.0.1:${ports.relayPort}`
  const homeKernelUrl = `ws://127.0.0.1:${ports.homeKernelPort}`
  const workerMachineId = `remote-skill-machine-worker-${process.pid}`
  const workerMachineAlias = `remote-skill-worker-${process.pid}`
  const homeDaemonId = `remote-skill-home-${process.pid}-${Date.now()}`
  const workerDaemonId = `remote-skill-worker-${process.pid}-${Date.now()}`
  const homeHistoryDir = path.join(rootDir, `${homeDaemonId}-history`)
  const workerHistoryDir = path.join(rootDir, `${workerDaemonId}-history`)

  let relayChild = null
  let homeChild = null
  let workerChild = null
  let client = null
  let succeeded = false
  try {
    relayChild = spawn(relayBinary, [], { cwd: repoRoot, env: relayEnv, stdio: ['ignore', 'ignore', 'inherit'] })
    await waitForTcpPort(ports.relayPort)
    homeChild = spawn(daemonBinary, [], {
      cwd: repoRoot,
      env: daemonEnv({
        ports,
        rootDir,
        relayToken,
        daemonId: homeDaemonId,
        daemonAlias: 'home',
        machineId: `remote-skill-machine-home-${process.pid}`,
        machineAlias: `remote-skill-home-machine-${process.pid}`,
        acceptRemoteLeases: false,
        kernelPort: ports.homeKernelPort,
        mcpPort: ports.homeMcpPort,
        opencodePort: ports.homeOpenCodePort,
        codexPort: ports.homeCodexPort,
        socketName: 'home.sock',
        historyDir: homeHistoryDir,
      }),
      stdio: ['ignore', 'ignore', 'inherit'],
    })
    workerChild = spawn(daemonBinary, [], {
      cwd: repoRoot,
      env: daemonEnv({
        ports,
        rootDir,
        relayToken,
        daemonId: workerDaemonId,
        daemonAlias: 'worker',
        machineId: workerMachineId,
        machineAlias: workerMachineAlias,
        acceptRemoteLeases: true,
        kernelPort: ports.workerKernelPort,
        mcpPort: ports.workerMcpPort,
        opencodePort: ports.workerOpenCodePort,
        codexPort: ports.workerCodexPort,
        socketName: 'worker.sock',
        historyDir: workerHistoryDir,
      }),
      stdio: ['ignore', 'ignore', 'inherit'],
    })

    await waitForLocalDaemon(LocalIpcClient, homeKernelUrl, createSessionRequest, endSessionRequest, workspace)
    await waitForRelayTarget(LocalIpcClient, listRemoteMachinesRequest, relayUrl, relayToken, 'home')
    await waitForRelayTarget(LocalIpcClient, listRemoteMachinesRequest, relayUrl, relayToken, 'worker')

    client = new LocalIpcClient(homeKernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    await waitForRemoteMachine(client, listRemoteMachinesRequest, workerMachineId)

    const sourceSkill = path.join(rootDir, 'remote-drill-skill')
    await mkdir(path.join(sourceSkill, 'assets'), { recursive: true })
    await writeFile(path.join(sourceSkill, 'SKILL.md'), [
      '---',
      'name: remote-drill',
      'description: Remote skill drill; read the asset and report its token.',
      'short-description: Remote skill drill',
      '---',
      'When asked to use this skill, read `assets/checklist.txt` under the provided `materialized_root` using Arroba workspace live sync if filesystem reads are not directly available.',
      'Write `outputs/remote-skill-provider.txt` through Arroba workspace live sync with the exact asset token and the exact phrase REMOTE_SKILL_DRILL_OK.',
      '',
    ].join('\n'), 'utf8')
    await writeFile(path.join(sourceSkill, 'assets', 'checklist.txt'), 'REMOTE_SKILL_ASSET_TOKEN=asset-remote-skill-ok\n', 'utf8')

    const session = unwrap(await client.send(createSessionRequest(workspace, workspace)), 'SessionCreated').session
    const attachment = unwrap(await client.send(attachToSessionRequest(session.id, `remote-skill-drill-${Date.now()}`)), 'SessionAttached').attachment
    const installed = unwrapVariant(await client.send(installSkillRequest(workspace, sourceSkill)), 'SkillInstalled')

    const scenarioResults = []
    let completionCount = 0
    const events = []
    if (!options.skipLiveProvider) {
      client.onKernelEvent((event) => events.push({ ...event, observed_at_ms: Date.now() }))
      await client.subscribeToKernelEvents(session.id, attachment.id)
    }

    const runLivePrompt = async (agentId, prompt, outputFile) => {
      if (options.skipLiveProvider) return null
      await rm(outputFile, { force: true }).catch(() => {})
      await client.send(submitPromptRequest(session.id, attachment.id, agentId, prompt, []))
      completionCount += 1
      await waitForCompletion({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        events,
        count: completionCount,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
      const text = await waitForFileText({
        filePath: outputFile,
        requiredText: 'REMOTE_SKILL_DRILL_OK',
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
      if (!text.includes('asset-remote-skill-ok')) {
        throw new Error(`provider file did not include asset token: ${text}`)
      }
      return text
    }

    const outputFile = path.join(workspace, 'outputs', 'remote-skill-provider.txt')

    const localSpawned = unwrapVariant(await client.send(spawnAgentRequest(
      session.id,
      options.provider,
      'local-then-remote-skill-agent',
      options.model,
      workspace,
      options.effort,
    )), 'AgentSpawned')
    const localAgent = localSpawned.agent
    await client.send(grantAgentExtensionRequest(workspace, localAgent.id, 'skill', 'remote-drill'))
    if (await materializedSkillExists(workspace, homeDaemonId, 'remote-drill')) {
      throw new Error('local-only skill grant unexpectedly materialized on the worker')
    }
    const moved = unwrapVariant(await client.send(moveAgentToRemoteRequest(
      session.id,
      localAgent.id,
      workerMachineId,
    )), 'AgentMovedToRemote').agent
    const movedMaterialized = await findMaterializedSkill(workspace, homeDaemonId, 'remote-drill', options.timeoutMs, options.pollMs)
    const movedOutput = await runLivePrompt(
      moved.id,
      'Use the remote-drill skill now after this local-to-remote move. Read the synchronized asset and write outputs/remote-skill-provider.txt with the asset token and REMOTE_SKILL_DRILL_OK.',
      outputFile,
    )
    scenarioResults.push({
      scenario: 'local_grant_then_move_to_remote',
      agent: { id: moved.id, ref: moved.agent_ref, remote_execution: moved.remote_execution },
      materialized: movedMaterialized,
      verifiedProviderFileText: movedOutput,
    })

    await clearMaterializedSkill(workspace, homeDaemonId, 'remote-drill')

    const spawned = unwrapVariant(await client.send(spawnRemoteAgentRequest(
      session.id,
      options.provider,
      'remote-then-grant-skill-agent',
      options.model,
      workspace,
      options.effort,
      workerDaemonId,
    )), 'AgentSpawned')
    const agent = spawned.agent
    await client.send(grantAgentExtensionRequest(workspace, agent.id, 'skill', 'remote-drill'))
    const materialized = await findMaterializedSkill(workspace, homeDaemonId, 'remote-drill', options.timeoutMs, options.pollMs)
    const verifiedProviderFileText = await runLivePrompt(
      agent.id,
      'Use the remote-drill skill now. Read the synchronized asset and write outputs/remote-skill-provider.txt with the asset token and REMOTE_SKILL_DRILL_OK.',
      outputFile,
    )
    scenarioResults.push({
      scenario: 'remote_then_grant',
      agent: { id: agent.id, ref: agent.agent_ref, remote_execution: agent.remote_execution },
      materialized,
      verifiedProviderFileText,
    })

    await clearMaterializedSkill(workspace, homeDaemonId, 'remote-drill')

    if (options.skipLiveProvider) {
      scenarioResults.push({
        scenario: 'remote_prompt_time_repair_after_deleted_materialization',
        skipped: 'requires a live provider prompt to trigger prompt-time repair',
      })
    } else {
      const repairedOutput = await runLivePrompt(
        agent.id,
        'Use the already granted remote-drill skill again. If the skill package needs to be synchronized again, wait for Arroba to expose it, then read the synchronized asset and write outputs/remote-skill-provider.txt with the asset token and REMOTE_SKILL_DRILL_OK.',
        outputFile,
      )
      const repairedMaterialized = await findMaterializedSkill(workspace, homeDaemonId, 'remote-drill', options.timeoutMs, options.pollMs)
      scenarioResults.push({
        scenario: 'remote_prompt_time_repair_after_deleted_materialization',
        agent: { id: agent.id, ref: agent.agent_ref, remote_execution: agent.remote_execution },
        materialized: repairedMaterialized,
        verifiedProviderFileText: repairedOutput,
      })
    }

    await clearMaterializedSkill(workspace, homeDaemonId, 'remote-drill')

    let requestScenario = null
    if (!options.skipLiveProvider) {
      const requestSpawned = unwrapVariant(await client.send(spawnRemoteAgentRequest(
        session.id,
        options.provider,
        'remote-request-skill-agent',
        options.model,
        workspace,
        options.effort,
        workerDaemonId,
      )), 'AgentSpawned')
      const requestAgent = requestSpawned.agent
      const requestOutput = await runLivePrompt(
        requestAgent.id,
        'You do not have the remote-drill skill yet. First call list_extensions for skills, then call request_extension with kind "skill" and name "remote-drill". Follow the returned SKILL.md body immediately. Read the synchronized asset and write outputs/remote-skill-provider.txt with the asset token and REMOTE_SKILL_DRILL_OK.',
        outputFile,
      )
      const requestMaterialized = await findMaterializedSkill(workspace, homeDaemonId, 'remote-drill', options.timeoutMs, options.pollMs)
      requestScenario = {
        scenario: 'remote_request_extension_same_turn',
        agent: { id: requestAgent.id, ref: requestAgent.agent_ref, remote_execution: requestAgent.remote_execution },
        materialized: requestMaterialized,
        verifiedProviderFileText: requestOutput,
      }
      scenarioResults.push(requestScenario)
    }

    console.log(JSON.stringify({
      status: 'ok',
      mode: 'remote-skill-live-drill',
      relayUrl,
      homeKernelUrl,
      workerMachineId,
      workerMachineAlias,
      provider: options.provider,
      model: options.model,
      effort: options.effort,
      installedSkill: installed.skill ?? installed.metadata ?? installed,
      scenarios: scenarioResults,
      liveProviderSkipped: options.skipLiveProvider,
      completionCount,
    }, null, 2))
    succeeded = true
  } finally {
    if (client) await client.close().catch(() => {})
    await terminateChild(homeChild)
    await terminateChild(workerChild)
    await terminateChild(relayChild)
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(rootDir, { recursive: true, force: true }).catch(() => {})
      await rm(cliRuntimeDir, { recursive: true, force: true }).catch(() => {})
    } else {
      console.error(`remote skill drill artifacts kept at ${rootDir}`)
      console.error(`remote skill drill transient CLI modules kept at ${cliRuntimeDir}`)
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
