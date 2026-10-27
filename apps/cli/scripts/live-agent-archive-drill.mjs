#!/usr/bin/env node
// MP-08 / MP-09 / MP-10 / MP-11: real official-provider archive/restart/resume.
import assert from 'node:assert/strict'
import { spawn, execFileSync } from 'node:child_process'
import { createWriteStream, readFileSync } from 'node:fs'
import { mkdir, readFile, writeFile, rm } from 'node:fs/promises'
import net from 'node:net'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createHash } from 'node:crypto'
import * as api from '../../../packages/kernel-client/dist/index.js'

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..')
const opts = Object.fromEntries(process.argv.slice(2).map(value => {
  const [key, ...rest] = value.replace(/^--/, '').split('=')
  return [key, rest.length ? rest.join('=') : true]
}))
assert.equal(typeof opts.binary, 'string', '--binary=<matching built kernel> is required')
assert.equal(typeof opts.root, 'string', '--root=<lane-owned state directory outside repository> is required')
assert.equal(typeof opts.workspace, 'string', '--workspace=<lane-owned scratch repository> is required')
const provider = opts.provider ?? 'codex'
assert.ok(['codex', 'claude', 'opencode'].includes(provider))
const model = provider === 'codex' ? 'gpt-6-luna' : provider === 'claude' ? 'claude-haiku-5-5' : 'opencode-go/deepseek-v4.1-flash'
const runtimeProvider = provider === 'claude' ? 'claude-p' : provider
const marker = `ARCHIVE_${provider.toUpperCase()}_${Date.now()}`
const runRoot = path.join(path.resolve(opts.root), `${provider}-${Date.now()}`)
const stateRoot = path.join(runRoot, 'home')
await mkdir(stateRoot, { recursive: true, mode: 0o700 })
await mkdir(opts.workspace, { recursive: true })
execFileSync('git', ['init', '-q', opts.workspace])
await writeFile(path.join(opts.workspace, 'README.md'), '# Agent archive drill\nA disposable workspace for the real provider lifecycle.\n')
const results = { area: 'MP-08 / MP-09 / MP-10 / MP-11', provider, model, marker, checks: [], source: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: repo, encoding: 'utf8' }).trim(), binary_sha256: createHash('sha256').update(await readFile(opts.binary)).digest('hex') }
const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
const unwrap = (response, key) => { if (!response || typeof response !== 'object' || !(key in response)) { results.response_variant = Object.keys(response ?? {}); assert.fail(`expected ${key}`) }; return response[key] }
const check = (name, condition) => { assert.ok(condition, name); results.checks.push(name); console.log(JSON.stringify({ check: name, status: 'PASS' })) }
const port = async () => { const server = net.createServer(); await new Promise(resolve => server.listen(0, '127.0.0.1', resolve)); const result = server.address().port; await new Promise(resolve => server.close(resolve)); return result }
const kernelPort = await port()
const env = { ...process.env, CHARIOX_HOME: stateRoot, CHARIOX_KERNEL_HOST: '127.0.0.1', CHARIOX_KERNEL_PORT: String(kernelPort), CHARIOX_MCP_PORT: String(await port()), CHARIOX_CODEX_PORT: String(await port()), CHARIOX_OPENCODE_PORT: String(await port()), CHARIOX_DAEMON_SOCKET: path.join(runRoot, 'kernel.sock'), CHARIOX_DAEMON_ID: `archive-drill-${provider}-${Date.now()}`, CHARIOX_SESSION_HISTORY_DIR: path.join(stateRoot, 'history'), CHARIOX_KERNEL_RUNTIME_ROLE: 'ordinary' }
let child, client, room, agent, attachment, stage = 'bootstrap'
const children = []
const secret = opts['credential-stdin'] ? readFileSync(0, 'utf8').trim() : null
async function stop() {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  assert.ok(Number.isSafeInteger(child.pid) && child.pid > 1, 'refuse invalid child PID')
  child.kill('SIGTERM')
  await Promise.race([new Promise(resolve => child.once('exit', resolve)), pause(8000)])
  if (child.exitCode === null && child.signalCode === null) {
    assert.ok(Number.isSafeInteger(child.pid) && child.pid > 1, 'refuse invalid child PID')
    child.kill('SIGKILL')
    await new Promise(resolve => child.once('exit', resolve))
  }
}
async function start() {
  const log = createWriteStream(path.join(runRoot, `kernel-${children.length}.log`), { mode: 0o600 })
  child = spawn(opts.binary, [], { cwd: opts.workspace, env, stdio: ['ignore', 'pipe', 'pipe'] })
  children.push(child.pid)
  child.stdout.pipe(log, { end: false }); child.stderr.pipe(log, { end: false }); child.once('close', () => log.end())
  for (let attempt = 0; attempt < 120; attempt++) {
    if (child.exitCode !== null) throw new Error('kernel exited before readiness')
    client = new api.LocalIpcClient(`ws://127.0.0.1:${kernelPort}/kernel`, { localAuthEnvironment: env })
    try { await client.send(api.listSessionsRequest()); return } catch { await client.close().catch(() => {}); await pause(250) }
  }
  throw new Error('kernel readiness timeout')
}
async function snapshot() { return unwrap(await client.send(api.getSessionStateRequest(room)), 'SessionState').session }
const promptState = state => state.prompt_states?.[agent] ?? { active_prompt: state.active_prompt, queued_prompts: state.queued_prompts }
const thread = value => Object.values(value.provider_resume_state ?? {}).find(item => typeof item === 'string' && item.length) ?? value.provider_session_id ?? null
async function pump() { return JSON.stringify(await client.send(api.pumpTerminalOutputRequest(room, attachment))) }
async function until(name, predicate, timeout = 120000) {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) { if (await predicate()) return; await pause(150) }
  const state = room ? await snapshot().catch(() => null) : null
  results.last_state = state ? { status: state.status, active: !!promptState(state).active_prompt, queued: promptState(state).queued_prompts?.length, wakes: (state.agent_wakes ?? []).filter(value => value.agent_id === agent).map(value => ({ state: value.state, kind: value.kind })) } : null
  throw new Error(`${name} timeout`)
}
async function submit(text) { return client.send(api.submitPromptRequest(room, attachment, agent, text, [])) }
try {
  await start()
  stage = 'account'
  let profile
  if (provider === 'claude') {
    assert.ok(secret, 'Claude needs the documented setup token on stdin')
    profile = unwrap(await client.send(api.createProviderAccountProfileRequest('claude', 'archive-drill')), 'ProviderAccountProfile').profile.profile_id
    await client.send(api.setProviderAccountCredentialRequest('claude', profile, secret, true))
  } else {
    const source = provider === 'codex'
      ? '/root/.chariox/dev/worker-k/home/state/provider-accounts/local/codex/kaissandra-ai-reviewer-login-esqv7b7c/codex'
      : '/root/.chariox/dev/provider-runtimes/browser-computer/acct-686/home/state/provider-accounts/local/opencode/opencode-1-76jonxf5'
    profile = unwrap(await client.send(api.linkProviderAccountProfileRequest(provider, 'archive-drill', source)), 'ProviderAccountProfile').profile.profile_id
    await client.send(api.refreshProviderAccountProfileRequest(provider, profile))
  }
  stage = 'create'
  const created = unwrap(await client.send(api.createSessionRequest(opts.workspace, opts.workspace, `archive-${provider}`, { provider: runtimeProvider, model, effort: 'low', account_profile: profile, execution_mode: 'build', permission_level: provider === 'claude' ? 'required' : 'yolo' })), 'SessionCreated')
  room = created.session.id; agent = created.agent.id
  const other = unwrap(await client.send(api.spawnAgentRequest(room, runtimeProvider, 'retained-sibling', model, opts.workspace, 'low', 'build', provider === 'claude' ? 'required' : 'yolo', undefined, undefined, undefined, profile)), 'AgentSpawned').agent
  attachment = unwrap(await client.send(api.attachToSessionRequest(room, 'archive-drill')), 'SessionAttached').attachment.id
  stage = 'warm-thread'
  await submit(`Remember this marker for later: ${marker}. Reply only READY. Do not use tools.`)
  let observed = ''
  await until('warm provider turn', async () => { observed += await pump(); return observed.includes('READY') && !promptState(await snapshot()).active_prompt })
  let before = await snapshot()
  const original = before.agents.find(value => value.id === agent)
  const savedThread = thread(original)
  check('real provider thread established', !!savedThread)
  results.agent_id = agent
  results.provider_thread_sha256 = createHash('sha256').update(savedThread).digest('hex')
  stage = 'running-and-queued'
  await submit('Use chariox.events.timer once to arm a 10-minute timer labelled archive-drill. Then write a long numbered list of 500 short words. Keep the timer armed. This is an archive cancellation drill.')
  let timer = false
  await until('running turn and timer', async () => {
    await pump(); const state = await snapshot(); timer = (state.agent_wakes ?? []).some(wake => wake.agent_id === agent && ['scheduled', 'running'].includes(wake.state));
    return timer && !!promptState(state).active_prompt
  }, 90000)
  await submit('This queued prompt must be cancelled by archive. Reply QUEUED_SHOULD_NOT_RUN.')
  before = await snapshot()
  check('prompt queued behind active real turn', promptState(before).queued_prompts.length > 0)
  stage = 'archive'
  unwrap(await client.send(api.archiveAgentRequest(room, agent, true)), 'AgentArchived')
  let archived = await snapshot()
  check('target archived with identity and thread', archived.agents.some(value => value.id === agent && value.archived && thread(value) === savedThread))
  check('target task has a definite archive outcome', (archived.agent_tasks ?? []).some(value => value.agent_id === agent && value.state === 'cancelled' && value.reason.includes('Agent archived')))
  check('target active and queued work cleared', !promptState(archived).active_prompt && promptState(archived).queued_prompts.length === 0)
  check('target wakes settled', !(archived.agent_wakes ?? []).some(wake => wake.agent_id === agent && ['scheduled', 'starting', 'running', 'cancelling'].includes(wake.state)))
  check('sibling and Room retained', archived.status !== 'Ended' && archived.agents.some(value => value.id === other.id && !value.archived))
  const listed = unwrap(await client.send(api.listAgentsRequest(room)), 'AgentsListed').agents
  check('default list excludes target', !listed.some(value => value.id === agent))
  stage = 'restart'
  await client.close(); await stop(); await start()
  const archiveList = unwrap(await client.send(api.listArchivedAgentsRequest(room)), 'AgentsListed').agents
  check('archive replays across kernel restart', archiveList.some(value => value.id === agent && thread(value) === savedThread))
  stage = 'unarchive'
  unwrap(await client.send(api.archiveAgentRequest(room, agent, false)), 'AgentArchived')
  const restored = await snapshot()
  check('same agent restored idle without a turn', restored.agents.some(value => value.id === agent && !value.archived && !value.is_processing) && !promptState(restored).active_prompt)
  attachment = unwrap(await client.send(api.attachToSessionRequest(room, 'archive-drill-resumed')), 'SessionAttached').attachment.id
  await pump()
  stage = 'provider-resume'
  await submit('Reply only with the exact marker you were asked to remember earlier. Do not use tools.')
  observed = ''
  await until('resumed provider answer', async () => { observed += await pump(); return observed.includes(marker) && !promptState(await snapshot()).active_prompt })
  check('official provider resumes previous thread', thread((await snapshot()).agents.find(value => value.id === agent)) === savedThread)
  stage = 'legacy-end'
  unwrap(await client.send({ EndSession: { session_id: room } }), 'SessionEnded')
  attachment = unwrap(await client.send(api.attachToSessionRequest(room, 'legacy-reopen')), 'SessionAttached').attachment.id
  const legacy = await snapshot()
  check('legacy EndSession still creates a new default agent', legacy.agents.length === 1 && legacy.agents[0].id !== agent)
  unwrap(await client.send({ EndSession: { session_id: room } }), 'SessionEnded')
  results.status = 'PASS'
} catch (error) {
  results.status = 'FAIL'; results.stage = stage
  const message = String(error?.message ?? error)
  results.failure_class = ['429', 'unauthorized', 'quota', 'model', 'credential', 'timeout', 'assertion'].find(value => message.toLowerCase().includes(value)) ?? error?.code ?? error?.name ?? 'error'
  console.error(JSON.stringify({ stage, status: 'FAIL', failure_class: results.failure_class }))
  process.exitCode = 1
} finally {
  await client?.close().catch(() => {}); await stop()
  results.kernel_pids = children
  await writeFile(path.join(runRoot, 'result.json'), `${JSON.stringify(results, null, 2)}\n`, { mode: 0o600 })
  console.log(JSON.stringify({ result: path.join(runRoot, 'result.json'), status: results.status }))
  // State contains product-generated keys and account registrations. Keep only
  // the safe result and private kernel logs; never remove linked provider data.
  await rm(stateRoot, { recursive: true, force: true })
}
