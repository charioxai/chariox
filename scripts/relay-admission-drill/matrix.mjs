// G09/G10 relay admission matrix: a real admitted peer sends one mismatched or outdated request at a time to the real home and
// worker kernels. Usage: node matrix.mjs <phase> [client-tag]; H_PID, W_PID, X_PID are the home, leased-worker and third enrolled
// kernel processes; ADMISSION_DRILL_TEST_BIN is the kernel test executable; ADMISSION_DRILL_REPLACE_RUN is a JSON argv that kills
// the worker's provider process (g10run); ADMISSION_DRILL_IDS is {home,worker,x:{kernel,machine}} (prep).
//   prep      : ids.json from the live sessions
//   g09       : needs an active leased run on agent A (started here)
//   g10run    : stale run deliveries while a successor run is active
//   g10lease  : stale lease deliveries after the agent is moved off and back onto the worker
//   g10done   : stale/duplicate completion on agent B
import assert from 'node:assert/strict'
import { readFileSync, writeFileSync, existsSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { client, requests, unwrap, senderCopy, Peer, log, sleep, EVID, STATE } from './lib.mjs'
const phase = process.argv[2]
const PID = { home: Number(process.env.H_PID), worker: Number(process.env.W_PID), x: Number(process.env.X_PID) }
const idsFile = `${STATE}/ids.json`
const H = client(PID.home), W = client(PID.worker)
const BIN = process.env.ADMISSION_DRILL_TEST_BIN
const longPrompt = (word, secs) => `Run exactly this shell command and wait for it to finish: sleep ${secs}. Then reply with exactly the single word ${word} and nothing else.`

const listAgents = async sid => unwrap(await H.send(requests.listAgentsRequest(sid)), 'AgentsListed').agents
const listSessions = async () => unwrap(await H.send(requests.listSessionsRequest()), 'SessionsListed').sessions
async function attach(sid) { return unwrap(await H.send(requests.attachToSessionRequest(sid, 'amh-driver')), 'SessionAttached').attachment.id }
async function submit(sid, agentId, prompt) {
  const att = await attach(sid)
  const r = await H.send(requests.submitPromptRequest(sid, att, agentId, prompt, []))
  assert.ok(!r.Error, JSON.stringify(r).slice(0, 300)); return r
}
async function waitFor(fn, what, ms = 120000) { const t0 = Date.now(); for (;;) { const v = await fn(); if (v) return v; assert.ok(Date.now() - t0 < ms, 'timeout: ' + what); await sleep(1500) } }
const projectAgent = a => ({ id: a.id, state: a.state, is_processing: a.is_processing, remote_execution: a.remote_execution && { ...a.remote_execution, active_worker_provider_run_id: undefined } })
async function snapshot() {
  const out = {}
  for (const s of await listSessions()) out[s.id] = { active_prompt: s.active_prompt?.id ?? null, agents: s.agents.map(projectAgent) }
  const wh = unwrap(await W.send(requests.getDaemonHealthRequest()), 'DaemonHealth').projection
  out.worker = { remote_execution: wh.remote_execution, invariants: wh.projection_invariants }
  return out
}
const same = (a, b) => JSON.stringify(a) === JSON.stringify(b)
const diffSnap = (a, b) => Object.keys({ ...a, ...b }).filter(k => !same(a[k], b[k])).map(k => ({ key: k, before: a[k], after: b[k] }))

if (phase === 'prep') {
  const sessions = await listSessions()
  const ids = { ...JSON.parse(process.env.ADMISSION_DRILL_IDS), sessions: {} }
  for (const s of sessions) { const a = s.agents[0]; ids.sessions[s.id] = { agent: a.id, remote: a.remote_execution } }
  writeFileSync(idsFile, JSON.stringify(ids, null, 1)); console.log(JSON.stringify(ids, null, 1)); process.exit(0)
}

const ids = JSON.parse(readFileSync(idsFile, 'utf8'))
const [S1, S2] = Object.keys(ids.sessions) // S1 = agent A, S2 = agent B
const A = ids.sessions[S1], B = ids.sessions[S2]
const senders = ['home', 'worker', 'x'].map(n => senderCopy(n, PID[n]))
const peer = new Peer(BIN, senders)
await peer.start()
const results = []
let seq = 0
async function sendCase(row, key, sender, target, request, expect, note = '') {
  const t = ids[target].kernel
  const r = await peer.send(sender, t, request)
  const o = r.outcome
  const refused = o.kind === 'refused'
  const rec = { tag: process.argv[3] ?? 'ipc', row, key, case: `${++seq}`, sender, target, request_kind: request.kind, expect, outcome_kind: o.kind, code: o.code, message: (o.message ?? '').slice(0, 220), response_kind: o.response?.kind, note }
  // an authorization refusal names the admission stage; an "accepted" baseline may still fail later in the tool layer
  const admission = /unauthorized|authorize forwarded|does not own|does not match|not belong|wrong|claims|absent|not found|stale|superseded|no longer current|not active|did not match/i.test(`${o.code ?? ''} ${o.message ?? ''}`)
  rec.admission_refusal = refused && admission
  rec.verdict = expect === 'refused' ? (refused || o.kind === 'local_error' ? 'ok-refused' : 'ACCEPTED-P1?') : (refused && admission ? 'unexpected-refusal' : 'ok-admitted')
  rec.response = o.response; results.push(rec); log('cases.jsonl', rec); console.log(rec.verdict.padEnd(18), `${row} ${sender}->${target}`.padEnd(26), request.kind.padEnd(34), o.kind, o.code ?? o.response?.kind ?? '', (o.message ?? '').slice(0, 90))
  return rec
}
const ctxOf = (sess, agentRec, run, over = {}) => ({ home_kernel_id: ids.home.kernel, home_session_id: sess, home_agent_id: agentRec.agent, leased_agent_id: agentRec.remote.leased_agent_id, worker_provider_run_id: run, worker_kernel_id: ids.worker.kernel, worker_machine_id: ids.worker.machine, ...over })
const check = ctx => ({ kind: 'check_leased_computer_grant', context: ctx, epoch: 'amh-epoch' })
const credTool = ctx => ({ kind: 'invoke_home_credential_tool', context: ctx, tool_name: 'amh_admission_probe', arguments: {} })
const peerStop = async (code = 0) => { peer.stop(); process.exit(code) }

try {
  if (phase === 'g09') {
    const before = await snapshot()
    // 1. start a long run on A so the worker->home forwarded requests have a live run to be checked against
    const word = process.env.G09_WORD ?? 'g09-done'
    if (!process.argv[3]) await submit(S1, A.agent, longPrompt(word, 100)) // with a tag, a real client (TUI/web) starts the turn
    const run = await waitFor(async () => (await listAgents(S1)).find(a => a.id === A.agent)?.remote_execution?.active_worker_provider_run_id, 'A active worker run', 150000)
    const sess = (await listSessions()).find(s => s.id === S1)
    console.log('active run', run, 'prompt', sess.active_prompt?.id)
    writeFileSync(`${STATE}/g09-active.json`, JSON.stringify({ run, prompt: sess.active_prompt?.id }))
    const good = ctxOf(S1, A, run)
    // 2. matching request still works (valid sender, valid binding)
    await sendCase('G09', 'valid', 'worker', 'home', check(good), 'accepted', 'worker->home grant check with the live run')
    await sendCase('G09', 'valid', 'worker', 'home', credTool(good), 'accepted', 'reaches the tool layer: authorization passed (a tool error is expected)')
    await sendCase('G09', 'valid', 'home', 'worker', { kind: 'get_leased_prompt_receipt', leased_agent_id: A.remote.leased_agent_id, home_prompt_id: sess.active_prompt?.id ?? 'none' }, 'accepted', 'home->worker receipt query for the live prompt')
    // 3. one mismatched field per request, sender = the real worker identity
    const faults = {
      'sender=x (other admitted kernel)': ['x', good],
      'room=other session (S2)': ['worker', { ...good, home_session_id: S2 }],
      'room=nonexistent session': ['worker', { ...good, home_session_id: 'session-does-not-exist' }],
      'home_kernel=x': ['worker', { ...good, home_kernel_id: ids.x.kernel }],
      'worker_kernel=x': ['worker', { ...good, worker_kernel_id: ids.x.kernel }],
      'worker_machine=x': ['worker', { ...good, worker_machine_id: ids.x.machine }],
      'worker_machine=home': ['worker', { ...good, worker_machine_id: ids.home.machine }],
      'lease=B (other lease)': ['worker', { ...good, leased_agent_id: B.remote.leased_agent_id }],
      'lease=bogus': ['worker', { ...good, leased_agent_id: 'leased-agent-0000deadbeef' }],
      'home_agent=B (other agent)': ['worker', { ...good, home_agent_id: B.agent }],
      'run=bogus': ['worker', { ...good, worker_provider_run_id: 'run-0000deadbeef' }],
    }
    for (const [key, [sender, ctx]] of Object.entries(faults)) {
      await sendCase('G09', key, sender, 'home', check(ctx), 'refused')
      await sendCase('G09', key, sender, 'home', credTool(ctx), 'refused')
    }
    // 4. home->worker: sender = x (other kernel) against the worker's lease operations; claim mismatches with the real home
    for (const op of [
      { kind: 'get_leased_prompt_receipt', leased_agent_id: A.remote.leased_agent_id, home_prompt_id: 'p' },
      { kind: 'cancel_leased_prompt', leased_agent_id: A.remote.leased_agent_id, home_prompt_id: sess.active_prompt?.id ?? 'p', worker_provider_run_id: run },
      { kind: 'complete_leased_prompt', leased_agent_id: A.remote.leased_agent_id },
      { kind: 'destroy_leased_agent', leased_agent_id: A.remote.leased_agent_id },
      { kind: 'destroy_leased_agent', leased_agent_id: B.remote.leased_agent_id },
      { kind: 'destroy_execution_lease', lease_id: A.remote.execution_lease_id },
    ]) await sendCase('G09', 'sender=x->worker', 'x', 'worker', op, 'refused')
    await sendCase('G09', 'claims.kernel=x', 'home', 'worker', { kind: 'create_execution_lease', home_kernel_id: ids.x.kernel, home_session_id: S1, home_agent_id: A.agent, owner_user_id: 'u' }, 'refused', 'lease claims name another home kernel')
    await sendCase('G09', 'claims.kernel=x', 'x', 'worker', { kind: 'create_execution_lease', home_kernel_id: ids.home.kernel, home_session_id: S1, home_agent_id: A.agent, owner_user_id: 'u' }, 'refused', 'sender x claims to be the home')
    await sendCase('G09', 'lease=bogus', 'home', 'worker', { kind: 'get_leased_prompt_receipt', leased_agent_id: 'leased-agent-0000deadbeef', home_prompt_id: 'p' }, 'refused')
    await sendCase('G09', 'lease=bogus', 'home', 'worker', { kind: 'destroy_execution_lease', lease_id: 'lease-0000deadbeef' }, 'refused')
    await sendCase('G09', 'wrong target kernel', 'home', 'x', { kind: 'get_leased_prompt_receipt', leased_agent_id: A.remote.leased_agent_id, home_prompt_id: 'p' }, 'refused', 'lease request delivered to a kernel that is not the worker')
    await sendCase('G09', 'wrong target kernel', 'worker', 'x', check(good), 'refused', 'worker->home request delivered to another kernel')
    // 5. unchanged-state + current run keeps working
    await waitFor(async () => { const s = (await listSessions()).find(x => x.id === S1); return !s.active_prompt }, 'A run to finish', 240000)
    const after = await snapshot()
    const text = JSON.stringify(unwrap(await H.send(requests.getSessionHistoryOutlineRequest(S1, null, 4)), 'SessionHistoryOutline')).includes(process.env.G09_WORD ?? 'g09-done')
    writeFileSync(`${EVID}/g09-snapshots.json`, JSON.stringify({ before, after, diff: diffSnap(before, after) }, null, 1))
    const summary = { before_equals_after: same(before, after), differing_keys: diffSnap(before, after).map(d => d.key), S2_unchanged: same(before[S2], after[S2]), final_answer_seen: text, active: { run } }
    log('g09-summary.jsonl', { summary, verdicts: results.map(r => r.verdict) })
    console.log(JSON.stringify(summary)); console.log('counts', JSON.stringify(results.reduce((m, r) => (m[r.verdict] = (m[r.verdict] ?? 0) + 1, m), {})))
  }

  if (phase === 'g10run') {
    const old = JSON.parse(readFileSync(`${STATE}/g09-active.json`, 'utf8')) // earlier completed run/prompt of agent A
    const before = await snapshot()
    // provider run ids are per provider process on the worker, so a run is only replaced by relaunching the provider: kill the worker's provider process (exact descendants, rig fault) and let the next prompt relaunch it
    const [replaceCmd, ...replaceArgs] = JSON.parse(process.env.ADMISSION_DRILL_REPLACE_RUN)
    const kill = spawnSync(replaceCmd, replaceArgs, { encoding: 'utf8' })
    log('cases.jsonl', { row: 'G10', step: 'replace-run', killProvider: kill.stdout.trim().slice(0, 200), err: kill.stderr.trim().slice(0, 200) })
    console.log('killProvider ->', kill.stdout.trim().slice(0, 120)); await sleep(4000)
    await submit(S1, A.agent, longPrompt('g10-done', 100))
    const run2 = await waitFor(async () => { const r = (await listAgents(S1)).find(a => a.id === A.agent)?.remote_execution?.active_worker_provider_run_id; return r && r !== old.run ? r : null }, 'successor run', 150000)
    const p2 = (await listSessions()).find(s => s.id === S1).active_prompt?.id
    console.log('old', JSON.stringify(old), 'successor run', run2, 'prompt', p2)
    writeFileSync(`${STATE}/g10-active.json`, JSON.stringify({ old, run2, p2 }))
    const cur = ctxOf(S1, A, run2), stale = ctxOf(S1, A, old.run)
    await sendCase('G10', 'valid-successor', 'worker', 'home', check(cur), 'accepted', 'successor run holds tool authority')
    await sendCase('G10', 'valid-successor', 'worker', 'home', credTool(cur), 'accepted', 'authorization passes for the successor run')
    await sendCase('G10', 'stale-run-tool', 'worker', 'home', check(stale), 'refused', 'earlier run id after replacement')
    await sendCase('G10', 'stale-run-tool', 'worker', 'home', credTool(stale), 'refused', 'earlier run id after replacement')
    const L = A.remote.leased_agent_id
    const ops = [
      { kind: 'cancel_leased_prompt', leased_agent_id: L, home_prompt_id: old.prompt, worker_provider_run_id: old.run },
      { kind: 'cancel_leased_prompt', leased_agent_id: L, home_prompt_id: p2, worker_provider_run_id: old.run },
      { kind: 'steer_leased_prompt', leased_agent_id: L, steer_id: 'amh-stale-steer', target_home_prompt_id: old.prompt, prompt: 'stale steer must not reach the provider' },
      { kind: 'reconcile_leased_prompt_steer_receipt', leased_agent_id: L, steer_id: 'amh-stale-steer', target_home_prompt_id: old.prompt, worker_provider_run_id: old.run, execution_lease_id: A.remote.execution_lease_id },
      { kind: 'drain_leased_runtime_projection', leased_agent_id: L, provider_run_id: old.run, pump_output: false, acknowledged_projection_sequence: 0 },
      { kind: 'observe_leased_git_after', leased_agent_id: L, provider_run_id: old.run },
    ]
    for (const op of ops) await sendCase('G10', 'stale-run-lease-op', 'home', 'worker', op, 'refused', 'earlier prompt/run after the successor started')
    await sendCase('G10', 'stale-run-lease-op', 'home', 'worker', { kind: 'get_leased_prompt_receipt', leased_agent_id: L, home_prompt_id: old.prompt }, 'accepted', 'read-only receipt of the completed earlier prompt (informational)')
    await waitFor(async () => !(await listSessions()).find(s => s.id === S1).active_prompt, 'successor to finish', 240000)
    const after = await snapshot()
    const ok = JSON.stringify(unwrap(await H.send(requests.getSessionHistoryOutlineRequest(S1, null, 4)), 'SessionHistoryOutline')).includes('g10-done')
    const summary = { successor_finished_with_answer: ok, A_state_equal: same(before[S1], after[S1]), S2_unchanged: same(before[S2], after[S2]) }
    log('g10run-summary.jsonl', { summary }); console.log(JSON.stringify(summary))
  }
  if (phase === 'g10lease') {
    const oldL = { ...A.remote }
    const machines = unwrap(await H.send(requests.listRemoteMachinesRequest()), 'RemoteMachinesListed')
    const m = (machines.machines ?? machines).find(x => JSON.stringify(x).includes(ids.worker.machine)); const ref = m.machine_ref ?? m.ref ?? m.id ?? ids.worker.machine
    const before = await snapshot()
    const nowRemote = (await listAgents(S1)).find(a => a.id === A.agent)?.remote_execution
    if (nowRemote && nowRemote.execution_lease_id === oldL.execution_lease_id) {
      unwrap(await H.send(requests.moveAgentToLocalRequest(S1, A.agent)), 'AgentMovedToLocal')
      await sleep(3000)
    }
    if (!(await listAgents(S1)).find(a => a.id === A.agent)?.remote_execution) {
      try { unwrap(await H.send(requests.moveAgentToRemoteRequest(S1, A.agent, ref)), 'AgentMovedToRemote') } catch (e) { if (!/already remote-backed/.test(String(e))) throw e }
    }
    // the lease for the new placement is created by the first turn
    const askNew = async word => {
      await submit(S1, A.agent, `Reply with exactly the single word ${word} and nothing else.`)
      await waitFor(async () => JSON.stringify(unwrap(await H.send(requests.getSessionHistoryOutlineRequest(S1, null, 3)), 'SessionHistoryOutline')).includes(word) && !(await listSessions()).find(s => s.id === S1).active_prompt, 'answer ' + word, 240000)
    }
    await askNew('g10lease-first')
    const cur = await waitFor(async () => (await listAgents(S1)).find(a => a.id === A.agent)?.remote_execution, 'new lease after first turn', 60000)
    console.log('old lease', oldL.execution_lease_id, oldL.leased_agent_id, 'new', cur.execution_lease_id, cur.leased_agent_id)
    assert.notEqual(cur.execution_lease_id, oldL.execution_lease_id)
    writeFileSync(`${STATE}/g10-lease.json`, JSON.stringify({ old: oldL, current: cur }))
    const ghost = ctxOf(S1, A, 'run-for-old-lease', { leased_agent_id: oldL.leased_agent_id })
    await sendCase('G10', 'stale-lease-tool', 'worker', 'home', check(ghost), 'refused', 'earlier leased agent id after the placement was replaced')
    await sendCase('G10', 'stale-lease-tool', 'worker', 'home', credTool(ghost), 'refused', 'earlier leased agent id after the placement was replaced')
    for (const op of [
      { kind: 'get_leased_prompt_receipt', leased_agent_id: oldL.leased_agent_id, home_prompt_id: 'p' },
      { kind: 'cancel_leased_prompt', leased_agent_id: oldL.leased_agent_id, home_prompt_id: 'p', worker_provider_run_id: 'r' },
      { kind: 'complete_leased_prompt', leased_agent_id: oldL.leased_agent_id },
      { kind: 'steer_leased_prompt', leased_agent_id: oldL.leased_agent_id, steer_id: 's', target_home_prompt_id: 'p', prompt: 'stale' },
      { kind: 'destroy_execution_lease', lease_id: oldL.execution_lease_id },
      { kind: 'destroy_leased_agent', leased_agent_id: oldL.leased_agent_id },
    ]) await sendCase('G10', 'stale-lease-op', 'home', 'worker', op, 'refused', 'earlier lease/leased agent after replacement')
    await askNew('g10lease-second') // the replacement keeps working after the stale deliveries
    const after = await snapshot()
    const summary = { new_placement_answered: true, S2_unchanged: same(before[S2], after[S2]) }
    log('g10lease-summary.jsonl', { summary }); console.log(JSON.stringify(summary))
    ids.sessions[S1].remote = cur; writeFileSync(idsFile, JSON.stringify(ids, null, 1))
  }
  if (phase === 'g10done') {
    // agent B: finish one prompt, start a second, then deliver a completion that belongs to the first
    await submit(S2, B.agent, 'Reply with exactly the single word b-first and nothing else.')
    await waitFor(async () => !(await listSessions()).find(s => s.id === S2).active_prompt && JSON.stringify(unwrap(await H.send(requests.getSessionHistoryOutlineRequest(S2, null, 3)), 'SessionHistoryOutline')).includes('b-first'), 'B first', 180000)
    const before = await snapshot()
    await submit(S2, B.agent, longPrompt('b-second', 100))
    const run = await waitFor(async () => (await listAgents(S2)).find(a => a.id === B.agent)?.remote_execution?.active_worker_provider_run_id, 'B successor run', 150000)
    const p = (await listSessions()).find(s => s.id === S2).active_prompt?.id
    console.log('B successor run', run, 'prompt', p)
    await sleep(8000) // the successor is running a tool
    const receiptReq = { kind: 'get_leased_prompt_receipt', leased_agent_id: B.remote.leased_agent_id, home_prompt_id: p }
    const rBefore = await peer.send('home', ids.worker.kernel, receiptReq)
    console.log('receipt before', JSON.stringify(rBefore.outcome.response ?? rBefore.outcome).slice(0, 500))
    const rec = await sendCase('G10', 'stale-completion', 'home', 'worker', { kind: 'complete_leased_prompt', leased_agent_id: B.remote.leased_agent_id }, 'refused', 'completion of the earlier prompt delivered while the successor run is active')
    console.log('complete response', JSON.stringify(rec.response_kind), JSON.stringify((await peer.send('home', ids.worker.kernel, receiptReq)).outcome.response ?? '').slice(0, 500))
    await sleep(5000)
    const s = (await listSessions()).find(x => x.id === S2)
    const state = { active_prompt_after: s.active_prompt?.id ?? null, successor_prompt: p, run_after: (await listAgents(S2)).find(a => a.id === B.agent)?.remote_execution?.active_worker_provider_run_id ?? null }
    log('g10done-summary.jsonl', { rec, state }); console.log(JSON.stringify(state))
    await waitFor(async () => !(await listSessions()).find(x => x.id === S2).active_prompt, 'B second finish', 240000)
    const ok = JSON.stringify(unwrap(await H.send(requests.getSessionHistoryOutlineRequest(S2, null, 4)), 'SessionHistoryOutline')).includes('b-second')
    console.log(JSON.stringify({ successor_finished_with_answer: ok }))
  }

  if (phase === 'g10live') {
    // the turn is started by a real client (TUI or web) before this runs; stale deliveries land while it is active
    const tag = process.argv[3] ?? 'tui'
    const lease = JSON.parse(readFileSync(`${STATE}/g10-lease.json`, 'utf8')), g10 = JSON.parse(readFileSync(`${STATE}/g10-active.json`, 'utf8'))
    const run = await waitFor(async () => (await listAgents(S1)).find(a => a.id === A.agent)?.remote_execution?.active_worker_provider_run_id, 'client turn active', 120000)
    const now = (await listAgents(S1)).find(a => a.id === A.agent).remote_execution
    const prompt = (await listSessions()).find(s => s.id === S1).active_prompt?.id
    console.log(tag, 'active run', run, 'prompt', prompt, 'lease', now.execution_lease_id)
    const cur = ctxOf(S1, { agent: A.agent, remote: now }, run)
    const stale = [ctxOf(S1, { agent: A.agent, remote: now }, g10.run2), ctxOf(S1, { agent: A.agent, remote: lease.old }, g10.run2), ctxOf(S1, { agent: A.agent, remote: lease.old }, run)]
    const before = await snapshot()
    await sendCase('G10', `${tag}-valid-current`, 'worker', 'home', credTool(cur), 'accepted', 'current run authorization during the client turn')
    for (const c of stale) { await sendCase('G10', `${tag}-stale-tool`, 'worker', 'home', check(c), 'refused'); await sendCase('G10', `${tag}-stale-tool`, 'worker', 'home', credTool(c), 'refused') }
    const L = now.leased_agent_id
    for (const op of [
      { kind: 'cancel_leased_prompt', leased_agent_id: L, home_prompt_id: g10.p2, worker_provider_run_id: g10.run2 },
      { kind: 'cancel_leased_prompt', leased_agent_id: L, home_prompt_id: prompt, worker_provider_run_id: g10.run2 },
      { kind: 'steer_leased_prompt', leased_agent_id: L, steer_id: `amh-${tag}-stale-steer`, target_home_prompt_id: g10.p2, prompt: 'stale steer must not reach the provider' },
      { kind: 'reconcile_leased_prompt_steer_receipt', leased_agent_id: L, steer_id: `amh-${tag}-stale-steer`, target_home_prompt_id: g10.p2, worker_provider_run_id: g10.run2, execution_lease_id: lease.old.execution_lease_id },
      { kind: 'get_leased_prompt_receipt', leased_agent_id: lease.old.leased_agent_id, home_prompt_id: g10.p2 },
      { kind: 'cancel_leased_prompt', leased_agent_id: lease.old.leased_agent_id, home_prompt_id: prompt, worker_provider_run_id: run },
    ]) await sendCase('G10', `${tag}-stale-lease-op`, 'home', 'worker', op, 'refused')
    await waitFor(async () => !(await listSessions()).find(s => s.id === S1).active_prompt, 'client turn to finish', 240000)
    const after = await snapshot()
    const summary = { tag, run, prompt, S2_unchanged: same(before[S2], after[S2]), lease_unchanged: same(before[S1].agents.map(a => a.remote_execution), after[S1].agents.map(a => a.remote_execution)) }
    log('g10live-summary.jsonl', { summary }); console.log(JSON.stringify(summary))
  }

  if (phase === 'g10stale') {
    // a real client starts the turn on agent A; the stale/duplicate completion lands mid-turn; the client screens are captured by the caller
    const tag = process.argv[3] ?? 'tui'
    const L = (await listAgents(S1)).find(a => a.id === A.agent)
    const run = await waitFor(async () => (await listAgents(S1)).find(a => a.id === A.agent)?.remote_execution?.active_worker_provider_run_id, 'client turn active', 120000)
    const prompt = (await listSessions()).find(s => s.id === S1).active_prompt?.id
    await sleep(10000)
    const rq = { kind: 'get_leased_prompt_receipt', leased_agent_id: L.remote_execution.leased_agent_id, home_prompt_id: prompt }
    console.log(tag, 'run', run, 'prompt', prompt, 'receipt before', JSON.stringify((await peer.send('home', ids.worker.kernel, rq)).outcome.response?.receipt))
    const rec = await sendCase('G10', `${tag}-stale-completion`, 'home', 'worker', { kind: 'complete_leased_prompt', leased_agent_id: L.remote_execution.leased_agent_id }, 'refused', 'late completion while the client-started successor turn runs')
    console.log(tag, 'receipt after', JSON.stringify((await peer.send('home', ids.worker.kernel, rq)).outcome.response?.receipt))
    const t0 = Date.now(); const seen = []
    while (Date.now() - t0 < 30000) { const s = (await listSessions()).find(x => x.id === S1); seen.push(s.active_prompt?.id ?? null); await sleep(2000) }
    console.log(tag, 'active_prompt samples after the late completion:', JSON.stringify([...new Set(seen)]))
    log('g10stale.jsonl', { tag, run, prompt, rec, active_prompt_samples: [...new Set(seen)] })
    await waitFor(async () => !(await listSessions()).find(s => s.id === S1).active_prompt, 'turn finish', 240000)
  }
} catch (e) { console.error('DRIVER FAIL', e.stack?.slice(0, 800) ?? e); await peerStop(1) }
await peerStop(0)
