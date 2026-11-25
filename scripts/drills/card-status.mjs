// Usage: node scripts/drills/card-status.mjs <kernel-binary> <lane-state-dir>
// A card only for what the person alone can give: a blocked task and a usage stop are status on the agent's row.
// L1 (CARD_DRILL_BLOCKED=1, needs a turn that owes work) a turn interrupted by a kernel crash leaves its task blocked: no card, the row names the way out, `agent resume cancel` ends it.
// L2 the same, ended by `agent resume`.
// L3 a usage stop with the default policy and no other account: armed, no card, the row says paused.
// L4 the same with the person's explicit "Ask me": asking, no card, nothing resumes by itself.
// Provider: apps/kernel/tests/hot_reload/fake_codex.py (a `limit` file in its record directory makes the turn fail on the usage limit).
import assert from 'node:assert/strict'
import { execFileSync, spawnSync } from 'node:child_process'
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { createServer } from 'node:net'
import { fileURLToPath } from 'node:url'
import { join, resolve } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { LocalIpcClient } from '../../packages/kernel-client/dist/ipc.js'
import * as requests from '../../packages/kernel-client/dist/ipc-requests.js'

const [binaryArg, stateDir] = process.argv.slice(2)
assert(binaryArg && stateDir, 'usage: card-status.mjs <kernel-binary> <lane-state-dir>')
const binary = resolve(binaryArg)
const root = await mkdtemp(join(resolve(stateDir), 'cs-'))
const work = join(root, 'work')
const home = join(root, 'chariox')
const records = join(root, 'provider')
const codexBin = join(root, 'codex')
const fixture = fileURLToPath(new URL('../../apps/kernel/tests/hot_reload/fake_codex.py', import.meta.url))
const cliVerbs = fileURLToPath(new URL('../../apps/cli/dist/agent-verbs.js', import.meta.url))
await mkdir(records, { mode: 0o700 })
await mkdir(work, { mode: 0o700 })
await mkdir(home, { mode: 0o700 })
await writeFile(codexBin, `#!/bin/sh\nexec /usr/bin/python3 ${JSON.stringify(fixture)} "$@"\n`, { mode: 0o700 })
const writeConfig = ask => writeFile(join(home, 'config.toml'), `version = 1
[history.archive]
mode = "disabled"
[credential_vault]
backend = "process_memory"
unlock_policy = "kernel_init"
agent_management = "allow"
${ask ? '[agents]\nusage_limit_resume = "ask"\n' : ''}`, { mode: 0o600 })

async function freePort() {
  const server = createServer()
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const { port } = server.address()
  await new Promise(resolve => server.close(resolve))
  return String(port)
}
const [kernel, mcp, opencode, codex] = await Promise.all(Array.from({ length: 4 }, freePort))
const name = 'card-status-' + root.split('/').at(-1).toLowerCase()
const env = {
  PATH: '/usr/bin:/bin', HOME: root, SHELL: '/bin/sh', LANG: 'C.UTF-8',
  CHARIOX_HOME: home, CHARIOX_DAEMON_SOCKET: join(root, 'kernel.sock'),
  CHARIOX_KERNEL_PORT: kernel, CHARIOX_MCP_PORT: mcp, CHARIOX_OPENCODE_PORT: opencode, CHARIOX_CODEX_PORT: codex,
  CHARIOX_DAEMON_ALIAS: name, CHARIOX_ROOM_AGENT_TOOLS: '1',
  CHARIOX_CODEX_BIN: codexBin, CHARIOX_FAKE_CODEX_RECORD: records,
  CODEX_HOME: join(root, 'codex-home'), CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT: '1', IS_SANDBOX: '1',
}
const results = []
const cell = (id, ok, details = {}) => { results.push({ id, ok, ...details }); console.log(id, ok ? 'PASS' : 'FAIL', JSON.stringify(details)) }
async function until(read, label, ms = 40_000) {
  const deadline = Date.now() + ms
  while (Date.now() < deadline) {
    const value = await read()
    if (value) return value
    await delay(250)
  }
  throw Error('Timed out waiting for ' + label)
}

let client, unit, pid
const unitPid = () => Number(execFileSync('systemctl', ['show', '-p', 'MainPID', '--value', unit]))
const rpc = async request => {
  const reply = await client.send(request, { timeoutMs: 4000 })
  assert(!reply.Error, JSON.stringify(reply.Error))
  return reply
}
async function start(round) {
  unit = `${name}-${round}.service`
  execFileSync('systemd-run', ['--quiet', '--collect', '--service-type=exec', `--unit=${unit}`,
    `--working-directory=${work}`, `--property=StandardOutput=append:${root}/kernel.log`,
    `--property=StandardError=append:${root}/kernel.log`, '/usr/bin/env', '-i',
    ...Object.entries(env).map(([key, value]) => `${key}=${value}`), binary])
  pid = unitPid()
  assert(pid > 1, 'the drill kernel started')
  client = new LocalIpcClient(`ws://127.0.0.1:${kernel}/kernel`, { localAuthEnvironment: env, controlRequestRetryDeadlineMs: 0 })
  await until(async () => {
    assert.equal(unitPid(), pid, 'the drill kernel is still running')
    try { return (await rpc({ RelayStatus: null })).RelayStatus } catch { return false }
  }, 'kernel startup')
}
async function stop(signal = 'SIGTERM') {
  await client?.close()
  if (!pid) return
  if (unitPid() === pid) process.kill(pid, signal)
  await until(() => unitPid() !== pid, 'kernel shutdown')
  pid = undefined
}
const state = async room => (await rpc({ GetSessionState: { session_id: room } })).SessionState
const cards = async room => ((await state(room)).session.active_interactions ?? [])
const tasks = async room => { const s = await state(room); return s.agent_tasks ?? s.session.agent_tasks ?? [] }
const agents = async room => (await state(room)).session.agents ?? []
// What the person runs: `chariox agent resume [cancel] AGENT` through the CLI's own verb entry.
function cli(...args) {
  const run = spawnSync('node', ['--input-type=module', '-e', `import { runAgentVerb } from ${JSON.stringify(cliVerbs)}; await runAgentVerb(process.argv.slice(1))`, ...args],
    { env: { ...env, PATH: process.env.PATH, CHARIOX_KERNEL_URL: `ws://127.0.0.1:${kernel}/kernel` }, encoding: 'utf8', timeout: 60_000 })
  return { status: run.status, out: run.stdout.trim(), err: run.stderr.trim() }
}
let room
async function newAgent(alias) {
  const spawned = await rpc({ SpawnAgent: { session_id: room, alias, provider: 'codex', model: 'gpt-6-luna', effort: 'low', permission_level: 'yolo', worktree_id: null, no_subscribe: true } })
  return spawned.AgentSpawned.agent.id
}
async function ask(agent, text) {
  const attached = await rpc({ AttachToSession: { session_id: room, client_id: `${name}-${agent}`, capability_level: 'FullTerminal' } })
  const submitted = await rpc(requests.submitPromptRequest(room, attached.SessionAttached.attachment.id, agent, text, []))
  const outcome = submitted.PromptSubmitted.outcome
  return outcome.prompt ?? Object.values(outcome)[0].prompt
}

try {
  await writeConfig(false)
  await start(1)
  room = (await rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: name } })).SessionCreated.session.id

  // L1/L2: an interrupted turn leaves its task blocked.
  // A plain turn is only recorded and never blocked: L1/L2 need a turn that owes work, so they run on request.
  for (const [id, verb] of process.env.CARD_DRILL_BLOCKED ? [['L1', 'cancel'], ['L2', 'resume']] : []) {
    const agent = await newAgent(`worker-${id.toLowerCase()}`)
    const prompt = await ask(agent, `${id}: work until the kernel dies`)
    await until(async () => (await tasks(room)).find(task => task.agent_id === agent && task.prompt_id === prompt.id && task.state === 'working'), `${id} working turn`)
    await stop('SIGKILL')
    await start(Number(id.slice(1)) + 1)
    const blocked = await until(async () => (await tasks(room)).find(task => task.agent_id === agent && task.state === 'blocked'), `${id} blocked task`, 220_000).catch(async error => { console.error(JSON.stringify({ tasks: await tasks(room), agent, session: (({ prompt_queue, active_prompts, queued_prompts, agents }) => ({ prompt_queue, active_prompts, queued_prompts, agents: agents?.map(a => ({ id: a.id, state: a.state })) }))((await state(room)).session) })); throw error })
    const open = await cards(room)
    const noCard = open.every(card => !card.id.startsWith('task-blocked-') && card.title !== 'Agent needs your action')
    const unblock = blocked.unblock
    cell(`${id}-row`, noCard && !!unblock?.resume_command && !!unblock?.cancel_command, { reason: blocked.reason, unblock, cards: open.map(card => card.id) })
    const ran = cli('agent', 'resume', ...(verb === 'cancel' ? ['cancel'] : []), `worker-${id.toLowerCase()}`, '--session', room)
    const after = await until(async () => { const task = (await tasks(room)).find(item => item.task_id === blocked.task_id); return task && task.state !== 'blocked' && task }, `${id} settled task`)
    cell(`${id}-${verb}`, ran.status === 0 && (verb === 'cancel' ? after.state === 'cancelled' : after.state !== 'cancelled'), { cli: ran, state: after.state })
  }

  // L3: a usage stop, default policy, no other account.
  await writeFile(join(records, 'limit'), '1')
  const limited = await newAgent('limited')
  await ask(limited, 'L3: this turn hits the usage limit')
  const stopped = await until(async () => (await agents(room)).find(agent => agent.id === limited && agent.usage_limit_stop), 'L3 usage stop')
  const l3cards = await cards(room)
  cell('L3-no-card', l3cards.length === 0 && stopped.usage_limit_stop.state.state === 'armed', { stop: stopped.usage_limit_stop, cards: l3cards.map(card => card.id) })
  const took = cli('agent', 'resume', 'cancel', 'limited', '--session', room)
  const declined = await until(async () => (await agents(room)).find(agent => agent.id === limited && agent.usage_limit_stop?.state?.state === 'declined'), 'L3 declined')
  cell('L3-cancel', took.status === 0 && !!declined, { cli: took })
  const back = cli('agent', 'resume', 'limited', '--session', room)
  const armed = await until(async () => (await agents(room)).find(agent => agent.id === limited && agent.usage_limit_stop?.state?.state === 'armed'), 'L3 armed again')
  cell('L3-resume', back.status === 0 && !!armed && (await cards(room)).length === 0, { cli: back })

  // L4: the person's explicit Ask me: nothing before the reset.
  await stop()
  await writeConfig(true)
  await start(9)
  const asked = await newAgent('asked')
  await ask(asked, 'L4: this turn hits the usage limit')
  const asking = await until(async () => (await agents(room)).find(agent => agent.id === asked && agent.usage_limit_stop), 'L4 usage stop')
  const l4cards = (await cards(room)).filter(card => card.related_agent_id === asked)
  cell('L4-ask-me', asking.usage_limit_stop.state.state === 'asking' && l4cards.length === 0, { stop: asking.usage_limit_stop, cards: l4cards.map(card => card.id) })
  const bad = results.filter(result => !result.ok)
  console.log(JSON.stringify({ cells: results.length, failed: bad.length }))
  if (bad.length) process.exitCode = 1
} catch (error) {
  const kernelLog = await readFile(join(root, 'kernel.log'), 'utf8').catch(() => '')
  await writeFile(join(resolve(stateDir), `${name}.log`), kernelLog, { mode: 0o600 })
  console.error(error)
  process.exitCode = 1
} finally {
  await stop().catch(() => {})
  await rm(root, { recursive: true, force: true })
}
