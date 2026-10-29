// KR-06: a kernel serving ~10 working lanes plus production-shaped client traffic must stay responsive.
//
// Load shape (defaults match the lanes executor kernel measured on 2026-10-29, see the flags):
//   lanes          dev-stub provider runs whose prompts start and complete (run-state transitions)
//   leftover       live sessions nobody ended (the executor carried ~540 such sessions)
//   subscriptions  outside event parts seeded into the stopped kernel's database, sized like production
//   churn          session create/attach/shell/detach/end cycles per minute
//   readers        clients issuing small reads (status, session state, file read) at a steady rate
//   toggles        durable room mutations per second (stand-in for gateway audit and profile events)
// Probes run on their own client during an idle and a load phase; the verdict compares both.
import assert from 'node:assert/strict'
import { DatabaseSync } from 'node:sqlite'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { laneChecks, verdictOf } from './multi-agent-verdict.mjs'

const CLOCK_TICKS_PER_SECOND = 100

function threadTicks(pid, name) {
  for (const tid of readdirSync(`/proc/${pid}/task`)) {
    if (readFileSync(`/proc/${pid}/task/${tid}/comm`, 'utf8').trim() !== name.slice(0, 15)) continue
    const fields = readFileSync(`/proc/${pid}/task/${tid}/stat`, 'utf8').split(') ')[1].split(' ')
    return Number(fields[11]) + Number(fields[12])
  }
  throw new Error(`thread ${name} not found`)
}
function processTicks(pid) {
  const fields = readFileSync(`/proc/${pid}/stat`, 'utf8').split(') ')[1].split(' ')
  return Number(fields[11]) + Number(fields[12])
}
function rssKb(pid) { return Number(readFileSync(`/proc/${pid}/status`, 'utf8').match(/^VmRSS:\s+(\d+)/m)[1]) }
function size(path) {
  try {
    const stat = statSync(path)
    return stat.isDirectory() ? readdirSync(path).reduce((sum, name) => sum + size(join(path, name)), 0) : stat.size
  } catch { return 0 }
}

// Subscription parts are seeded into the stopped kernel's database because a local client cannot
// subscribe (outside principals arrive over the relay or a kernel gateway). The shapes copy the
// live executor: whole-session targets, a run state per followed agent, forwarded counters at the
// kernel's own cap. The path under test is what the kernel then does for every session update.
function seedSubscriptions(root, kernel, sizes, targets, runStates, padding) {
  assert(root.startsWith('/root/.chariox/dev/'))
  const db = new DatabaseSync(join(root, 'chariox/state.db'))
  try {
    const insert = db.prepare('INSERT OR REPLACE INTO agent_outside_subscriptions(id,payload) VALUES(?,?)')
    sizes.forEach((count, index) => {
      const forwarded = {}
      for (let i = 0; i < padding; i++) forwarded[JSON.stringify([`18e2${i.toString(16).padStart(12, '0')}`, `agent-${i.toString(16).padStart(32, '0')}`])] = i + 1
      const states = {}
      for (const [sessionId, agentId] of runStates.slice(0, count)) states[JSON.stringify([kernel, sessionId, agentId])] = 'idle'
      insert.run(`outside:kr06:grant-${index}`, JSON.stringify({
        id: `outside:kr06:grant-${index}`, home: false,
        targets: targets.slice(0, count).map(session_id => ({ kernel, session_id, agent_id: null })),
        ack: 0, floor: 0, latest: 0, ended: null, remote_cursors: {}, cleanup_targets: [],
        forwarded_counts: forwarded, run_states: states, kernel_health: {} }))
    })
    return db.prepare('SELECT count(*) AS n, sum(length(payload)) AS bytes FROM agent_outside_subscriptions').get()
  } finally { db.close() }
}
function countEvents(root) {
  const db = new DatabaseSync(join(root, 'chariox/state.db'), { readOnly: true })
  try { return db.prepare('SELECT count(*) AS n FROM durable_state_events').get().n } finally { db.close() }
}

export async function measureMultiAgentLoad({ rpc, connect, root, mcpUrl, pid, stop, start, report, save, distribution, logPath, options }) {
  const o = { lanes: 10, leftover: 500, subscriptions: [30, 24, 20, 2], padding: 1024, churnPerMin: 60, readers: 3, readerHz: 8,
    toggleHz: 3, promptIntervalMs: 3000, idleSeconds: 30, seconds: 120, ...options }
  const out = report.multi_agent_load = { options: o, lanes: [], phases: {} }
  save()
  const work = join(root, 'work')
  const mainClient = connect()
  const send = async request => { const result = await mainClient.send(request); if (result.Error) throw new Error(`${Object.keys(request)[0]}: ${result.Error.code}: ${result.Error.message}`); return result }
  // Only completed requests enter a latency distribution; failures are counted separately.
  const timed = (timings, kind) => async run => { const at = performance.now(); const result = await run(); (timings[kind] ??= []).push(performance.now() - at); return result }

  // Lanes: a session, an attachment and a dev-stub provider run per lane.
  for (let i = 0; i < o.lanes; i++) {
    const made = (await rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: `lane-${i}`,
      agent_defaults: { provider: 'dev-stub', model: 'runtime-mcp-fixture', account_profile: 'default' } } })).SessionCreated
    const lane = { session_id: made.session.id, agent_id: made.agent?.id }
    lane.attachment_id = (await rpc({ AttachToSession: { session_id: lane.session_id, client_id: `lane-${i}`, capability_level: 'FullTerminal' } })).SessionAttached.attachment.id
    try {
      const launched = await rpc({ LaunchProviderRun: { session_id: lane.session_id, agent_id: lane.agent_id, adapter_key: 'dev-stub', provider: 'dev-stub',
        account_profile: 'default', model: 'runtime-mcp-fixture', variant: null, native_tui: false } })
      lane.launch = Object.keys(launched)[0]
    } catch (error) { lane.launch_error = String(error.message) }
    out.lanes.push(lane)
  }
  // Leftover sessions: created, never ended, no provider.
  const leftover = []
  for (let i = 0; i < o.leftover; i++) leftover.push((await rpc({ CreateSession: { workspace_id: work, worktree_id: work, alias: `leftover-${i}` } })).SessionCreated.session.id)
  out.leftover_sessions = leftover.length
  const kernel = (await rpc({ RelayStatus: null })).RelayStatus.status.daemon_id
  if (o.subscriptions.length) {
    await mainClient.close()
    await stop()
    const targets = [...out.lanes.map(l => l.session_id), ...leftover]
    out.seeded_subscriptions = seedSubscriptions(root, kernel, o.subscriptions, targets, out.lanes.map(l => [l.session_id, l.agent_id]), o.padding)
    out.restart_ms = await start()
    // Attachments do not survive a restart; the lanes re-attach like a reconnecting client.
    for (const [i, lane] of out.lanes.entries()) {
      lane.attachment_id = (await rpc({ AttachToSession: { session_id: lane.session_id, client_id: `lane-${i}`, capability_level: 'FullTerminal' } })).SessionAttached.attachment.id
    }
  }
  const probe = connect()
  // The session the probes read: a lane when there is one, otherwise a leftover session.
  const probeSession = out.lanes[0]?.session_id ?? leftover[0]
  assert(probeSession, 'the drill needs at least one lane or leftover session')
  const path = join(work, 'kr06.bin')
  await send({ WriteKernelFile: { path, offset: 0, total_size: 1, data_base64: 'eA==', overwrite: true } }).catch(async () => {
    await probe.send({ WriteKernelFile: { path, offset: 0, total_size: 1, data_base64: 'eA==', overwrite: true } }) })
  const health = async client => (await client.send({ GetDaemonHealth: null })).DaemonHealth.projection
  const timingsOf = (projection, kinds) => Object.fromEntries((projection.request_timings ?? []).filter(t => kinds.includes(t.kind))
    .map(t => [t.kind, { samples: t.samples, handler_us: t.handler_us, durable_wait_us: t.durable_wait_us, fsync_us: t.fsync_us, queue_us: t.queue_us }]))

  async function phase(name, seconds, load) {
    const timings = {}
    const errors = {}
    const p = pid()
    const before = { at: performance.now(), writer: threadTicks(p, 'chariox-durable-writer'), process: processTicks(p), rss_kb: rssKb(p),
      log_bytes: size(logPath), db_bytes: size(join(root, 'chariox/state.db')) + size(join(root, 'chariox/state.db-wal')), events: countEvents(root) }
    let stopping = false
    const tasks = []
    const guarded = (kind, fn) => fn().catch(error => { errors[kind] = (errors[kind] ?? 0) + 1; errors[`${kind}_last`] = String(error.message).slice(0, 160) })
    // A rate of zero disables the generator.
    const paced = (hz, body) => (async () => { if (!(hz > 0)) return; const interval = 1000 / hz; for (let next = performance.now(); !stopping; next += interval) { await delay(Math.max(0, next - performance.now())); await body() } })()
    if (load) {
      // Session churn, as the lane tooling does it.
      const churn = connect(), c = timed(timings, 'churn_cycle')
      let churning = 0
      tasks.push(paced(o.churnPerMin / 60, () => { if (churning >= 16) return; churning++
        guarded('churn', () => c(async () => {
          const made = (await churn.send({ CreateSession: { workspace_id: work, worktree_id: work } })).SessionCreated
          const attached = (await churn.send({ AttachToSession: { session_id: made.session.id, client_id: 'churn', capability_level: 'FullTerminal' } })).SessionAttached
          await churn.send({ RunShellCommand: { session_id: made.session.id, attachment_id: attached.attachment.id, command: '/bin/true', args: [], timeout_ms: 1000 } })
          await churn.send({ DetachFromSession: { session_id: made.session.id, attachment_id: attached.attachment.id } })
          await churn.send({ EndSession: { session_id: made.session.id } })
        })).finally(() => churning--) }))
      // Readers: the relay request mix (status, session state, small file reads).
      for (let r = 0; r < o.readers; r++) {
        const client = connect()
        let n = 0
        tasks.push(paced(o.readerHz, () => { const i = n++ % 3
          return guarded('reader', () => i === 0 ? client.send({ RelayStatus: null })
            : i === 1 ? client.send({ GetSessionState: { session_id: out.lanes.length ? out.lanes[n % out.lanes.length].session_id : leftover[n % leftover.length] } })
            : client.send({ ReadKernelFile: { path, offset: 0, length: 1 } })) }))
      }
      // Coordinator-like list every two seconds.
      const lister = connect(), l = timed(timings, 'ListSessions_background')
      tasks.push(paced(0.5, () => guarded('list', () => l(() => lister.send({ ListSessions: {} })))))
      // Durable toggles on leftover sessions.
      const toggler = connect(), t = timed(timings, 'SetRoomPinned')
      let pins = 0
      tasks.push(paced(o.toggleHz, () => guarded('toggle', () => t(() => toggler.send({ SetRoomPinned: { session_id: leftover[pins % leftover.length], pinned: (pins++ & 1) === 0 } })))))
      // Lanes: prompts start and are cancelled, so each lane's run state keeps changing.
      for (const [i, lane] of out.lanes.entries()) {
        if (!lane.launch) continue
        const client = connect(), s = timed(timings, 'SubmitPrompt')
        // The kernel detaches terminal attachments that stop pumping; a lane client re-attaches like a TUI would.
        const laneSend = async build => {
          try {
            const result = await client.send(build(lane.attachment_id))
            if (result.Error) throw new Error(result.Error.message)
            return result
          } catch (error) {
            if (!String(error.message).includes('was not found')) throw error
            lane.attachment_id = (await client.send({ AttachToSession: { session_id: lane.session_id, client_id: `lane-${i}`, capability_level: 'FullTerminal' } })).SessionAttached.attachment.id
            lane.reattached = (lane.reattached ?? 0) + 1
            return client.send(build(lane.attachment_id))
          }
        }
        // Each prompt is one turn: the agent goes working, and back to idle when the turn ends.
        // The fixture never ends a turn itself, so the lane ends it halfway to the next prompt;
        // otherwise the prompt queue only grows and the drill measures that queue.
        const f = timed(timings, 'CompletePrompt')
        tasks.push(paced(1000 / o.promptIntervalMs, async () => {
          let submitted = false
          await guarded('prompt', () => s(() => laneSend(attachment_id => ({ SubmitPrompt: { session_id: lane.session_id, attachment_id, target_agent_id: lane.agent_id, prompt: 'kr06 load turn', attachments: [] } }))).then(() => { submitted = true }))
          await delay(o.promptIntervalMs / 2)
          // A prompt still being dispatched is ended by the next tick; that is not a load error.
          await guarded('complete', () => f(() => laneSend(() => ({ CompletePrompt: { session_id: lane.session_id } }))).then(() => {
            if (submitted) lane.prompt_cycles = (lane.prompt_cycles ?? 0) + 1
          }, error => {
            if (!String(error.message).includes('has no active prompt')) throw error
            out.complete_before_dispatch = (out.complete_before_dispatch ?? 0) + 1
          }))
        }))
      }
    }
    let n = 0
    const end = performance.now() + seconds * 1000
    while (performance.now() < end) {
      const next = performance.now() + 250
      await guarded('probe', () => timed(timings, 'RelayStatus')(() => probe.send({ RelayStatus: null })))
      await guarded('probe', () => timed(timings, 'GetSessionState')(() => probe.send({ GetSessionState: { session_id: probeSession } })))
      await guarded('probe', () => timed(timings, 'ListSessions')(() => probe.send({ ListSessions: {} })))
      if (n % 4 === 0) {
        await guarded('probe', () => timed(timings, 'GetDaemonHealth')(() => probe.send({ GetDaemonHealth: null })))
        await guarded('probe', () => timed(timings, 'McpInitialize')(async () => {
          const response = await fetch(mcpUrl, { method: 'POST', headers: { authorization: 'Bearer responsiveness-probe', 'content-type': 'application/json' },
            body: JSON.stringify({ jsonrpc: '2.0', id: n + 1, method: 'initialize', params: { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'kr06', version: '1' } } }) })
          if ((await response.json()).result?.serverInfo?.name !== 'chariox-runtime') throw new Error('MCP initialize failed')
        }))
      }
      n++
      await delay(Math.max(0, next - performance.now()))
    }
    stopping = true
    await Promise.allSettled(tasks)
    const elapsed = (performance.now() - before.at) / 1000
    const projection = await health(probe)
    const after = { writer: threadTicks(p, 'chariox-durable-writer'), process: processTicks(p), rss_kb: rssKb(p), log_bytes: size(logPath),
      db_bytes: size(join(root, 'chariox/state.db')) + size(join(root, 'chariox/state.db-wal')), events: countEvents(root) }
    out.phases[name] = { seconds: +elapsed.toFixed(1), errors,
      writer_cpu_percent: +(100 * (after.writer - before.writer) / CLOCK_TICKS_PER_SECOND / elapsed).toFixed(1),
      process_cpu_percent: +(100 * (after.process - before.process) / CLOCK_TICKS_PER_SECOND / elapsed).toFixed(1),
      rss_kb_start: before.rss_kb, rss_kb_end: after.rss_kb, rss_slope_mb_per_min: +((after.rss_kb - before.rss_kb) / 1024 / (elapsed / 60)).toFixed(2),
      log_bytes_per_s: Math.round((after.log_bytes - before.log_bytes) / elapsed), db_bytes_per_s: Math.round((after.db_bytes - before.db_bytes) / elapsed),
      durable_events_per_s: +((after.events - before.events) / elapsed).toFixed(2),
      durable_writer: projection.durable_writer,
      latency: Object.fromEntries(Object.entries(timings).filter(([, v]) => v.length).map(([kind, values]) => [kind, distribution(values)])),
      request_timings: timingsOf(projection, ['relay.status', 'session.state.get', 'session.list', 'session.create', 'session.end', 'session.attach', 'prompt.submit', 'prompt.cancel', 'room.pinned.set', 'machine_file.read']) }
    save()
  }

  await phase('idle', o.idleSeconds, false)
  await phase('load', o.seconds, true)
  const { idle, load } = out.phases
  const p99 = (ph, kind) => ph.latency[kind]?.p99_ms ?? 0
  const checks = {
    // 100 ms absorbs the probe client's own pauses on a loaded host (readers-only runs reached 89 ms on the base).
    plain_call_p99_under_load_ms: { value: p99(load, 'RelayStatus'), limit: Math.max(100, p99(idle, 'RelayStatus') + 50) },
    session_state_p99_under_load_ms: { value: p99(load, 'GetSessionState'), limit: 100 },
    list_sessions_p99_under_load_ms: { value: p99(load, 'ListSessions'), limit: 150 },
    mcp_initialize_p99_under_load_ms: { value: p99(load, 'McpInitialize'), limit: 100 },
    prompt_submit_p99_ms: { value: p99(load, 'SubmitPrompt'), limit: 500 },
    create_session_cycle_p99_ms: { value: p99(load, 'churn_cycle'), limit: 2000 },
    writer_cpu_percent_under_load: { value: load.writer_cpu_percent, limit: 40 },
    rss_slope_mb_per_min_under_load: { value: load.rss_slope_mb_per_min, limit: 5 },
    // The kernel reports its durable writer queue and commits in GetDaemonHealth.
    durable_writer_unreported: { value: load.durable_writer?.committed_batches > 0 ? 0 : 1, limit: 0 },
    load_errors: { value: Object.entries(load.errors).filter(([k]) => !k.endsWith('_last')).reduce((n, [, v]) => n + v, 0), limit: 0 },
    ...laneChecks(out.lanes),
  }
  out.checks = checks
  Object.assign(out, verdictOf(checks))
  save()
  for (const lane of out.lanes) await rpc({ EndSession: { session_id: lane.session_id } }).catch(() => { out.lane_cleanup_failed = true })
  await probe.close().catch(() => {})
  return out
}
