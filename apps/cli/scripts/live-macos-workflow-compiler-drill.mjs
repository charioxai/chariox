#!/usr/bin/env bun
// MP-08 / MP-10 / MP-11: real kernel and rendered TUI on macOS or Linux. Reports contain verdicts, never guard source,
// provider prompts, credential contents or raw tool responses.
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { createConnection, createServer } from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { LocalIpcClient } from '../../../packages/kernel-client/dist/ipc.js'
import * as requests from '../../../packages/kernel-client/dist/ipc-requests.js'

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const options = {}
for (let index = 2; index < process.argv.length; index += 2) options[process.argv[index].replace(/^--/, '')] = process.argv[index + 1]
for (const name of ['kernel', 'home', 'workspace-parent', 'evidence']) {
  if (!options[name] || !path.isAbsolute(options[name])) throw new Error(`--${name} must be an absolute path`)
}
if (!['darwin', 'linux'].includes(process.platform)) throw new Error('macOS or Linux is required')
options.home = path.resolve(options.home)
if (!options.home.startsWith(path.join(process.env.HOME, '.chariox/dev/'))) throw new Error('home must be below ~/.chariox/dev/')
process.env.CHARIOX_HOME = options.home
const report = { mp: ['MP-08', 'MP-10', 'MP-11'], node: process.env.NODE ?? null, platform: process.platform, architecture: process.arch, cases: [], status: 'running' }
let stage = 'setup', kernel, tui, client, automation, sessionId, workspace, socketPath, homeCreated = false, screen = ''
const step = (name) => { stage = name; console.log(`${name}: running`) }
const passed = (name, data = {}) => { report.cases.push({ name, status: 'passed', ...data }); console.log(`${name}: passed`) }
const requireValue = (value) => {
  if (!value) { report.failedAssertionStage = stage; throw new Error(stage) }
}
function stopOwned(child, signal) {
  if (!child || child.exitCode !== null) return
  if (!Number.isInteger(child.pid) || child.pid <= 1) throw new Error('invalid owned child PID')
  child.kill(signal)
}
function observeGuardResponse(value) {
  if (stage !== 'room-agent-compile-request-is-refused') return
  report.guardResponseCount = (report.guardResponseCount ?? 0) + 1
  report.guardResponseTypes ??= {}
  for (const name of Object.keys(value ?? {})) {
    report.guardResponseTypes[name] = (report.guardResponseTypes[name] ?? 0) + 1
  }
  report.guardStates ??= {}
  const visit = (item, depth = 0) => {
    if (depth > 16 || !item || typeof item !== 'object') return
    for (const [key, child] of Object.entries(item)) {
      if (['status', 'state'].includes(key) && typeof child === 'string' &&
          ['queued', 'running', 'completed', 'failed', 'cancelled', 'idle', 'starting', 'stopped'].includes(child.toLowerCase())) {
        report.guardStates[child.toLowerCase()] = (report.guardStates[child.toLowerCase()] ?? 0) + 1
      }
      if (child && typeof child === 'object') visit(child, depth + 1)
    }
  }
  visit(value)
}
function toolCall(text) {
  try { return JSON.parse(text.slice(text.indexOf('{'))) } catch { return null }
}
const isValidateCall = (call) => String(call?.tool ?? '').replace(/^mcp__chariox__/, '').replaceAll('.', '_') === 'chariox_workflow_code_validate'
// The room agent's compile request reached the workflow-code validator and the
// isolated compiler refused host access.
function refusedCompile(call) {
  return isValidateCall(call) && /not defined|script failed/.test(`${call.output ?? ''}\n${call.error ?? ''}`)
}
// The room agent's TypeScript source compiled inside the boundary and validated.
function validatedTypeScript(call) {
  return isValidateCall(call) && ['typescript', 'type_script'].includes(call.input?.language) &&
    call.status === 'completed' && !call.error && /"ok"\s*:\s*true/.test(call.output ?? '')
}
// Sanitized diagnostics only: lifecycle, tool identifiers and verdict flags.
async function awaitRoomToolCall(agentId, key, matches) {
  await until(async () => {
    const result = await client.send(requests.getSessionHistoryOutlineRequest(sessionId, [agentId], 5))
    const turns = result.SessionHistoryOutline?.agents?.flatMap((agent) => agent.turns) ?? []
    report[`${key}TurnLifecycles`] = turns.map((turn) => turn.lifecycle)
    report[`${key}ToolCalls`] = []
    for (const turn of turns) {
      for (const blob of turn.blobs.filter((blob) => blob.kind === 'provider_tool')) {
        const content = await client.send(requests.getSessionHistoryBlobContentRequest(sessionId, agentId, blob.blob_id))
        for (const { entry, fragment_end, total_chars } of content.SessionHistoryBlobContent?.entries ?? []) {
          const call = toolCall(entry.text)
          report[`${key}ToolCalls`].push({ tool: call?.tool ?? null, status: call?.status ?? null, language: call?.input?.language ?? null, truncated: fragment_end < total_chars, matched: matches(call) })
          if (matches(call)) return true
        }
      }
    }
    return false
  }, 180_000)
}
async function freePort() {
  const server = createServer()
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  const port = server.address().port
  await new Promise((resolve) => server.close(resolve))
  return port
}
async function until(check, timeout = 90_000) {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) { if (await check()) return; await Bun.sleep(250) }
  report.timedOutStage = stage
  throw new Error(stage)
}
async function connectAutomation(socketPath) {
  const socket = createConnection(socketPath)
  await new Promise((resolve, reject) => { socket.once('connect', resolve); socket.once('error', reject) })
  let sequence = 0, buffer = ''
  const pending = new Map()
  socket.on('data', (chunk) => {
    buffer += chunk.toString()
    while (buffer.includes('\n')) {
      const end = buffer.indexOf('\n'), frame = JSON.parse(buffer.slice(0, end)); buffer = buffer.slice(end + 1)
      const waiter = pending.get(frame.id); pending.delete(frame.id)
      if (frame.ok) waiter?.resolve(frame.data); else {
        const detail = JSON.stringify(frame.error ?? frame.message ?? '')
        report.automationCompilerStartupFailed = /Cannot load external builtin|node_builtins|SIGABRT|Aborted|134/i.test(detail)
        report.automationCompilerFailed = /compiler failed|workflow-code compiler/i.test(detail)
        waiter?.reject(new Error(stage))
      }
    }
  })
  return {
    send(action, fields = {}) {
      const id = ++sequence
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject })
        socket.write(JSON.stringify({ id, action, ...fields }) + '\n')
      })
    },
    close() { socket.destroy() },
  }
}
try {
  await mkdir(options.evidence, { recursive: true })
  await mkdir(path.dirname(options.home), { recursive: true, mode: 0o700 })
  await mkdir(options.home, { recursive: process.platform === 'darwin', mode: 0o700 })
  homeCreated = process.platform === 'linux'
  workspace = await mkdtemp(path.join(options['workspace-parent'], 'seatbelt-drill-'))
  socketPath = `/tmp/chariox-seatbelt-${process.pid}.sock`
  const port = await freePort(), mcpPort = await freePort(), codexPort = await freePort(), opencodePort = await freePort()
  const url = `ws://127.0.0.1:${port}`
  const env = { ...process.env, CHARIOX_HOME: options.home, CHARIOX_LOG_DIR: path.join(options.home, 'logs'), CHARIOX_KERNEL_PORT: String(port), CHARIOX_MCP_PORT: String(mcpPort), CHARIOX_CODEX_PORT: String(codexPort), CHARIOX_OPENCODE_PORT: String(opencodePort), CHARIOX_ROOM_AGENT_TOOLS: '1', CHARIOX_TEST_TUI: '1', CHARIOX_DAEMON_SOCKET: path.join(options.home, 'daemon.sock') }
  const ordinary = `workflow.define({alias: "seatbelt-ordinary", flushAgentContextBeforeRun: false});
const worker = workflow.node({handle: "worker", agent: workflow.existingAgent("ORDINARY_AGENT_REF"), canCompleteWorkflowRun: true, instructions: 'Return exactly a fenced JSON block with summary "completed" and output.message "seatbelt ordinary complete". Do not use tools.'});
workflow.endpoint(worker, {handle: "entry"});`
  await writeFile(path.join(workspace, 'ordinary.js'), ordinary)
  for (let index = 0; index < 32; index++) await writeFile(path.join(workspace, `schema-${index}.json`), '{"type":"object"}')
  await writeFile(path.join(workspace, 'schemas32.js'), `workflow.define({alias: "seatbelt-schemas-32"});
for(let index=0;index<32;index++) workflow.schemaFromFile({handle: 'schema_'+index,path: 'schema-'+index+'.json'});
const worker=workflow.node({handle:'worker',agent:workflow.newAgent({provider:'codex',model:'default'}),canCompleteWorkflowRun:true});
workflow.endpoint(worker,{handle:'entry'});`)
  step('built-kernel-ready')
  kernel = Bun.spawn([options.kernel], { cwd: repo, env, stdout: 'ignore', stderr: 'ignore' })
  report.kernelPid = kernel.pid
  client = new LocalIpcClient(url, {})
  const send = client.send.bind(client)
  client.send = async (...args) => {
    const response = await send(...args)
    observeGuardResponse(response)
    return response
  }
  await until(async () => { try { return Boolean((await client.send(requests.listSessionsRequest())).SessionsListed) } catch { return false } }, 30_000)
  passed(stage)
  step('native-codex-login-available')
  let imported
  try {
    imported = await client.send(options['provider-profile']
      ? requests.linkProviderAccountProfileRequest('codex', 'compiler-drill', options['provider-profile'])
      : requests.importNativeProviderAccountProfileRequest('codex'))
  } catch (error) {
    // A new kernel can register its bootstrap profile automatically. Reuse the
    // exact profile identified by the product's duplicate-scope response.
    const existing = String(error).match(/\(([A-Za-z0-9_-]+)\); use that profile instead/)
    if (!options['provider-profile'] || !existing) throw error
    imported = await client.send(requests.getProviderAccountProfileRequest('codex', existing[1]))
    report.reusedExistingProviderProfile = true
  }
  requireValue(imported.ProviderAccountProfile?.profile)
  const profileId = imported.ProviderAccountProfile.profile.profile_id
  const refreshed = await client.send(requests.refreshProviderAccountProfileRequest('codex', profileId))
  report.nativeAuthState = refreshed.ProviderAccountProfile.profile.auth_state
  requireValue(report.nativeAuthState.toLowerCase() === 'authenticated')
  await client.send(requests.setDefaultProviderAccountProfileRequest('codex', profileId))
  passed(stage)
  step('room-created')
  const created = await client.send(requests.createSessionRequest(workspace, workspace, 'seatbelt-live', {provider: 'codex', model: 'default'}))
  sessionId = created.SessionCreated.session.id
  report.sessionId = sessionId
  await writeFile(path.join(workspace, 'ordinary.js'), ordinary.replace('ORDINARY_AGENT_REF', created.SessionCreated.agent.id))
  passed(stage)
  step('rendered-tui-attached')
  tui = Bun.spawn([process.execPath, path.join(repo, 'apps/cli/dist/index.js'), '--kernel-url', url, '--session', sessionId, '--automation-socket', socketPath], {
    cwd: repo, env: {...env, TERM: 'xterm-256color'},
    terminal: {cols: 160, rows: 48, data(_terminal, chunk) { screen = (screen + new TextDecoder().decode(chunk)).slice(-100_000) }},
  })
  report.tuiPid = tui.pid
  await until(async () => {
    if (!screen.includes('Ctrl+T hotkeys')) return false
    try { automation = await connectAutomation(socketPath); return true } catch { return false }
  }, 30_000)
  const attached = await automation.send('snapshot')
  requireValue(attached.session?.id === sessionId && attached.attachmentId)
  passed(stage)
  step('owner-validates-ordinary-workflow-through-tui')
  const validated = await automation.send('workspace_shell_exec', {command: 'workflow code validate ordinary.js'})
  await Bun.sleep(250)
  await writeFile(path.join(options.evidence, 'ordinary-tui.ansi'), screen)
  requireValue(validated.result?.ok === true)
  passed(stage)
  step('owner-compiles-32-schemas-through-tui')
  const schemas = await automation.send('workspace_shell_exec', {command: 'workflow code validate schemas32.js'})
  await Bun.sleep(250)
  await writeFile(path.join(options.evidence, 'schemas-tui.ansi'), screen)
  requireValue(schemas.result?.ok === true)
  passed(stage, {schemaCount: 32})
  step('owner-runs-ordinary-workflow-through-tui')
  const run = await automation.send('workspace_shell_exec', {command: 'workflow code run ordinary.js --endpoint entry --prompt "Complete this workflow."'})
  report.runAccepted = run.result?.ok === true
  requireValue(report.runAccepted)
  await until(async () => {
    const state = await automation.send('snapshot')
    const workflow = state.workflows.find((flow) => flow.alias === 'seatbelt-ordinary')
    report.workflowStatuses = state.workflowRuns.map((entry) => entry.status)
    // Sanitized run-path diagnostics: statuses and kinds only.
    report.ordinaryQueuedPromptStatuses = workflow?.queuedPrompts.map((prompt) => prompt.status) ?? null
    report.ordinaryAgentState = state.session?.agents?.find((agent) => agent.id === created.SessionCreated.agent.id)?.state ?? null
    report.pendingInteractionKinds = state.interactions.map((interaction) => interaction.kind)
    const complete = state.workflowRuns.find((entry) => entry.workflowId === workflow?.id && entry.status === 'Completed')
    if (!complete) return false
    report.workflowCompleted = true
    report.finalOutputPresent = Boolean(complete.finalOutput)
    report.workflowId = workflow.id
    requireValue(complete.finalOutput?.message === 'seatbelt ordinary complete')
    return true
  }, 180_000)
  await Bun.sleep(250)
  await writeFile(path.join(options.evidence, 'completed-tui.ansi'), screen)
  passed(stage)
  step('room-agent-compile-request-is-refused')
  const spawned = await client.send(requests.spawnAgentRequest(sessionId, 'codex', 'seatbelt-room-agent', 'default', workspace))
  const agentId = spawned.AgentSpawned.agent.id
  const launched = await client.send(requests.launchProviderRunRequest(sessionId, 'codex', 'default', 'default', '', agentId))
  requireValue(launched.ProviderRunLaunched?.provider_run?.id || launched.ProviderRunLaunchAccepted?.provider_run?.id)
  // Generate the guard source locally. Never print or put it in the report.
  const source = String.fromCharCode(112,114,111,99,101,115,115,46,99,119,100,40,41,59)
  const prompt = `Call the Chariox workflow_code validate MCP tool exactly once using this inline source: ${source}\nDo not run any shell tools or read any files. The expected result is refusal. After the tool returns, reply only "compiler refused host access".`
  await client.send(requests.submitPromptRequest(sessionId, attached.attachmentId, agentId, prompt, []))
  await awaitRoomToolCall(agentId, 'guard', refusedCompile)
  report.roomAgentId = agentId
  passed(stage)
  step('room-agent-validates-typescript-workflow')
  // MP-08/MP-11: TypeScript (including transform-only syntax) compiles inside the
  // boundary on every Node build, through a real provider's tool call.
  const typescript = 'enum Lane { Primary = 1 }\ntype Provider = "codex";\nconst provider: Provider = "codex";\nworkflow.define({alias: "seatbelt-typescript", maxConcurrent: Lane.Primary});\nconst worker = workflow.node({handle: "worker", agent: workflow.newAgent({provider, model: "default"}), canCompleteWorkflowRun: true});\nworkflow.endpoint(worker, {handle: "entry"});'
  await client.send(requests.submitPromptRequest(sessionId, attached.attachmentId, agentId, `Call the Chariox workflow_code validate MCP tool exactly once with language "typescript" and this inline source:\n${typescript}\nDo not run any shell tools or read any files. After the tool returns, reply only "typescript validated".`, []))
  await awaitRoomToolCall(agentId, 'typescript', validatedTypeScript)
  passed(stage)
  report.status = 'passed'
} catch (error) {
  if (screen) {
    await Bun.sleep(250)
    await writeFile(path.join(options.evidence, 'failed-tui.ansi'), screen)
  }
  report.providerUnauthorized = /401|unauthorized/i.test(String(error))
  report.status = 'failed'; report.failedStage = stage
  console.error(`${stage}: failed`)
  process.exitCode = 1
} finally {
  step('cleanup-owned-processes')
  if (sessionId) await client?.send(requests.endSessionRequest(sessionId)).catch(() => {})
  await automation?.send('exit').catch(() => {})
  automation?.close()
  stopOwned(tui, 'SIGKILL')
  await client?.close().catch(() => {})
  stopOwned(kernel, 'SIGTERM')
  if (tui) { await tui.exited; tui.terminal?.close() }
  if (kernel) await kernel.exited
  if (socketPath) await rm(socketPath, {force: true})
  // Linux uses a new disposable home. Preserve macOS kernel homes under the
  // existing operator key-retention contract.
  if (workspace) await rm(workspace, {recursive: true, force: true})
  if (homeCreated) await rm(options.home, {recursive: true, force: true})
  report.cleanup = {kernelExited: true, tuiExited: true, fixtureWorkspaceRemoved: true, disposableKernelHomeRemoved: homeCreated, kernelHomeRetained: process.platform === 'darwin' ? options.home : null}
  await writeFile(path.join(options.evidence, 'macos-live-drill.json'), JSON.stringify(report, null, 2) + '\n')
  console.log(`drill: ${report.status}`)
}
