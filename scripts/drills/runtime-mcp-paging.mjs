// Exercise paging through a real private kernel and its dev-stub provider.
// Run on a dedicated test host with the shared kernel client built.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdir, mkdtemp, open, rm, writeFile } from 'node:fs/promises'
import { createServer } from 'node:net'
import { hostname } from 'node:os'
import { join } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { bindProviderMcpFixture } from '../../apps/cli/scripts/lib/provider-mcp-fixture.mjs'
import { LocalIpcClient } from '../../packages/kernel-client/dist/ipc.js'
import * as requests from '../../packages/kernel-client/dist/ipc-requests.js'

const [binary, expectedHost, phase, stateParent] = process.argv.slice(2)
assert(binary && hostname() === expectedHost && ['red', 'green'].includes(phase))
assert(stateParent?.startsWith('/root/.chariox/dev/'), 'private lane state required')
await mkdir(stateParent, { recursive: true, mode: 0o700 })
const root = await mkdtemp(join(stateParent, 'cell-'))
const ports = []
for (let i = 0; i < 5; i++) {
  const server = createServer()
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  ports.push(server.address().port)
  await new Promise(resolve => server.close(resolve))
}
const model = 'runtime-mcp-fixture'
const env = {
  PATH: process.env.PATH,
  HOME: join(root, 'home'),
  TMPDIR: join(root, 'tmp'),
  SHELL: '/bin/bash',
  IS_SANDBOX: '1',
  CHARIOX_PROVIDER_DEV_STUB: '1',
  CHARIOX_ROOM_AGENT_TOOLS: '1',
  CHARIOX_HOME: join(root, 'chariox'),
  CHARIOX_LOG_DIR: join(root, 'logs'),
  CHARIOX_SESSION_HISTORY_DIR: join(root, 'history'),
  CHARIOX_DAEMON_SOCKET: join(root, 'kernel.sock'),
  CHARIOX_DAEMON_ALIAS: 'private-mcp-paging',
  CHARIOX_KERNEL_PORT: String(ports[0]),
  CHARIOX_MCP_PORT: String(ports[1]),
  CHARIOX_OPENCODE_PORT: String(ports[2]),
  CHARIOX_CODEX_PORT: String(ports[3]),
  CHARIOX_KERNEL_BROWSER_PORT: String(ports[4]),
}
const workspace = join(root, 'work')
for (const path of [env.HOME, env.TMPDIR, env.CHARIOX_HOME, workspace]) {
  await mkdir(path, { mode: 0o700 })
}
await writeFile(join(env.CHARIOX_HOME, 'config.toml'), `
[credential_vault]
backend = "chariox_encrypted"
path = "${env.CHARIOX_HOME}/vault.json"
unlock_policy = "kernel_init"
[workflow]
session_default_max_agents = 64
[state]
path = "${env.CHARIOX_HOME}/state.db"
`, { mode: 0o600 })

const logPath = process.env.CHARIOX_PAGING_DRILL_LOG ?? join(root, 'kernel.log')
const log = await open(logPath, 'w', 0o600)
const kernel = spawn(binary, [], { env, stdio: ['ignore', log.fd, log.fd] })
const finished = new Promise((resolve, reject) => {
  kernel.once('exit', resolve)
  kernel.once('error', reject)
})
const client = new LocalIpcClient(`ws://127.0.0.1:${ports[0]}/kernel`, {
  localAuthEnvironment: env,
  controlRequestRetryDeadlineMs: 1000,
})
const rpc = async request => {
  const result = await client.send(request)
  assert(!result.Error, result.Error?.message)
  return result
}
async function until(probe, reason, ms = 30_000) {
  const end = Date.now() + ms
  while (Date.now() < end) {
    assert(kernel.exitCode === null && kernel.signalCode === null, 'private kernel exited')
    const result = await probe()
    if (result) return result
    await delay(200)
  }
  throw Error(`Timed out waiting for ${reason}`)
}

const wakes = 'chariox.events.wakes'
const roster = 'chariox.list_session_agents'
const inbox = 'chariox.events.inbox'
const ack = 'chariox.events.ack'

try {
  await until(() => rpc({ RelayStatus: null }).then(r => r.RelayStatus, () => null), 'kernel')
  const { session, agent } = (await rpc({
    CreateSession: {
      workspace_id: workspace, worktree_id: workspace, alias: 'mcp-paging',
      agent_defaults: { provider: 'dev-stub', model, account_profile: 'default', permission_level: 'yolo' },
    },
  })).SessionCreated
  const room = session.id
  const attachment = (await rpc({
    AttachToSession: { session_id: room, client_id: 'paging-owner', capability_level: 'FullTerminal' },
  })).SessionAttached.attachment.id
  const launch = await rpc(requests.launchProviderRunsRequest([{
    sessionId: room, provider: 'dev-stub', accountProfile: 'default', model, effort: 'low',
    agentId: agent.id, native: { nativeTui: true },
  }], 1))
  const id = launch.ProviderRunsLaunchAccepted.provider_runs[0].provider_run.id
  const run = await until(async () => {
    const result = Object.values(await rpc(requests.getProviderRunRequest(id)))[0]
    const r = result.provider_run ?? result
    if (r.state === 'Running') return r
  }, 'dev-stub provider')
  let subscription
  const { fixtureMcp } = bindProviderMcpFixture(run, {
    client: {
      send: rpc,
      subscribeToKernelEvents: (...args) => subscription ??= client.subscribeToKernelEvents(...args),
      onKernelEvent: client.onKernelEvent.bind(client),
    },
    requests, sessionId: room, attachmentId: attachment,
  })
  await rpc(requests.submitPromptRequest(room, attachment, agent.id, 'Keep this fixture turn open.', []))
  const task = await until(async () => {
    const state = (await rpc({ GetSessionState: { session_id: room } })).SessionState
    return (state.agent_tasks ?? state.session.agent_tasks).find(t => t.agent_id === agent.id && t.state === 'working')
  }, 'working fixture task')
  let origin = task.prompt_id

  async function invoke(name, arguments_) {
    for (let attempt = 0; attempt < 3; attempt++) {
      const authority = name.startsWith('chariox.events.')
        ? { task_id: task.task_id, origin_prompt_id: origin }
        : {}
      const response = await fixtureMcp.call('tools/call', {
        name, arguments: { ...authority, ...arguments_ },
      }, { rawEnvelope: true, timeoutMs: 40_000 })
      const renewed = response.error?.message?.match(/origin_prompt_id is `([^`]+)`/)
      if (renewed) {
        origin = renewed[1]
        continue
      }
      assert(!response.error && !response.result.isError, response.error?.message)
      const text = response.result.content.find(block => block.type === 'text').text
      return { page: JSON.parse(text), bytes: Buffer.byteLength(text), text }
    }
    throw Error('Fixture turn changed repeatedly')
  }

  async function readPage(name, args = {}) {
    const result = await invoke(name, args)
    assert(result.bytes < 64 * 1024 && !result.text.includes('Result exceeded the 64 KiB MCP text limit'))
    if (phase === 'green' && name === wakes) assert(result.text.startsWith('{"scheduler":'))
    if (phase === 'green' && name === roster) assert(result.text.startsWith('{"agents":'))
    return result.page
  }

  const timer = async (label, options) =>
    readPage('chariox.events.timer', { label, ...options })
  const small = await timer('Small-Room timer', { delay_ms: 120_000 })
  const smallWakes = await readPage(wakes)
  assert.equal(smallWakes.wakes.length, 1)
  assert.equal(smallWakes.wakes[0].registration_id, small.registration_id)
  assert.equal(smallWakes.wakes[0].label, 'Small-Room timer')
  assert.deepEqual(smallWakes.receipts, [])
  assert.equal(typeof smallWakes.scheduler.stalled, 'boolean')
  const smallRoom = await readPage(roster)
  assert.equal(smallRoom.agents.length, 1)
  assert.equal(smallRoom.agents[0].id, agent.id)
  assert.equal(smallRoom.agents[0].provider, 'dev-stub')
  assert.equal(smallRoom.agent_tasks[0].task_id, task.task_id)
  await readPage('chariox.events.cancel_wake', { wake_id: small.wake_id })
  console.log('M2 PASS: one agent, one active task, and one scheduled timer retain their fields.')

  const ids = [agent.id]
  for (let i = 1; i < 60; i++) {
    const { AgentSpawned } = await rpc(requests.spawnAgentRequest(
      room, 'dev-stub', `peer-${i}`, model, undefined, 'low', 'build', 'yolo',
      undefined, undefined, undefined, 'default',
    ))
    ids.push(AgentSpawned.agent.id)
  }

  // Leave a substantial settled history in the requesting agent's task.
  for (let i = 0; i < 140; i++) {
    const wake = await timer(`Cancelled timer ${i}`, { delay_ms: 600_000 })
    await readPage('chariox.events.cancel_wake', { wake_id: wake.wake_id })
  }

  let after = 0
  const expected = []
  async function collect() {
    const page = await readPage(inbox, { after, limit: 6 })
    const events = page.events
    for (const event of events) {
      after = event.sequence
      if (event.kind === 'timer_fired') expected.push(after)
    }
    if (events.length) await readPage(ack, { through: after, handled: true })
  }

  const recurring = []
  const firstDue = Date.now() + 60_000
  for (let i = 0; i < 32; i++) {
    const delay_ms = Math.max(1_000, firstDue - Date.now())
    recurring.push(await timer('🕰'.repeat(200), { delay_ms, interval_ms: 60_000 }))
  }
  const started = Date.now()
  for (let count = 32; count <= 480; count += 32) {
    await until(async () => {
      await collect()
      return expected.length >= count
    }, `fire receipt ${count}`, 150_000)
  }
  for (const wake of recurring.slice(20)) {
    await readPage('chariox.events.cancel_wake', { wake_id: wake.wake_id })
  }
  await until(async () => {
    await collect()
    return expected.length >= 500
  }, '500 retained fire receipts', 150_000)
  assert.equal(expected.length, 500)

  if (phase === 'red') {
    for (const name of [wakes, roster]) {
      const result = await invoke(name, {})
      const truncated = result.page.chariox_truncated === true
      if (name === wakes) {
        assert(truncated || result.page.receipts.length < 500 ||
          result.page.receipts[0].sequence !== expected.at(-1), name)
      } else {
        assert(truncated || result.page.agent_tasks.some(t => t.obligations.some(o => o.status === 'cancelled')), name)
      }
      console.log(`M1 RED: ${name} ${truncated ? 'exceeded the MCP text limit' : 'failed the ordering, completeness or history check'}.`)
    }
  } else {
    async function pages(name, field, cursorField) {
      let args = {}
      const rows = []
      let pageCount = 0
      for (;;) {
        const page = await readPage(name, args)
        if (name === wakes) assert.equal(typeof page.scheduler.stalled, 'boolean')
        if (name === roster && pageCount === 0) {
          assert(page.agent_tasks.every(t => t.obligations.every(o => o.status !== 'cancelled')))
        }
        rows.push(...page[field])
        assert(++pageCount <= 100, 'cursor advanced')
        const cursor = page[`next_${cursorField}_cursor`]
        if (cursor == null) return rows
        args = { [`${cursorField}_cursor`]: cursor }
      }
    }
    const receipts = await pages(wakes, 'receipts', 'receipts')
    assert.deepEqual(receipts.map(r => r.sequence), expected.reverse())
    const agents = await pages(roster, 'agents', 'agents')
    assert.equal(agents[0].alias, 'peer-59')
    assert.equal(agents.at(-1).id, agent.id)
    assert.deepEqual(agents.map(a => a.id).sort(), ids.sort())
    assert(agents.every(a => typeof a.state === 'string' && Number.isInteger(a.queued_prompt_count)))
    console.log(`M1 PASS: ${agents.length} agents and ${receipts.length} receipts, newest first to oldest, in ${Date.now() - started}ms.`)
  }
} finally {
  try {
    await client.close()
  } finally {
    if (kernel.pid > 1 && kernel.exitCode === null && kernel.signalCode === null) {
      kernel.kill('SIGTERM')
      await Promise.race([finished, delay(5_000)])
      if (kernel.exitCode === null && kernel.signalCode === null) kernel.kill('SIGKILL')
    }
    await finished
    await log.close()
    await rm(root, { recursive: true, force: true })
  }
}
