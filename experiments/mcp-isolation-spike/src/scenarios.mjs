#!/usr/bin/env node
import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { McpClientProcess, McpSupervisor } from './mcp-supervisor.mjs'
import { writeProviderPlans } from './provider-launcher.mjs'
import { launchOpenCodeServer, resolveOpenCodeBinary } from './opencode-driver.mjs'
import { launchCodexServer, resolveCodexBinary } from './codex-driver.mjs'

const spikeRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const artifactsRoot = path.join(spikeRoot, 'artifacts')

function timestamp() {
  return new Date().toISOString().replace(/[:.]/g, '-')
}

async function artifactDir(name) {
  const dir = path.join(artifactsRoot, `${timestamp()}-${name}`)
  await mkdir(dir, { recursive: true })
  return dir
}

function countStarts(snapshot, matcher) {
  return Object.values(snapshot.servers).filter(matcher).reduce((sum, server) => sum + server.starts, 0)
}

function countToolCalls(snapshot, matcher) {
  return Object.values(snapshot.servers).filter(matcher).reduce((sum, server) => sum + server.tool_calls, 0)
}

async function runLifecycle() {
  const dir = await artifactDir('lifecycle')
  const supervisor = new McpSupervisor({ artifactDir: dir })
  await supervisor.prepare()
  try {
    const alpha = await supervisor.ensure({ name: 'fake-alpha', scope: 'shared' })
    await alpha.toolsList()
    await alpha.toolsCall('fake-alpha_echo', { message: 'first grant' })

    // Simulate provider restart/re-render with an expanded grant set. The supervised
    // backing for fake-alpha must stay warm while fake-beta is added.
    const beta = await supervisor.ensure({ name: 'fake-beta', scope: 'shared' })
    await alpha.toolsList()
    await beta.toolsList()

    const gamma = await supervisor.ensure({ name: 'fake-gamma', scope: 'shared' })
    await alpha.toolsCall('fake-alpha_echo', { message: 'still warm after gamma grant' })
    await gamma.toolsCall('fake-gamma_echo', { message: 'new grant' })

    const snapshot = await supervisor.snapshot()
    const alphaStarts = countStarts(snapshot, (server) => server.name === 'fake-alpha' && server.mode === 'backing-shared')
    const passed = alphaStarts === 1
    const result = {
      scenario: 'lifecycle',
      passed,
      artifact_dir: dir,
      assertions: {
        fake_alpha_backing_starts: alphaStarts,
        fake_alpha_backing_started_once: passed,
      },
      snapshot,
    }
    await writeFile(path.join(dir, 'results.json'), JSON.stringify(result, null, 2), 'utf8')
    console.log(JSON.stringify(result, null, 2))
    if (!passed) process.exitCode = 1
  } finally {
    await supervisor.stopAll()
  }
}

async function runVisibilityPlan() {
  const dir = await artifactDir('visibility-plan')
  const supervisor = new McpSupervisor({ artifactDir: dir })
  await supervisor.prepare()
  const agentA = {
    id: 'agent-a',
    mcps: [supervisor.providerStdioConfig({ name: 'fake-alpha', agentId: 'agent-a', mode: 'provider-visible' })],
  }
  const agentB = {
    id: 'agent-b',
    mcps: [],
  }
  const plans = await writeProviderPlans({ artifactDir: dir, agents: [agentA, agentB] })

  // Validate the fake MCP itself can perform the provider-visible handshake. The next slice
  // connects these generated plans to live Codex/OpenCode provider processes.
  const probe = new McpClientProcess({ name: 'fake-alpha', statePath: supervisor.statePath, mode: 'provider-visible', agentId: 'agent-a' })
  await probe.initialize()
  const tools = await probe.toolsList()
  await probe.stop()
  const snapshot = await supervisor.snapshot()
  const result = {
    scenario: 'visibility-plan',
    passed: plans.length === 4 && tools.tools?.some((tool) => tool.name === 'fake-alpha_echo'),
    artifact_dir: dir,
    provider_plans: plans.map((plan) => ({ provider: plan.provider, agent_id: plan.agent_id, mcp_names: plan.mcp_names })),
    generated_files: ['provider-plans/agent-a-codex.json', 'provider-plans/agent-a-opencode.json', 'provider-plans/agent-b-codex.json', 'provider-plans/agent-b-opencode.json'],
    snapshot,
  }
  await writeFile(path.join(dir, 'results.json'), JSON.stringify(result, null, 2), 'utf8')
  console.log(JSON.stringify(result, null, 2))
  if (!result.passed) process.exitCode = 1
}


async function runOpenCodeStatus() {
  const dir = await artifactDir('opencode-status')
  const supervisor = new McpSupervisor({ artifactDir: dir })
  await supervisor.prepare()
  const agentA = {
    id: 'agent-a',
    mcps: [supervisor.providerStdioConfig({ name: 'fake-alpha', agentId: 'agent-a', mode: 'opencode-provider-visible' })],
  }
  const agentB = { id: 'agent-b', mcps: [] }
  await writeProviderPlans({ artifactDir: dir, agents: [agentA, agentB] })

  const runs = []
  try {
    runs.push(await launchOpenCodeServer({ agentId: agentA.id, mcps: agentA.mcps, artifactDir: dir }))
    runs.push(await launchOpenCodeServer({ agentId: agentB.id, mcps: agentB.mcps, artifactDir: dir }))

    const statuses = {}
    for (const run of runs) {
      statuses[run.agentId] = await run.mcpStatus()
    }
    const snapshot = await supervisor.snapshot()
    const agentAHasAlpha = statuses['agent-a']?.['fake-alpha']?.status === 'connected'
    const agentBHasAlpha = Object.prototype.hasOwnProperty.call(statuses['agent-b'] ?? {}, 'fake-alpha')
    const providerVisibleStarts = countStarts(
      snapshot,
      (server) => server.name === 'fake-alpha' && server.mode === 'opencode-provider-visible' && server.agent_id === 'agent-a',
    )
    const passed = agentAHasAlpha && !agentBHasAlpha && providerVisibleStarts === 1
    const result = {
      scenario: 'opencode-status',
      passed,
      artifact_dir: dir,
      executable: resolveOpenCodeBinary(),
      assertions: {
        agent_a_fake_alpha_connected: agentAHasAlpha,
        agent_b_fake_alpha_absent: !agentBHasAlpha,
        fake_alpha_provider_visible_starts: providerVisibleStarts,
      },
      servers: runs.map((run) => ({ agent_id: run.agentId, base_url: run.baseUrl, log_path: run.logPath })),
      mcp_status: statuses,
      snapshot,
    }
    await writeFile(path.join(dir, 'results.json'), JSON.stringify(result, null, 2), 'utf8')
    console.log(JSON.stringify(result, null, 2))
    if (!passed) process.exitCode = 1
  } finally {
    await Promise.all(runs.map((run) => run.stop()))
    await supervisor.stopAll()
  }
}

async function runCodexThreadStart() {
  const dir = await artifactDir('codex-thread-start')
  const supervisor = new McpSupervisor({ artifactDir: dir })
  await supervisor.prepare()
  const agentA = {
    id: 'agent-a',
    mcps: [supervisor.providerStdioConfig({ name: 'fake-alpha', agentId: 'agent-a', mode: 'codex-provider-visible' })],
  }
  const agentB = { id: 'agent-b', mcps: [] }
  await writeProviderPlans({ artifactDir: dir, agents: [agentA, agentB] })

  const runs = []
  const sockets = []
  try {
    runs.push(await launchCodexServer({ agentId: agentA.id, mcps: agentA.mcps, artifactDir: dir }))
    runs.push(await launchCodexServer({ agentId: agentB.id, mcps: agentB.mcps, artifactDir: dir }))

    for (const run of runs) {
      const socket = await run.connectInitialized()
      sockets.push(socket)
      await socket.threadStart({ cwd: spikeRoot })
    }

    const snapshot = await supervisor.snapshot()
    const agentAAlphaStarts = countStarts(
      snapshot,
      (server) => server.name === 'fake-alpha' && server.mode === 'codex-provider-visible' && server.agent_id === 'agent-a',
    )
    const agentBAlphaStarts = countStarts(
      snapshot,
      (server) => server.name === 'fake-alpha' && server.mode === 'codex-provider-visible' && server.agent_id === 'agent-b',
    )
    const passed = agentAAlphaStarts === 1 && agentBAlphaStarts === 0
    const result = {
      scenario: 'codex-thread-start',
      passed,
      artifact_dir: dir,
      executable: resolveCodexBinary(),
      assertions: {
        agent_a_fake_alpha_started_once: agentAAlphaStarts === 1,
        agent_b_fake_alpha_absent: agentBAlphaStarts === 0,
        fake_alpha_agent_a_starts: agentAAlphaStarts,
        fake_alpha_agent_b_starts: agentBAlphaStarts,
      },
      servers: runs.map((run) => ({ agent_id: run.agentId, endpoint: run.endpoint, log_path: run.logPath })),
      snapshot,
    }
    await writeFile(path.join(dir, 'results.json'), JSON.stringify(result, null, 2), 'utf8')
    console.log(JSON.stringify(result, null, 2))
    if (!passed) process.exitCode = 1
  } finally {
    await Promise.all(sockets.map((socket) => socket.close()))
    await Promise.all(runs.map((run) => run.stop()))
    await supervisor.stopAll()
  }
}

async function runCodexRestartResume() {
  const dir = await artifactDir('codex-restart-resume')
  const supervisor = new McpSupervisor({ artifactDir: dir })
  await supervisor.prepare()
  const alpha = supervisor.proxiedProviderStdioConfig({ name: 'fake-alpha', agentId: 'agent-a', backingMode: 'backing-per-agent' })
  const beta = supervisor.proxiedProviderStdioConfig({ name: 'fake-beta', agentId: 'agent-a', backingMode: 'backing-per-agent' })

  const runs = []
  const sockets = []
  try {
    const firstRun = await launchCodexServer({ agentId: 'agent-a-before', mcps: [alpha], artifactDir: dir })
    runs.push(firstRun)
    const firstSocket = await firstRun.connectInitialized()
    sockets.push(firstSocket)
    const started = await firstSocket.threadStart({ cwd: spikeRoot, ephemeral: false })
    const threadId = started?.thread?.id
    if (!threadId) throw new Error(`Codex thread/start did not return a thread id: ${JSON.stringify(started)}`)
    const token = `chariox-spike-${Date.now()}`
    const firstTurn = await firstSocket.turnStart(threadId, `Remember this token for a resume test: ${token}. Reply with exactly ACK.`, { cwd: spikeRoot })
    await firstSocket.waitForTurnCompleted(firstTurn.turnId)
    await firstSocket.close()
    await firstRun.stop()

    const secondRun = await launchCodexServer({ agentId: 'agent-a-after', mcps: [alpha, beta], artifactDir: dir })
    runs.push(secondRun)
    const secondSocket = await secondRun.connectInitialized()
    sockets.push(secondSocket)
    const resumed = await secondSocket.threadResume(threadId, { cwd: spikeRoot })
    const resumedThreadId = resumed?.thread?.id
    const secondTurn = await secondSocket.turnStart(threadId, 'Continue after reload. Reply with the token I asked you to remember, then call fake-beta if it is available.', { cwd: spikeRoot })
    await secondSocket.waitForTurnCompleted(secondTurn.turnId)
    const transcriptText = [...sockets.map((socket) => socket.transcriptText())].join('\n')
    const transcriptIncludesToken = transcriptText.includes(token)

    const snapshot = await supervisor.snapshot()
    const alphaStarts = countStarts(
      snapshot,
      (server) => server.name === 'fake-alpha' && server.mode === 'backing-per-agent' && server.agent_id === 'agent-a',
    )
    const betaStarts = countStarts(
      snapshot,
      (server) => server.name === 'fake-beta' && server.mode === 'backing-per-agent' && server.agent_id === 'agent-a',
    )
    const alphaProxyStarts = countStarts(
      snapshot,
      (server) => server.name === 'fake-alpha' && server.mode === 'proxy' && server.agent_id === 'agent-a',
    )
    const betaToolCalls = Object.values(snapshot.servers)
      .filter((server) => server.name === 'fake-beta' && server.mode === 'backing-per-agent' && server.agent_id === 'agent-a')
      .reduce((sum, server) => sum + server.tool_calls, 0)
    const passed = resumedThreadId === threadId && alphaStarts === 1 && alphaProxyStarts === 2 && betaStarts === 1 && transcriptIncludesToken
    const result = {
      scenario: 'codex-restart-resume',
      passed,
      artifact_dir: dir,
      executable: resolveCodexBinary(),
      thread_id: threadId,
      resumed_thread_id: resumedThreadId,
      token,
      transcript_text: transcriptText,
      assertions: {
        thread_id_preserved: resumedThreadId === threadId,
        transcript_includes_remembered_token: transcriptIncludesToken,
        fake_alpha_backing_stayed_warm_across_provider_restart: alphaStarts === 1,
        fake_alpha_proxy_restarted_with_provider: alphaProxyStarts === 2,
        fake_beta_started_after_expanded_grant: betaStarts === 1,
        fake_beta_tool_calls_after_resume: betaToolCalls,
        completed_turn_before_resume_required: true,
      },
      note: 'This scenario uses provider-facing proxy stdio MCPs. Provider proxies restart with Codex, but backing MCP runtimes stay warm across provider restart.',
      servers: runs.map((run) => ({ agent_id: run.agentId, endpoint: run.endpoint, log_path: run.logPath })),
      snapshot,
    }
    await writeFile(path.join(dir, 'results.json'), JSON.stringify(result, null, 2), 'utf8')
    console.log(JSON.stringify(result, null, 2))
    if (!passed) process.exitCode = 1
  } finally {
    await Promise.all(sockets.map((socket) => socket.close()))
    await Promise.all(runs.map((run) => run.stop()))
    await supervisor.stopAll()
  }
}

async function runOpenCodeRestartResume() {
  const dir = await artifactDir('opencode-restart-resume')
  const supervisor = new McpSupervisor({ artifactDir: dir })
  await supervisor.prepare()
  const alpha = supervisor.proxiedProviderStdioConfig({ name: 'fake-alpha', agentId: 'agent-a', backingMode: 'backing-per-agent' })
  const beta = supervisor.proxiedProviderStdioConfig({ name: 'fake-beta', agentId: 'agent-a', backingMode: 'backing-per-agent' })
  const runs = []
  try {
    const firstRun = await launchOpenCodeServer({ agentId: 'agent-a-before', mcps: [alpha], artifactDir: dir })
    runs.push(firstRun)
    const created = await firstRun.createSession({ directory: spikeRoot })
    const sessionId = created?.id
    if (!sessionId) throw new Error(`OpenCode session create did not return id: ${JSON.stringify(created)}`)
    const token = `chariox-spike-${Date.now()}`
    await firstRun.prompt(sessionId, `Remember this token for a resume test: ${token}. Reply with exactly ACK.`, { directory: spikeRoot })
    await firstRun.stop()

    const secondRun = await launchOpenCodeServer({ agentId: 'agent-a-after', mcps: [alpha, beta], artifactDir: dir })
    runs.push(secondRun)
    const resumed = await secondRun.getSession(sessionId, { directory: spikeRoot })
    await secondRun.prompt(sessionId, 'Continue after reload. Reply with the token I asked you to remember, then call fake-beta if it is available.', { directory: spikeRoot })
    const transcriptText = await secondRun.transcriptText(sessionId, { directory: spikeRoot })
    const transcriptIncludesToken = transcriptText.includes(token)

    const snapshot = await supervisor.snapshot()
    const alphaStarts = countStarts(
      snapshot,
      (server) => server.name === 'fake-alpha' && server.mode === 'backing-per-agent' && server.agent_id === 'agent-a',
    )
    const betaStarts = countStarts(
      snapshot,
      (server) => server.name === 'fake-beta' && server.mode === 'backing-per-agent' && server.agent_id === 'agent-a',
    )
    const alphaProxyStarts = countStarts(
      snapshot,
      (server) => server.name === 'fake-alpha' && server.mode === 'proxy' && server.agent_id === 'agent-a',
    )
    const betaToolCalls = Object.values(snapshot.servers)
      .filter((server) => server.name === 'fake-beta' && server.mode === 'backing-per-agent' && server.agent_id === 'agent-a')
      .reduce((sum, server) => sum + server.tool_calls, 0)
    const passed = resumed?.id === sessionId && alphaStarts === 1 && alphaProxyStarts === 2 && betaStarts === 1 && betaToolCalls >= 1 && transcriptIncludesToken
    const result = {
      scenario: 'opencode-restart-resume',
      passed,
      artifact_dir: dir,
      executable: resolveOpenCodeBinary(),
      session_id: sessionId,
      resumed_session_id: resumed?.id ?? null,
      token,
      transcript_text: transcriptText,
      assertions: {
        session_id_preserved: resumed?.id === sessionId,
        transcript_includes_remembered_token: transcriptIncludesToken,
        fake_alpha_backing_stayed_warm_across_provider_restart: alphaStarts === 1,
        fake_alpha_proxy_restarted_with_provider: alphaProxyStarts === 2,
        fake_beta_started_after_expanded_grant: betaStarts === 1,
        fake_beta_tool_calls_after_resume: betaToolCalls,
      },
      note: 'This scenario uses provider-facing proxy stdio MCPs. Provider proxies restart with OpenCode, but backing MCP runtimes stay warm across provider restart.',
      servers: runs.map((run) => ({ agent_id: run.agentId, base_url: run.baseUrl, log_path: run.logPath })),
      snapshot,
    }
    await writeFile(path.join(dir, 'results.json'), JSON.stringify(result, null, 2), 'utf8')
    console.log(JSON.stringify(result, null, 2))
    if (!passed) process.exitCode = 1
  } finally {
    await Promise.all(runs.map((run) => run.stop()))
    await supervisor.stopAll()
  }
}

async function runCodexAgentTriggeredGrant() {
  const dir = await artifactDir('codex-agent-triggered-grant')
  const supervisor = new McpSupervisor({ artifactDir: dir })
  await supervisor.prepare()
  const beta = supervisor.proxiedProviderStdioConfig({ name: 'fake-beta', agentId: 'agent-a', backingMode: 'backing-per-agent' })
  const runs = []
  const sockets = []
  try {
    const firstRun = await launchCodexServer({ agentId: 'agent-a-before-grant', mcps: [], artifactDir: dir })
    runs.push(firstRun)
    const firstSocket = await firstRun.connectInitialized()
    sockets.push(firstSocket)
    const started = await firstSocket.threadStart({ cwd: spikeRoot, ephemeral: false })
    const threadId = started?.thread?.id
    if (!threadId) throw new Error(`Codex thread/start did not return a thread id: ${JSON.stringify(started)}`)
    const requestTurn = await firstSocket.turnStart(
      threadId,
      'For this drill, you need the fake-beta MCP before doing any work. Since it is not loaded yet, reply exactly: REQUEST_MCP fake-beta',
      { cwd: spikeRoot },
    )
    await firstSocket.waitForTurnCompleted(requestTurn.turnId)
    const requestTranscript = firstSocket.transcriptText()
    const requestedGrant = requestTranscript.includes('REQUEST_MCP fake-beta')
    await firstSocket.close()
    await firstRun.stop()

    const secondRun = await launchCodexServer({ agentId: 'agent-a-after-grant', mcps: [beta], artifactDir: dir })
    runs.push(secondRun)
    const secondSocket = await secondRun.connectInitialized()
    sockets.push(secondSocket)
    const resumed = await secondSocket.threadResume(threadId, { cwd: spikeRoot })
    const continuationTurn = await secondSocket.turnStart(
      threadId,
      'MCP is now loaded. Continue by calling fake-beta_echo with message "agent-triggered-grant".',
      { cwd: spikeRoot },
    )
    await secondSocket.waitForTurnCompleted(continuationTurn.turnId)

    const transcriptText = sockets.map((socket) => socket.transcriptText()).join('\n')
    const snapshot = await supervisor.snapshot()
    const betaStarts = countStarts(snapshot, (server) => server.name === 'fake-beta' && server.mode === 'backing-per-agent' && server.agent_id === 'agent-a')
    const betaProxyStarts = countStarts(snapshot, (server) => server.name === 'fake-beta' && server.mode === 'proxy' && server.agent_id === 'agent-a')
    const betaToolCalls = countToolCalls(snapshot, (server) => server.name === 'fake-beta' && server.mode === 'backing-per-agent' && server.agent_id === 'agent-a')
    const passed = resumed?.thread?.id === threadId && requestedGrant && betaStarts === 1 && betaProxyStarts === 1 && betaToolCalls >= 1
    const result = {
      scenario: 'codex-agent-triggered-grant',
      passed,
      artifact_dir: dir,
      executable: resolveCodexBinary(),
      thread_id: threadId,
      resumed_thread_id: resumed?.thread?.id ?? null,
      transcript_text: transcriptText,
      assertions: {
        thread_id_preserved: resumed?.thread?.id === threadId,
        assistant_requested_fake_beta: requestedGrant,
        fake_beta_backing_started_once_after_grant: betaStarts === 1,
        fake_beta_proxy_started_once_after_grant: betaProxyStarts === 1,
        fake_beta_tool_calls_after_continuation: betaToolCalls,
      },
      note: 'Simulates agent-triggered MCP grant: assistant requests fake-beta, Chariox relaunches provider with proxied MCP, then sends synthetic continuation.',
      servers: runs.map((run) => ({ agent_id: run.agentId, endpoint: run.endpoint, log_path: run.logPath })),
      snapshot,
    }
    await writeFile(path.join(dir, 'results.json'), JSON.stringify(result, null, 2), 'utf8')
    console.log(JSON.stringify(result, null, 2))
    if (!passed) process.exitCode = 1
  } finally {
    await Promise.all(sockets.map((socket) => socket.close()))
    await Promise.all(runs.map((run) => run.stop()))
    await supervisor.stopAll()
  }
}

async function runOpenCodeAgentTriggeredGrant() {
  const dir = await artifactDir('opencode-agent-triggered-grant')
  const supervisor = new McpSupervisor({ artifactDir: dir })
  await supervisor.prepare()
  const beta = supervisor.proxiedProviderStdioConfig({ name: 'fake-beta', agentId: 'agent-a', backingMode: 'backing-per-agent' })
  const runs = []
  try {
    const firstRun = await launchOpenCodeServer({ agentId: 'agent-a-before-grant', mcps: [], artifactDir: dir })
    runs.push(firstRun)
    const created = await firstRun.createSession({ directory: spikeRoot })
    const sessionId = created?.id
    if (!sessionId) throw new Error(`OpenCode session create did not return id: ${JSON.stringify(created)}`)
    await firstRun.prompt(
      sessionId,
      'For this drill, you need the fake-beta MCP before doing any work. Since it is not loaded yet, reply exactly: REQUEST_MCP fake-beta',
      { directory: spikeRoot },
    )
    const requestTranscript = await firstRun.transcriptText(sessionId, { directory: spikeRoot })
    const requestedGrant = requestTranscript.includes('REQUEST_MCP fake-beta')
    await firstRun.stop()

    const secondRun = await launchOpenCodeServer({ agentId: 'agent-a-after-grant', mcps: [beta], artifactDir: dir })
    runs.push(secondRun)
    const resumed = await secondRun.getSession(sessionId, { directory: spikeRoot })
    await secondRun.prompt(
      sessionId,
      'MCP is now loaded. Continue by calling fake-beta_echo with message "agent-triggered-grant".',
      { directory: spikeRoot },
    )
    const transcriptText = await secondRun.transcriptText(sessionId, { directory: spikeRoot })
    const snapshot = await supervisor.snapshot()
    const betaStarts = countStarts(snapshot, (server) => server.name === 'fake-beta' && server.mode === 'backing-per-agent' && server.agent_id === 'agent-a')
    const betaProxyStarts = countStarts(snapshot, (server) => server.name === 'fake-beta' && server.mode === 'proxy' && server.agent_id === 'agent-a')
    const betaToolCalls = countToolCalls(snapshot, (server) => server.name === 'fake-beta' && server.mode === 'backing-per-agent' && server.agent_id === 'agent-a')
    const passed = resumed?.id === sessionId && requestedGrant && betaStarts === 1 && betaProxyStarts === 1 && betaToolCalls >= 1
    const result = {
      scenario: 'opencode-agent-triggered-grant',
      passed,
      artifact_dir: dir,
      executable: resolveOpenCodeBinary(),
      session_id: sessionId,
      resumed_session_id: resumed?.id ?? null,
      transcript_text: transcriptText,
      assertions: {
        session_id_preserved: resumed?.id === sessionId,
        assistant_requested_fake_beta: requestedGrant,
        fake_beta_backing_started_once_after_grant: betaStarts === 1,
        fake_beta_proxy_started_once_after_grant: betaProxyStarts === 1,
        fake_beta_tool_calls_after_continuation: betaToolCalls,
      },
      note: 'Simulates agent-triggered MCP grant: assistant requests fake-beta, Chariox relaunches provider with proxied MCP, then sends synthetic continuation.',
      servers: runs.map((run) => ({ agent_id: run.agentId, base_url: run.baseUrl, log_path: run.logPath })),
      snapshot,
    }
    await writeFile(path.join(dir, 'results.json'), JSON.stringify(result, null, 2), 'utf8')
    console.log(JSON.stringify(result, null, 2))
    if (!passed) process.exitCode = 1
  } finally {
    await Promise.all(runs.map((run) => run.stop()))
    await supervisor.stopAll()
  }
}

async function runScaleMatrix() {
  const dir = await artifactDir('scale-matrix')
  const matrix = []
  const providers = ['codex', 'opencode']
  const agentCounts = [1, 2]
  const mcpsPerAgentValues = [1, 3]

  for (const provider of providers) {
    for (const agentCount of agentCounts) {
      for (const mcpsPerAgent of mcpsPerAgentValues) {
        const caseDir = path.join(dir, `${provider}-agents-${agentCount}-mcps-${mcpsPerAgent}`)
        const supervisor = new McpSupervisor({ artifactDir: caseDir })
        await supervisor.prepare()
        const startedAt = Date.now()
        const runs = []
        const sockets = []
        let passed = false
        let error = null
        try {
          for (let agentIndex = 0; agentIndex < agentCount; agentIndex += 1) {
            const agentId = `agent-${agentIndex + 1}`
            const mcps = []
            for (let mcpIndex = 0; mcpIndex < mcpsPerAgent; mcpIndex += 1) {
              mcps.push(supervisor.proxiedProviderStdioConfig({
                name: `fake-${mcpIndex + 1}`,
                agentId,
                backingMode: 'backing-per-agent',
              }))
            }
            if (provider === 'codex') {
              const run = await launchCodexServer({ agentId, mcps, artifactDir: caseDir })
              runs.push(run)
              const socket = await run.connectInitialized()
              sockets.push(socket)
              await socket.threadStart({ cwd: spikeRoot })
            } else {
              const run = await launchOpenCodeServer({ agentId, mcps, artifactDir: caseDir })
              runs.push(run)
              const status = await run.mcpStatus()
              for (const mcp of mcps) {
                if (status?.[mcp.name]?.status !== 'connected') {
                  throw new Error(`OpenCode ${agentId} did not connect ${mcp.name}: ${JSON.stringify(status)}`)
                }
              }
            }
          }
          const snapshot = await supervisor.snapshot()
          const backingStarts = countStarts(snapshot, (server) => server.mode === 'backing-per-agent')
          const proxyStarts = countStarts(snapshot, (server) => server.mode === 'proxy')
          const expected = agentCount * mcpsPerAgent
          passed = backingStarts === expected && proxyStarts === expected
          matrix.push({
            provider,
            agents: agentCount,
            mcps_per_agent: mcpsPerAgent,
            passed,
            elapsed_ms: Date.now() - startedAt,
            expected_mcp_count: expected,
            backing_starts: backingStarts,
            proxy_starts: proxyStarts,
            provider_processes_started: runs.length,
          })
        } catch (caught) {
          error = caught
          matrix.push({
            provider,
            agents: agentCount,
            mcps_per_agent: mcpsPerAgent,
            passed: false,
            elapsed_ms: Date.now() - startedAt,
            error: caught.message,
            provider_processes_started: runs.length,
          })
        } finally {
          await Promise.all(sockets.map((socket) => socket.close()))
          await Promise.all(runs.map((run) => run.stop()))
          await supervisor.stopAll()
        }
        if (error) {
          // Continue the matrix to collect all failures in one artifact.
          error = null
        }
      }
    }
  }

  const passed = matrix.every((entry) => entry.passed)
  const result = {
    scenario: 'scale-matrix',
    passed,
    artifact_dir: dir,
    matrix,
    note: 'Small smoke scale matrix. Codex uses app-server thread/start without a model turn; OpenCode uses /mcp status. This measures provider/proxy/backing lifecycle shape, not full model latency.',
  }
  await writeFile(path.join(dir, 'results.json'), JSON.stringify(result, null, 2), 'utf8')
  console.log(JSON.stringify(result, null, 2))
  if (!passed) process.exitCode = 1
}

function countServer(snapshot, { name, mode, agentId }) {
  return Object.values(snapshot.servers)
    .find((server) => server.name === name && server.mode === mode && server.agent_id === agentId)
}

function overlapAssertions(snapshot) {
  const expectedPresent = [
    ['agent-a', 'fake-alpha'],
    ['agent-a', 'fake-beta'],
    ['agent-b', 'fake-beta'],
    ['agent-b', 'fake-gamma'],
  ]
  const expectedAbsent = [
    ['agent-a', 'fake-gamma'],
    ['agent-b', 'fake-alpha'],
  ]
  const present = Object.fromEntries(expectedPresent.map(([agentId, name]) => {
    const backing = countServer(snapshot, { name, mode: 'backing-per-agent', agentId })
    const proxy = countServer(snapshot, { name, mode: 'proxy', agentId })
    return [`${agentId}:${name}`, {
      backing_starts: backing?.starts ?? 0,
      proxy_starts: proxy?.starts ?? 0,
      ok: backing?.starts === 1 && proxy?.starts === 1,
    }]
  }))
  const absent = Object.fromEntries(expectedAbsent.map(([agentId, name]) => {
    const backing = countServer(snapshot, { name, mode: 'backing-per-agent', agentId })
    const proxy = countServer(snapshot, { name, mode: 'proxy', agentId })
    return [`${agentId}:${name}`, {
      backing_starts: backing?.starts ?? 0,
      proxy_starts: proxy?.starts ?? 0,
      ok: !backing && !proxy,
    }]
  }))
  return {
    present,
    absent,
    passed: Object.values(present).every((value) => value.ok) && Object.values(absent).every((value) => value.ok),
  }
}

async function runOverlapIsolation() {
  const dir = await artifactDir('overlap-isolation')
  const providers = ['codex', 'opencode']
  const cases = []
  for (const provider of providers) {
    const caseDir = path.join(dir, provider)
    const supervisor = new McpSupervisor({ artifactDir: caseDir })
    await supervisor.prepare()
    const agents = [
      {
        id: 'agent-a',
        mcps: ['fake-alpha', 'fake-beta'].map((name) => supervisor.proxiedProviderStdioConfig({ name, agentId: 'agent-a', backingMode: 'backing-per-agent' })),
      },
      {
        id: 'agent-b',
        mcps: ['fake-beta', 'fake-gamma'].map((name) => supervisor.proxiedProviderStdioConfig({ name, agentId: 'agent-b', backingMode: 'backing-per-agent' })),
      },
    ]
    const runs = []
    const sockets = []
    let caseResult
    try {
      if (provider === 'codex') {
        for (const agent of agents) {
          const run = await launchCodexServer({ agentId: agent.id, mcps: agent.mcps, artifactDir: caseDir })
          runs.push(run)
          const socket = await run.connectInitialized()
          sockets.push(socket)
          await socket.threadStart({ cwd: spikeRoot })
        }
      } else {
        const statuses = {}
        for (const agent of agents) {
          const run = await launchOpenCodeServer({ agentId: agent.id, mcps: agent.mcps, artifactDir: caseDir })
          runs.push(run)
          statuses[agent.id] = await run.mcpStatus()
        }
        const agentAKeys = Object.keys(statuses['agent-a'] ?? {}).sort()
        const agentBKeys = Object.keys(statuses['agent-b'] ?? {}).sort()
        const statusOk = JSON.stringify(agentAKeys) === JSON.stringify(['fake-alpha', 'fake-beta'])
          && JSON.stringify(agentBKeys) === JSON.stringify(['fake-beta', 'fake-gamma'])
        if (!statusOk) {
          throw new Error(`OpenCode MCP status mismatch: ${JSON.stringify(statuses)}`)
        }
      }
      const snapshot = await supervisor.snapshot()
      const assertions = overlapAssertions(snapshot)
      caseResult = {
        provider,
        passed: assertions.passed,
        assertions,
        provider_processes_started: runs.length,
        snapshot,
      }
    } catch (error) {
      caseResult = {
        provider,
        passed: false,
        error: error.message,
        provider_processes_started: runs.length,
      }
    } finally {
      await Promise.all(sockets.map((socket) => socket.close()))
      await Promise.all(runs.map((run) => run.stop()))
      await supervisor.stopAll()
    }
    cases.push(caseResult)
    await writeFile(path.join(caseDir, 'results.json'), JSON.stringify(caseResult, null, 2), 'utf8')
  }
  const passed = cases.every((entry) => entry.passed)
  const result = {
    scenario: 'overlap-isolation',
    passed,
    artifact_dir: dir,
    grants: {
      'agent-a': ['fake-alpha', 'fake-beta'],
      'agent-b': ['fake-beta', 'fake-gamma'],
    },
    cases: cases.map((entry) => ({
      provider: entry.provider,
      passed: entry.passed,
      assertions: entry.assertions,
      provider_processes_started: entry.provider_processes_started,
      error: entry.error,
    })),
    note: 'Validates overlapping-but-not-identical MCP grants per provider. With backing-per-agent, fake-beta has separate backing runtimes for agent-a and agent-b.',
  }
  await writeFile(path.join(dir, 'results.json'), JSON.stringify(result, null, 2), 'utf8')
  console.log(JSON.stringify(result, null, 2))
  if (!passed) process.exitCode = 1
}

function printHelp() {
  console.log([
    'Usage: node src/scenarios.mjs <scenario>',
    '',
    'Scenarios:',
    '  lifecycle        Validate supervised MCP backing process reuse across grant growth.',
    '  visibility-plan  Generate Codex/OpenCode per-agent MCP config plans and probe fake MCP framing.',
    '  opencode-status  Launch two isolated OpenCode servers and verify MCP status isolation.',
    '  codex-thread-start  Launch two isolated Codex app-servers and verify MCP startup isolation.',
    '  codex-restart-resume  Restart Codex with expanded MCP config and resume the same thread.',
    '  opencode-restart-resume  Restart OpenCode with expanded MCP config and reuse the same session.',
    '  codex-agent-triggered-grant  Simulate a Codex agent-requested MCP grant and synthetic continuation.',
    '  opencode-agent-triggered-grant  Simulate an OpenCode agent-requested MCP grant and synthetic continuation.',
    '  scale-matrix  Run a small provider/proxy/backing lifecycle scale matrix.',
    '  overlap-isolation  Validate overlapping-but-not-identical MCP grants across two agents per provider.',
  ].join('\n'))
}

const scenario = process.argv[2]
if (!scenario || scenario === '--help') {
  printHelp()
} else if (scenario === 'lifecycle') {
  await runLifecycle()
} else if (scenario === 'visibility-plan') {
  await runVisibilityPlan()
} else if (scenario === 'opencode-status') {
  await runOpenCodeStatus()
} else if (scenario === 'codex-thread-start') {
  await runCodexThreadStart()
} else if (scenario === 'codex-restart-resume') {
  await runCodexRestartResume()
} else if (scenario === 'opencode-restart-resume') {
  await runOpenCodeRestartResume()
} else if (scenario === 'codex-agent-triggered-grant') {
  await runCodexAgentTriggeredGrant()
} else if (scenario === 'opencode-agent-triggered-grant') {
  await runOpenCodeAgentTriggeredGrant()
} else if (scenario === 'scale-matrix') {
  await runScaleMatrix()
} else if (scenario === 'overlap-isolation') {
  await runOverlapIsolation()
} else {
  throw new Error(`unknown scenario: ${scenario}`)
}
