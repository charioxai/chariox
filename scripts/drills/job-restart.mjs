// Usage: node scripts/drills/job-restart.mjs <kernel-binary> <lane-state-dir> [crash]
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { mkdtemp, mkdir, readFile, rename, rm, stat, writeFile } from 'node:fs/promises'
import { createServer } from 'node:net'
import { fileURLToPath } from 'node:url'
import { join, resolve } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { LocalIpcClient } from '../../packages/kernel-client/dist/ipc.js'
import * as requests from '../../packages/kernel-client/dist/ipc-requests.js'

const [binaryArg, stateDir, mode = 'stop'] = process.argv.slice(2)
assert(binaryArg && stateDir && ['stop', 'crash'].includes(mode))
const binary = resolve(binaryArg)
const root = await mkdtemp(join(resolve(stateDir), 'jr-'))
const work = join(root, 'work')
const home = join(root, 'chariox')
const records = join(root, 'provider')
const codexBin = join(root, 'codex')
const fixture = fileURLToPath(new URL('../../apps/kernel/tests/hot_reload/fake_codex.py', import.meta.url))
await mkdir(records, { mode: 0o700 })
await writeFile(codexBin, `#!/bin/sh
exec /usr/bin/python3 ${JSON.stringify(fixture)} "$@"
`, { mode: 0o700 })
await mkdir(work, { mode: 0o700 })
await mkdir(home, { mode: 0o700 })
await writeFile(join(home, 'config.toml'), `version = 1
[history.archive]
mode = "disabled"
[credential_vault]
backend = "process_memory"
unlock_policy = "kernel_init"
agent_management = "allow"
`, { mode: 0o600 })

async function freePort() {
  const server = createServer()
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  const { port } = server.address()
  await new Promise(resolve => server.close(resolve))
  return String(port)
}

const [kernel, mcp, opencode, codex] = await Promise.all(Array.from({ length: 4 }, freePort))
const name = 'job-restart-' + root.split('/').at(-1).toLowerCase()
const env = {
  PATH: '/usr/bin:/bin', HOME: root, SHELL: '/bin/sh', LANG: 'C.UTF-8',
  CHARIOX_HOME: home, CHARIOX_DAEMON_SOCKET: join(root, 'kernel.sock'),
  CHARIOX_KERNEL_PORT: kernel, CHARIOX_MCP_PORT: mcp,
  CHARIOX_OPENCODE_PORT: opencode, CHARIOX_CODEX_PORT: codex,
  CHARIOX_DAEMON_ALIAS: name, CHARIOX_ROOM_AGENT_TOOLS: '1',
  CHARIOX_CODEX_BIN: codexBin, CHARIOX_FAKE_CODEX_RECORD: records,
  CODEX_HOME: join(root, 'codex-home'), CHARIOX_ALLOW_VOLATILE_PROCESS_MEMORY_VAULT: '1',
  IS_SANDBOX: '1',
}

async function until(read, label) {
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    const value = await read()
    if (value) return value
    await delay(200)
  }
  throw Error('Timed out waiting for ' + label)
}

let client, unit, pid
const unitPid = () => Number(execFileSync('systemctl', ['show', '-p', 'MainPID', '--value', unit]))
const rpc = async request => {
  const reply = await client.send(request, { timeoutMs: 2000 })
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
  client = new LocalIpcClient(`ws://127.0.0.1:${kernel}/kernel`, {
    localAuthEnvironment: env, controlRequestRetryDeadlineMs: 0,
  })
  await until(async () => {
    assert.equal(unitPid(), pid, 'the drill kernel is still running')
    try { return (await rpc({ RelayStatus: null })).RelayStatus } catch { return false }
  }, 'kernel startup')
}

async function stop(signal = 'SIGTERM') {
  await client?.close()
  if (!pid) return
  // We only signal the exact PID of the transient kernel we started.
  if (unitPid() === pid) process.kill(pid, signal)
  await until(() => unitPid() !== pid, 'kernel shutdown')
  pid = undefined
}

let room, agent
async function attach(round) {
  const attached = await rpc({ AttachToSession: {
    session_id: room, client_id: `${name}-${round}`, capability_level: 'FullTerminal',
  } })
  const attachment = attached.SessionAttached.attachment.id
  const submitted = await rpc(requests.submitPromptRequest(room, attachment, agent, `job restart drill ${round}`, []))
  const outcome = submitted.PromptSubmitted.outcome
  const prompt = outcome.prompt ?? Object.values(outcome)[0].prompt
  let task = await until(async () => {
    const { SessionState: state } = await rpc({ GetSessionState: { session_id: room } })
    return (state.agent_tasks ?? state.session.agent_tasks)
      .find(task => task.agent_id === agent && task.prompt_id === prompt.id && task.state === 'working')
  }, 'active drill turn')
  let sequence = 0
  async function invoke(name, args) {
    const file = join(records, `mcp-${round}-${++sequence}`)
    await writeFile(file + '.tmp', JSON.stringify({
      jsonrpc: '2.0', id: sequence, method: 'tools/call',
      params: { name, arguments: { ...args, task_id: task.task_id, origin_prompt_id: task.prompt_id } },
    }))
    await rename(file + '.tmp', file)
    const answer = await until(async () => {
      const { SessionState: state } = await rpc({ GetSessionState: { session_id: room } })
      for (const interaction of state.session.active_interactions ?? []) {
        if (interaction.agent_id !== agent || !interaction.id.startsWith('watched-process-')) continue
        assert(interaction.choices.some(choice => choice.id === 'allow_agent'))
        await rpc({ RespondToInteraction: {
          session_id: room, interaction_id: interaction.id, choice_id: 'allow_agent',
        } })
      }
      const saved = await readFile(file + '.out', 'utf8').catch(error => {
        if (error.code === 'ENOENT') return ''
        throw error
      })
      return saved && JSON.parse(saved)
    }, name)
    await rm(file)
    return answer
  }
  return async (name, args) => {
    let answer = await invoke(name, args)
    const turn = answer.error?.message?.match(/task_id is `([^`]+)` and its origin_prompt_id is `([^`]+)`/)
    if (turn) {
      task = { task_id: turn[1], prompt_id: turn[2] }
      answer = await invoke(name, args)
    }
    assert(!answer.error && !answer.fake_codex_error, JSON.stringify(answer))
    assert(!answer.result.isError, JSON.stringify(answer.result.content))
    return answer.result.structuredContent ?? JSON.parse(answer.result.content.find(entry => entry.type === 'text').text)
  }
}

try {
  await start(1)
  const { SessionCreated: made } = await rpc({ CreateSession: {
    workspace_id: work, worktree_id: work, alias: name,
  } })
  room = made.session.id
  const spawned = await rpc({ SpawnAgent: {
    session_id: room, alias: 'job-runner', provider: 'codex', model: 'gpt-6-luna',
    effort: 'low', permission_level: 'yolo', worktree_id: null, no_subscribe: true,
  } })
  agent = spawned.AgentSpawned.agent.id
  const launch = await attach(1)
  const launched = await launch('chariox.jobs.run', { wait_s: 1, command: ['/bin/sh', '-c', `
    printf 'ran\\n' >> runs
    printf 'Completed: prepare inputs\\n'
    printf 'Completed: validate inputs\\n'
    printf 'Started: process inputs\\n'
    printf 'partial output'
    touch ready
    while [ ! -f finish ]; do sleep 0.1; done
    printf 'Completed: process inputs\\n'
  `] })
  assert.equal(launched.placement, 'local')
  const args = { job_id: launched.job_id }
  const before = await launch('chariox.jobs.status', args)
  assert.equal(before.state, 'running')
  await until(() => stat(join(work, 'ready')).catch(() => false), 'first two job steps')
  const oldPid = pid
  await stop(mode === 'crash' ? 'SIGKILL' : 'SIGTERM')
  await start(2)
  assert.notEqual(pid, oldPid, 'successor is a different kernel process')
  const query = await attach(2)
  const status = await until(async () => {
    const value = await query('chariox.jobs.status', args)
    assert.equal(value.job_id, launched.job_id)
    return value.state === 'finished' && value
  }, 'terminal result after restart')
  assert.equal(status.registration_id, launched.registration_id)
  assert.equal(status.result.exit_code, null)
  assert.equal(status.result.outcome, 'machine_lost')
  const expected = ['Completed: prepare inputs', 'Completed: validate inputs', 'Started: process inputs']
  const log = status.result.log
  assert.equal(log, join(home, 'state', 'jobs', launched.job_id, 'output.log'))
  const logBytes = (await stat(log)).size
  assert(logBytes > 0, 'the output artifact is retained')
  assert.equal(await readFile(join(work, 'runs'), 'utf8'), 'ran\n')
  const output = status.result.output_tail
  console.log(JSON.stringify({ mode, oldPid, newPid: pid, status, logBytes }))
  assert.match(output, /kernel restart/)
  for (const line of expected) assert(output.includes(line), `result is missing: ${line}`)
  assert(!output.includes('Completed: process inputs'))
  assert.match(output, /Rerun only/)
  if (mode === 'stop') assert(output.includes('partial output'), 'orderly shutdown drains pending output')
} catch (error) {
  const kernelLog = await readFile(join(root, 'kernel.log'), 'utf8').catch(() => '')
  await writeFile(join(stateDir, `${name}.log`), kernelLog, { mode: 0o600 })
  throw error
} finally {
  await stop()
  await rm(root, { recursive: true, force: true })
}
