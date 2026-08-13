#!/usr/bin/env node
import { spawn } from 'node:child_process'
import { createWriteStream } from 'node:fs'
import { mkdir, readdir, readFile, stat, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const repoRoot = path.resolve(cliRoot, '..', '..')
const DEFAULT_TIMEOUT_MS = 900_000
const DEFAULT_POLL_MS = 1_000
const DEFAULT_PROVIDER = process.env.CHARIOX_METAAGENT_PROVIDER_IMPORT_PROVIDER ?? 'codex'
const DEFAULT_MODEL = process.env.CHARIOX_METAAGENT_PROVIDER_IMPORT_MODEL ?? 'gpt-5.5'
const DEFAULT_EFFORT = process.env.CHARIOX_METAAGENT_PROVIDER_IMPORT_EFFORT ?? 'medium'
const CHARIOX_ONLY_MCP = 'ma-chariox-only-mcp'
const CHARIOX_ONLY_SKILL = 'ma-chariox-only-skill'
const WORKER_MARKER = 'PROVIDER_CAPABILITY_IMPORT_DRILL_COMPLETE'

function parseArgs(argv) {
  const options = {
    provider: DEFAULT_PROVIDER,
    model: DEFAULT_MODEL,
    effort: DEFAULT_EFFORT,
    accountProfile: 'default',
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    keepArtifactsOnFailure: true,
    preserveOnSuccess: true,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--') continue
    else if (arg === '--provider') options.provider = String(argv[++index] ?? '').trim()
    else if (arg === '--model') options.model = String(argv[++index] ?? '').trim()
    else if (arg === '--effort') options.effort = String(argv[++index] ?? '').trim()
    else if (arg === '--account-profile') options.accountProfile = String(argv[++index] ?? '').trim()
    else if (arg === '--timeout-ms') options.timeoutMs = Number(argv[++index])
    else if (arg === '--poll-ms') options.pollMs = Number(argv[++index])
    else if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--discard-artifacts-on-failure') options.keepArtifactsOnFailure = false
    else if (arg === '--preserve-on-success') options.preserveOnSuccess = true
    else if (arg === '--discard-artifacts-on-success') options.preserveOnSuccess = false
    else if (arg === '--help' || arg === '-h') {
      console.log([
        'Usage: node apps/cli/scripts/live-metaagent-provider-capability-import-drill.mjs [options]',
        '',
        'Runs provider capability import validation:',
        '- uses real Codex MCPs and skills as provider import sources',
        '- validates the shared extension import providers command in an isolated Chariox registry',
        '- runs a real metaagent and observes it import/grant capabilities to a worker',
        '- preserves drill artifacts by default, including successful runs',
      ].join('\n'))
      process.exit(0)
    } else {
      throw new Error(`unknown option: ${arg}`)
    }
  }
  if (!options.provider || options.provider === 'dev-stub') {
    throw new Error('provider capability import drill requires a real provider; dev-stub is not valid evidence')
  }
  if (!options.model) throw new Error('--model is required')
  if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) throw new Error('--timeout-ms must be positive')
  if (!Number.isFinite(options.pollMs) || options.pollMs <= 0) throw new Error('--poll-ms must be positive')
  return options
}

function makePorts(base = 62000) {
  const maxOffset = 65_535 - base - 2_001
  if (maxOffset <= 0) throw new Error(`port base ${base} leaves no room for derived drill ports`)
  const kernelPort = base + Math.floor(Math.random() * Math.min(600, maxOffset))
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
  }
}

function log(name, details) {
  if (details === undefined) console.log(`[metaagent-provider-import-drill] ${name}`)
  else console.log(`[metaagent-provider-import-drill] ${name}`, JSON.stringify(details))
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

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

async function runChecked(command, args, options = {}) {
  const result = await run(command, args, options)
  if (result.code !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
  }
  return result
}

function assert(condition, message, details) {
  if (!condition) throw new Error(`${message}${details ? `\n${JSON.stringify(details, null, 2)}` : ''}`)
}

function unwrap(response, key) {
  return response?.[key] ?? response
}

function unwrapVariant(response, ...keys) {
  return keys.map((key) => response?.[key]).find((value) => value != null) ?? response
}

async function buildKernel() {
  const binary = path.join(repoRoot, 'apps/kernel/target/debug/chariox-kernel')
  await runChecked('cargo', ['build', '--manifest-path', path.join(repoRoot, 'apps/kernel/Cargo.toml'), '--bin', 'chariox-kernel'])
  const existing = await stat(binary).then((info) => info.isFile()).catch(() => false)
  if (!existing) throw new Error(`kernel build did not produce ${binary}`)
  return binary
}

async function initGitWorktree(root) {
  await runChecked('git', ['init', '-b', 'main'], { cwd: root })
  await runChecked('git', ['config', 'user.email', 'metaagent-provider-import-drill@example.com'], { cwd: root })
  await runChecked('git', ['config', 'user.name', 'Metaagent Provider Import Drill'], { cwd: root })
}

async function writeWorkspaceFixture(workspace) {
  await writeFile(path.join(workspace, 'README.md'), [
    '# Provider Capability Import Drill',
    '',
    'This repository exists only to validate metaagent capability provisioning.',
    `A worker should report ${WORKER_MARKER} after receiving the requested capabilities.`,
    '',
  ].join('\n'), 'utf8')
  await writeFile(path.join(workspace, '.gitignore'), '.chariox-wait.chariox\n.charioxignore\n', 'utf8')
}

async function waitForDaemon(shellBin, kernelUrl, workspace, scriptsDir, env) {
  const scriptPath = path.join(scriptsDir, 'wait.chariox')
  await writeFile(scriptPath, 'session list\n', 'utf8')
  const deadline = Date.now() + 20_000
  let last = null
  while (Date.now() < deadline) {
    last = await run(process.execPath, [shellBin, 'run', scriptPath, '--kernel-url', kernelUrl, '--workspace', workspace, '--worktree', workspace], { env })
    if (last.code === 0) return
    await sleep(250)
  }
  throw new Error(`daemon did not become ready\nstdout:\n${last?.stdout ?? ''}\nstderr:\n${last?.stderr ?? ''}`)
}

async function terminateChild(child, signal = 'SIGTERM') {
  if (!child || child.exitCode != null || child.signalCode != null) return
  child.kill(signal)
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    sleep(5_000),
  ])
  if (child.exitCode == null && child.signalCode == null) {
    child.kill('SIGKILL')
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      sleep(2_000),
    ])
  }
}

async function cleanupSession(kernelUrl, sessionId) {
  if (!sessionId) return
  const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
  const { endSessionRequest } = await import('../../../packages/kernel-client/dist/ipc-requests.js')
  const client = new LocalIpcClient(kernelUrl)
  try {
    await client.send(endSessionRequest(sessionId)).catch(() => {})
  } finally {
    await client.close().catch(() => {})
  }
}

function requireOutput(output, pattern, label) {
  if (!pattern.test(output)) {
    throw new Error(`missing ${label}: ${pattern}\n--- output ---\n${output}`)
  }
}

function parseSummaryCount(output, name) {
  const match = output.match(new RegExp(`${name}=(\\d+)`))
  return match ? Number(match[1]) : 0
}

async function runShellScript({ shellBin, kernelUrl, workspace, scriptsDir, env, name, lines, vars = {} }) {
  const scriptPath = path.join(scriptsDir, `${name}.chariox`)
  await writeFile(scriptPath, `${lines.join('\n')}\n`, 'utf8')
  const args = [
    shellBin,
    'run',
    scriptPath,
    '--kernel-url',
    kernelUrl,
    '--workspace',
    workspace,
    '--worktree',
    workspace,
  ]
  for (const [key, value] of Object.entries(vars)) {
    args.push('--var', `${key}=${value}`)
  }
  const result = await run(process.execPath, args, { env })
  await writeFile(path.join(scriptsDir, `${name}.stdout.log`), result.stdout, 'utf8')
  await writeFile(path.join(scriptsDir, `${name}.stderr.log`), result.stderr, 'utf8')
  if (result.code !== 0) {
    throw new Error(`shell script ${name} failed\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`)
  }
  return result
}

async function readHistoryEntries(historyDir) {
  const files = (await readdir(historyDir).catch(() => []))
    .filter((file) => file.endsWith('.jsonl'))
    .sort()
  const entries = []
  for (const file of files) {
    const text = await readFile(path.join(historyDir, file), 'utf8').catch(() => '')
    for (const [index, line] of text.split(/\r?\n/).entries()) {
      if (!line.trim()) continue
      try {
        entries.push({ file, line: index + 1, ...JSON.parse(line) })
      } catch {
        entries.push({ file, line: index + 1, parse_error: true, raw: line.slice(0, 300) })
      }
    }
  }
  return entries
}

function parseProviderToolText(text) {
  if (typeof text !== 'string' || !text.trim().startsWith('{')) return null
  try {
    return JSON.parse(text)
  } catch {
    return null
  }
}

function metaagentToolIsAllowed(toolName) {
  if (typeof toolName !== 'string') return false
  return toolName.startsWith('chariox.')
    || toolName.startsWith('mcp__chariox__')
    || toolName.startsWith('mcp__chariox.')
}

function listMetaagentEventsRequest(sessionId, metaagentId, limit = 100) {
  return {
    ListMetaagentEvents: {
      session_id: sessionId,
      metaagent_id: metaagentId,
      limit,
    },
  }
}

async function getSession(client, requests, sessionId) {
  return unwrapVariant(await client.send(requests.getSessionStateRequest(sessionId)), 'SessionState', 'SessionStateLoaded').session
}

async function waitForProviderRun(client, requests, providerRunId, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    last = unwrap(await client.send(requests.getProviderRunRequest(providerRunId)), 'ProviderRun').provider_run
    if (last?.state === 'Running' || last?.state === 'Active' || last?.runtime_mcp_server_url) return last
    if (last?.state === 'Ended') throw new Error(`provider run ended before becoming active: ${JSON.stringify(last)}`)
    await sleep(pollMs)
  }
  throw new Error(`provider run did not become active: ${JSON.stringify(last)}`)
}

async function waitForAgentProviderRun(client, requests, sessionId, agentId, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastSession = null
  while (Date.now() < deadline) {
    lastSession = await getSession(client, requests, sessionId)
    const providerRunId = lastSession.active_provider_run_id
    if (providerRunId) {
      const run = unwrap(await client.send(requests.getProviderRunRequest(providerRunId)), 'ProviderRun').provider_run
      if (run?.agent_instance_id === agentId || run?.agent_id === agentId) {
        return await waitForProviderRun(client, requests, providerRunId, timeoutMs, pollMs)
      }
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for provider run for agent ${agentId}: ${JSON.stringify(lastSession)}`)
}

function userPrompt({ providerMcpName, providerSkillName }) {
  return [
    'Coordinate a small capability provisioning check.',
    '',
    'A regular worker needs these capabilities before doing the check:',
    `- Chariox MCP ${CHARIOX_ONLY_MCP}`,
    `- Chariox skill ${CHARIOX_ONLY_SKILL}`,
    `- local Codex MCP ${providerMcpName}`,
    `- local Codex skill ${providerSkillName}`,
    '',
    'Provision whatever is needed in Chariox, give the capabilities to a worker, and ask the worker to inspect README.md.',
    `The worker must reply with ${WORKER_MARKER} and name the capabilities it received.`,
    'When the worker has responded, mark this metaagent task complete with a concise report.',
    'Do not do the worker task yourself.',
  ].join('\n')
}

function summarizeEvent(event) {
  return {
    sequence: event.sequence,
    kind: event.kind,
    sourceAgentId: event.source_agent_id ?? null,
    delivery: event.prompt_delivery_status ?? null,
    title: event.title,
    summary: event.summary,
  }
}

function agentHasGrant(agent, kind, name) {
  return (agent.extension_grants ?? []).some((grant) => grant.kind === kind && grant.name === name)
}

function commandMatches(command, prefix, name = null) {
  const value = String(command ?? '').trim()
  return value.startsWith(prefix) && (name == null || value.includes(name))
}

async function observeMetaagentProvisioning({
  client,
  requests,
  sessionId,
  metaagentId,
  historyDir,
  beforeAgentIds,
  options,
  providerMcpName,
  providerSkillName,
}) {
  const deadline = Date.now() + options.timeoutMs
  const seenHistoryTools = new Set()
  const seenEvents = new Set()
  const commands = []
  let finalSession = null
  let finalTask = null
  let finalEvents = []
  let finalWorkers = []
  let sawWorkerMarker = false

  while (Date.now() < deadline) {
    const session = await getSession(client, requests, sessionId)
    finalSession = session
    const task = (session.metaagent_tasks ?? []).find((entry) => entry.metaagent_id === metaagentId)
    finalTask = task ?? null
    const workers = (session.agents ?? []).filter((agent) => !beforeAgentIds.has(agent.id) && agent.id !== metaagentId)
    finalWorkers = workers
    const workerIds = new Set(workers.map((agent) => agent.id))

    if (workers.length > 0) {
      log('workers-observed', workers.map((agent) => ({
        id: agent.id,
        alias: agent.alias ?? null,
        role: agent.role ?? null,
        provider: agent.provider,
        model: agent.model ?? null,
        grants: (agent.extension_grants ?? []).map((grant) => `${grant.kind}:${grant.name}`),
      })))
    }

    const historyEntries = await readHistoryEntries(historyDir)
    for (const entry of historyEntries) {
      if (entry.agent_id && workerIds.has(entry.agent_id) && String(entry.text ?? '').includes(WORKER_MARKER)) {
        sawWorkerMarker = true
      }
      if (entry.kind !== 'provider_tool') continue
      if (entry.agent_id !== metaagentId && !workerIds.has(entry.agent_id)) continue
      const tool = parseProviderToolText(entry.text)
      if (!tool?.tool) continue
      const historyKey = `${entry.file}:${entry.line}:${entry.agent_id}:${entry.merge_key ?? ''}:${tool.status ?? ''}`
      if (seenHistoryTools.has(historyKey)) continue
      seenHistoryTools.add(historyKey)
      if (entry.agent_id === metaagentId && !metaagentToolIsAllowed(tool.tool)) {
        throw new Error(`metaagent used disallowed provider-native tool ${tool.tool} at ${entry.file}:${entry.line}`)
      }
      const command = tool.input?.command ?? null
      if (entry.agent_id === metaagentId && command) commands.push(String(command))
      log('history-tool-observed', {
        agentId: entry.agent_id,
        role: entry.agent_id === metaagentId ? 'meta' : 'worker',
        tool: tool.tool,
        status: tool.status ?? null,
        command,
      })
    }

    const eventsPayload = unwrap(await client.send(listMetaagentEventsRequest(sessionId, metaagentId, 100)), 'MetaagentEventsListed')
    finalEvents = eventsPayload.events ?? []
    for (const event of finalEvents) {
      if (seenEvents.has(event.event_id)) continue
      seenEvents.add(event.event_id)
      log('metaagent-event', summarizeEvent(event))
    }

    if (task?.status) {
      log('task-observed', {
        status: task.status,
        revision: task.revision ?? null,
        planLength: task.plan_markdown?.length ?? 0,
        summary: task.completion_summary ?? task.blocked_reason ?? task.aborted_reason ?? null,
      })
    }
    if (task?.status === 'blocked' || task?.status === 'aborted') {
      throw new Error(`metaagent task ended as ${task.status}: ${task.blocked_reason ?? task.aborted_reason ?? 'no reason'}`)
    }

    const provisionedWorker = workers.find((agent) =>
      agentHasGrant(agent, 'mcp', CHARIOX_ONLY_MCP)
      && agentHasGrant(agent, 'skill', CHARIOX_ONLY_SKILL)
      && agentHasGrant(agent, 'mcp', providerMcpName)
      && agentHasGrant(agent, 'skill', providerSkillName)
    )
    if (task?.status === 'completed' && provisionedWorker && sawWorkerMarker) {
      const sawMcpList = commands.some((command) => commandMatches(command, 'mcp list') || commandMatches(command, 'mcp show'))
      const sawSkillList = commands.some((command) => commandMatches(command, 'skill list') || commandMatches(command, 'skill show'))
      const sawProviderImport = commands.some((command) =>
        commandMatches(command, 'extension import providers')
        || commandMatches(command, 'mcp import codex', providerMcpName)
        || commandMatches(command, 'skill import codex', providerSkillName)
      )
      const sawCharioxOnlyMcpGrant = commands.some((command) => commandMatches(command, 'mcp grant', CHARIOX_ONLY_MCP))
      const sawCharioxOnlySkillGrant = commands.some((command) => commandMatches(command, 'skill grant', CHARIOX_ONLY_SKILL))
      const sawProviderMcpGrant = commands.some((command) => commandMatches(command, 'mcp grant', providerMcpName))
      const sawProviderSkillGrant = commands.some((command) => commandMatches(command, 'skill grant', providerSkillName))
      assert(sawMcpList, 'metaagent should list/show Chariox MCPs before granting', { commands })
      assert(sawSkillList, 'metaagent should list/show Chariox skills before granting', { commands })
      assert(sawProviderImport, 'metaagent should import missing provider capabilities into Chariox before granting', { commands })
      assert(sawCharioxOnlyMcpGrant, 'metaagent should grant the Chariox-only MCP', { commands })
      assert(sawCharioxOnlySkillGrant, 'metaagent should grant the Chariox-only skill', { commands })
      assert(sawProviderMcpGrant, 'metaagent should grant the imported provider MCP', { commands })
      assert(sawProviderSkillGrant, 'metaagent should grant the imported provider skill', { commands })
      return {
        task,
        worker: provisionedWorker,
        workers,
        events: finalEvents,
        commands,
      }
    }

    if (task?.status === 'completed') {
      throw new Error(`metaagent completed without validated provisioning: ${JSON.stringify({
        sawWorkerMarker,
        workers: workers.map((agent) => ({
          id: agent.id,
          alias: agent.alias,
          grants: agent.extension_grants,
        })),
        commands,
        summary: task.completion_summary ?? null,
      }, null, 2)}`)
    }

    await sleep(options.pollMs)
  }

  throw new Error(`timed out waiting for provider capability drill completion\nlast=${JSON.stringify({
    task: finalTask,
    agents: finalSession?.agents?.map((agent) => ({ id: agent.id, alias: agent.alias, role: agent.role, provider: agent.provider, grants: agent.extension_grants })),
    events: finalEvents.map(summarizeEvent),
    workers: finalWorkers.map((agent) => agent.id),
    sawWorkerMarker,
    commands,
  }, null, 2)}`)
}

async function writeMcpServer(serverPath) {
  await writeFile(serverPath, [
    'let buffer = ""',
    'process.stdin.setEncoding("utf8")',
    'function write(payload) { process.stdout.write(JSON.stringify(payload) + "\\n") }',
    'process.stdin.on("data", (chunk) => {',
    '  buffer += chunk',
    '  let index',
    '  while ((index = buffer.indexOf("\\n")) >= 0) {',
    '    const line = buffer.slice(0, index).trim()',
    '    buffer = buffer.slice(index + 1)',
    '    if (!line) continue',
    '    let request',
    '    try { request = JSON.parse(line) } catch { continue }',
    '    const { id, method, params } = request',
    '    if (method === "notifications/initialized") continue',
    '    if (method === "initialize") {',
    '      write({ jsonrpc: "2.0", id, result: { protocolVersion: "2024-11-05", capabilities: { tools: {} }, serverInfo: { name: "ma-chariox-only-mcp", version: "1.0.0" } } })',
    '      continue',
    '    }',
    '    if (method === "tools/list") {',
    '      write({ jsonrpc: "2.0", id, result: { tools: [{ name: "capability_marker", description: "Returns the metaagent provider import drill marker.", inputSchema: { type: "object", properties: {}, additionalProperties: false } }] } })',
    '      continue',
    '    }',
    '    if (method === "tools/call" && params?.name === "capability_marker") {',
    '      write({ jsonrpc: "2.0", id, result: { content: [{ type: "text", text: "CHARIOX_ONLY_MCP_OK" }] } })',
    '      continue',
    '    }',
    '    write({ jsonrpc: "2.0", id, error: { code: -32601, message: `unknown method ${method}` } })',
    '  }',
    '})',
  ].join('\n'), 'utf8')
}

async function findCodexSkillName() {
  const roots = [
    path.join(os.homedir(), '.codex', 'skills'),
    path.join(os.homedir(), '.agents', 'skills'),
  ]
  const preferred = 'frontend-design'
  for (const root of roots) {
    const preferredPath = path.join(root, preferred, 'SKILL.md')
    if (await stat(preferredPath).then((info) => info.isFile()).catch(() => false)) return preferred
  }
  for (const root of roots) {
    const names = await readdir(root, { withFileTypes: true }).catch(() => [])
    const match = names.find((entry) =>
      entry.isDirectory()
      && !entry.name.startsWith('.')
      && entry.name !== CHARIOX_ONLY_SKILL
    )
    if (match) return match.name
  }
  throw new Error('no real Codex skill found under ~/.codex/skills or ~/.agents/skills')
}

async function findCodexMcpName() {
  const configPath = path.join(os.homedir(), '.codex', 'config.toml')
  const text = await readFile(configPath, 'utf8').catch(() => '')
  const names = []
  const pattern = /^\[mcp_servers\.("?)([^"\]\s]+)\1\]/gm
  let match
  while ((match = pattern.exec(text)) != null) {
    names.push(match[2])
  }
  if (names.includes('node_repl')) return 'node_repl'
  if (names.length > 0) return names[0]
  throw new Error('no real Codex MCP found in ~/.codex/config.toml')
}

async function startKernel({ rootDir, kernelBinary, workspace, scriptsDir, capabilityRoot, ports, shellBin }) {
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const env = {
    ...process.env,
    CHARIOX_KERNEL_PORT: String(ports.kernelPort),
    CHARIOX_MCP_PORT: String(ports.mcpPort),
    CHARIOX_OPENCODE_PORT: String(ports.opencodePort),
    CHARIOX_CODEX_PORT: String(ports.codexPort),
    CHARIOX_DAEMON_ID: `metaagent-provider-import-drill-${process.pid}-${Date.now()}`,
    CHARIOX_DAEMON_SOCKET: path.join(rootDir, 'daemon.sock'),
    CHARIOX_SESSION_HISTORY_DIR: path.join(rootDir, 'history'),
    CHARIOX_CAPABILITY_ISOLATION_ROOT: capabilityRoot,
  }
  const kernelStdout = createWriteStream(path.join(rootDir, 'kernel.stdout.log'), { flags: 'a' })
  const kernelStderr = createWriteStream(path.join(rootDir, 'kernel.stderr.log'), { flags: 'a' })
  const daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ['ignore', 'pipe', 'pipe'] })
  daemon.stdout.pipe(kernelStdout)
  daemon.stderr.pipe(kernelStderr)
  await waitForDaemon(shellBin, kernelUrl, workspace, scriptsDir, env)
  return { daemon, env, kernelUrl }
}

async function runProviderImportCommandDrill({ rootDir, kernelBinary, shellBin, providerMcpName, providerSkillName }) {
  const workspace = path.join(rootDir, 'workspace')
  const scriptsDir = path.join(rootDir, 'scripts')
  const capabilityRoot = path.join(rootDir, 'capabilities')
  await mkdir(workspace, { recursive: true })
  await mkdir(scriptsDir, { recursive: true })
  await writeWorkspaceFixture(workspace)
  await initGitWorktree(workspace)
  const runtime = await startKernel({
    rootDir,
    kernelBinary,
    workspace,
    scriptsDir,
    capabilityRoot,
    ports: makePorts(61200),
    shellBin,
  })
  try {
    const dry = await runShellScript({
      shellBin,
      kernelUrl: runtime.kernelUrl,
      workspace,
      scriptsDir,
      env: runtime.env,
      name: 'provider-import-dry-run',
      lines: ['extension import providers --provider codex --kind all --dry-run'],
    })
    requireOutput(dry.stdout, /Provider capability import dry run/, 'dry-run provider import report')
    assert(parseSummaryCount(dry.stdout, 'candidates') > 0, 'dry run should discover real Codex provider candidates', dry.stdout)
    requireOutput(dry.stdout, new RegExp(`(?:mcp|skill) ${providerMcpName}|(?:mcp|skill) ${providerSkillName}`), 'real Codex provider capability in dry run')

    const imported = await runShellScript({
      shellBin,
      kernelUrl: runtime.kernelUrl,
      workspace,
      scriptsDir,
      env: runtime.env,
      name: 'provider-import',
      lines: ['extension import providers --provider codex --kind all'],
    })
    requireOutput(imported.stdout, /Provider capability import:/, 'provider import report')
    assert(parseSummaryCount(imported.stdout, 'imported') > 0 || parseSummaryCount(imported.stdout, 'updated') > 0, 'real import should import or update at least one capability', imported.stdout)

    const repeated = await runShellScript({
      shellBin,
      kernelUrl: runtime.kernelUrl,
      workspace,
      scriptsDir,
      env: runtime.env,
      name: 'provider-import-repeat',
      lines: [
        'extension import providers --provider codex --kind all',
        'mcp list',
        'skill list',
      ],
    })
    assert(parseSummaryCount(repeated.stdout, 'already_installed') > 0, 'repeat import should report already installed provider capabilities', repeated.stdout)
    requireOutput(repeated.stdout, new RegExp(providerMcpName.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')), 'imported Codex MCP listed')
    requireOutput(repeated.stdout, new RegExp(providerSkillName.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')), 'imported Codex skill listed')

    return {
      kernelUrl: runtime.kernelUrl,
      dryRunOutput: dry.stdout,
      importOutput: imported.stdout,
      repeatOutput: repeated.stdout,
    }
  } finally {
    await terminateChild(runtime.daemon)
  }
}

async function runRealMetaagentDrill({ rootDir, kernelBinary, shellBin, options, providerMcpName, providerSkillName }) {
  const workspace = path.join(rootDir, 'workspace')
  const scriptsDir = path.join(rootDir, 'scripts')
  const skillDir = path.join(rootDir, 'chariox-only-skill')
  const mcpServer = path.join(rootDir, 'chariox-only-mcp-server.mjs')
  const capabilityRoot = path.join(rootDir, 'capabilities')
  await mkdir(workspace, { recursive: true })
  await mkdir(scriptsDir, { recursive: true })
  await mkdir(skillDir, { recursive: true })
  await writeWorkspaceFixture(workspace)
  await initGitWorktree(workspace)
  await writeMcpServer(mcpServer)
  await writeFile(path.join(skillDir, 'SKILL.md'), [
    '---',
    `name: ${CHARIOX_ONLY_SKILL}`,
    'description: Chariox-only skill for the live provider capability import drill',
    '---',
    '',
    `Use this skill only for the live provider capability import drill. Mention ${CHARIOX_ONLY_SKILL} when asked what capability was granted.`,
    '',
  ].join('\n'), 'utf8')

  const runtime = await startKernel({
    rootDir,
    kernelBinary,
    workspace,
    scriptsDir,
    capabilityRoot,
    ports: makePorts(62600),
    shellBin,
  })
  let client = null
  let sessionId = null
  try {
    const setup = await runShellScript({
      shellBin,
      kernelUrl: runtime.kernelUrl,
      workspace,
      scriptsDir,
      env: runtime.env,
      name: 'setup-metaagent',
      vars: {
        workspace,
        node_bin: process.execPath,
        mcp_server: mcpServer,
        skill_dir: skillDir,
      },
      lines: [
        `mcp install ${CHARIOX_ONLY_MCP} --command $node_bin --arg $mcp_server`,
        'skill install $skill_dir',
        `set provider ${options.provider}`,
        `set model ${options.model}`,
        `set effort ${options.effort}`,
        'session new $workspace as session',
        'session mode build',
        'session permissions yolo',
        'agent list',
      ],
    })
    requireOutput(setup.stdout, /installed MCP|installed mcp|mcp/i, 'Chariox-only MCP install')
    requireOutput(setup.stdout, /installed skill/i, 'Chariox-only skill install')
    sessionId = setup.stdout.match(/bound \$session = (\S+)/)?.[1] ?? null
    assert(sessionId, 'setup script did not bind session id', { stdout: setup.stdout })

    const { LocalIpcClient } = await import('../../../packages/kernel-client/dist/ipc.js')
    const requests = await import('../../../packages/kernel-client/dist/ipc-requests.js')
    client = new LocalIpcClient(runtime.kernelUrl, {
      kernelMaxMissedPongs: Math.max(120, Math.ceil(options.timeoutMs / 5_000)),
    })
    const attachment = unwrap(await client.send(requests.attachToSessionRequest(sessionId, `metaagent-provider-import-drill-${Date.now()}`)), 'SessionAttached').attachment
    let initialSession = await getSession(client, requests, sessionId)
    let metaagent = (initialSession.agents ?? []).find((agent) => agent.id === initialSession.focused_agent_id) ?? (initialSession.agents ?? [])[0]
    assert(metaagent, 'session should contain a default regular agent', initialSession)
    assert(!metaagent.meta_mode, 'default agent must start outside meta mode', metaagent)
    await client.send(requests.updateAgentProfileRequest({
      sessionId,
      agentId: metaagent.id,
      provider: options.provider,
      model: options.model,
      effort: options.effort,
    }))
    initialSession = await getSession(client, requests, sessionId)
    metaagent = (initialSession.agents ?? []).find((agent) => agent.id === metaagent.id)
    assert(
      metaagent?.provider === options.provider && metaagent?.model === options.model && metaagent?.effort === options.effort,
      'default agent profile should match requested drill provider/model/effort before /meta',
      { metaagent, expected: { provider: options.provider, model: options.model, effort: options.effort } },
    )
    const beforeAgentIds = new Set((initialSession.agents ?? []).map((agent) => agent.id))

    const prompt = userPrompt({ providerMcpName, providerSkillName })
    await writeFile(path.join(rootDir, 'metaagent-user-prompt.txt'), prompt, 'utf8')
    const metaPrompt = `/meta ${prompt}`
    await client.send(requests.submitPromptRequest(sessionId, attachment.id, metaagent.id, metaPrompt, []))
    log('single-prompt-submitted', { metaagentId: metaagent.id, prompt: metaPrompt })

    const metaRun = await waitForAgentProviderRun(client, requests, sessionId, metaagent.id, options.timeoutMs, options.pollMs)
    assert(metaRun.adapter_key !== 'dev-stub' && metaRun.provider !== 'dev-stub', 'metaagent must run on a real provider', metaRun)
    assert(metaRun.execution_mode === 'plan', 'meta-mode provider run must be plan mode', metaRun)
    const metaSession = await getSession(client, requests, sessionId)
    const metaModeAgent = (metaSession.agents ?? []).find((agent) => agent.id === metaagent.id)
    assert(metaModeAgent?.meta_mode, 'same regular agent should enter meta mode after /meta prompt', metaModeAgent)
    log('metaagent-run-observed', {
      providerRunId: metaRun.id,
      provider: metaRun.provider,
      adapterKey: metaRun.adapter_key,
      executionMode: metaRun.execution_mode,
      permissionLevel: metaRun.permission_level ?? null,
    })

    const observed = await observeMetaagentProvisioning({
      client,
      requests,
      sessionId,
      metaagentId: metaagent.id,
      historyDir: runtime.env.CHARIOX_SESSION_HISTORY_DIR,
      beforeAgentIds,
      options,
      providerMcpName,
      providerSkillName,
    })

    return {
      kernelUrl: runtime.kernelUrl,
      sessionId,
      metaagentId: metaagent.id,
      workerId: observed.worker.id,
      workerAlias: observed.worker.alias ?? null,
      taskStatus: observed.task.status,
      planLength: observed.task.plan_markdown?.length ?? 0,
      completionSummary: observed.task.completion_summary ?? null,
      commands: observed.commands,
      workerGrants: observed.worker.extension_grants ?? [],
      metaagentEventCount: observed.events.length,
    }
  } finally {
    if (client) await client.close().catch(() => {})
    await cleanupSession(runtime.kernelUrl, sessionId)
    await terminateChild(runtime.daemon)
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const rootDir = path.join(repoRoot, 'target', 'live-metaagent-provider-capability-import-drill', `${process.pid}-${Date.now()}`)
  const phaseA = path.join(rootDir, 'phase-a-provider-import-command')
  const phaseB = path.join(rootDir, 'phase-b-real-metaagent')
  const shellBin = path.join(repoRoot, 'apps/shell/dist/shell.js')
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(phaseA, { recursive: true })
    await mkdir(phaseB, { recursive: true })

    const providerMcpName = await findCodexMcpName()
    const providerSkillName = await findCodexSkillName()
    log('real-codex-capabilities-selected', { providerMcpName, providerSkillName })

    const kernelBinary = await buildKernel()
    const providerImport = await runProviderImportCommandDrill({
      rootDir: phaseA,
      kernelBinary,
      shellBin,
      providerMcpName,
      providerSkillName,
    })
    log('provider-import-command-drill-passed', {
      kernelUrl: providerImport.kernelUrl,
      dryRunCandidates: parseSummaryCount(providerImport.dryRunOutput, 'candidates'),
      imported: parseSummaryCount(providerImport.importOutput, 'imported'),
      alreadyInstalled: parseSummaryCount(providerImport.repeatOutput, 'already_installed'),
    })

    const metaagent = await runRealMetaagentDrill({
      rootDir: phaseB,
      kernelBinary,
      shellBin,
      options,
      providerMcpName,
      providerSkillName,
    })
    log('real-metaagent-drill-passed', {
      sessionId: metaagent.sessionId,
      metaagentId: metaagent.metaagentId,
      workerId: metaagent.workerId,
      taskStatus: metaagent.taskStatus,
      commandCount: metaagent.commands.length,
    })

    const report = {
      status: 'ok',
      mode: 'metaagent-provider-capability-import-drill',
      provider: options.provider,
      model: options.model,
      effort: options.effort,
      providerMcpName,
      providerSkillName,
      providerImport: {
        dryRunCandidates: parseSummaryCount(providerImport.dryRunOutput, 'candidates'),
        imported: parseSummaryCount(providerImport.importOutput, 'imported'),
        updated: parseSummaryCount(providerImport.importOutput, 'updated'),
        alreadyInstalled: parseSummaryCount(providerImport.repeatOutput, 'already_installed'),
      },
      metaagent,
      artifactRoot: rootDir,
    }
    await writeFile(path.join(rootDir, 'drill-report.json'), `${JSON.stringify(report, null, 2)}\n`, 'utf8')
    console.log(JSON.stringify(report, null, 2))
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      preserveOnSuccess: options.preserveOnSuccess,
      failure,
      metadata: {
        drill: 'metaagent-provider-capability-import',
        provider: options.provider,
        model: options.model,
        effort: options.effort,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      },
      log,
    })
  }
  log('passed')
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack ?? error.message : error)
  process.exit(1)
})
