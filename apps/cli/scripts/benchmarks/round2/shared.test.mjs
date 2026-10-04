// MP-08 / MP-10 / MP-11. Product RPC seams are synthetic, never live.
import assert from 'node:assert/strict'
import test from 'node:test'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { PassiveHar, observePassiveHar, Round2Room, loadTurnHistory, waitForSettlement, runRound2Episode, settlementRecord, finalAnswer } from './index.mjs'

const helpers = await import(pathToFileURL(process.env.CHARIOX_FRAGMENT_HELPERS_MODULE).href)
const entry = (index, text, kind = 'provider_output', start = 0, total = Array.from(text).length, extra = {}) => ({
  entry_index: index, fragment_start: start, fragment_end: start + Array.from(text).length, total_chars: total,
  entry: { kind, text, provider_run_id: 'run-1', merge_key: 'final', timestamp_ms: index, ...extra },
})

function fixture({ lifecycle = 'completed', collisions = 0, cleanupFailure = false, incompleteFinal = false, finalText = 'Fixture final 👋.\n' } = {}) {
  const calls = [], slices = [], sessions = []
  let created = 0, turn = { lifecycle, prompt_id: 'prompt-1', turn_id: 'turn-1', entries: [], summary: entry(9, finalText), blobs: [] }
  if (incompleteFinal) turn.summary = entry(9, 'Fixture', 'provider_output', 0, 20)
  if (lifecycle === 'failed') turn.entries = [entry(10, '{"code":"provider_unavailable","message":"fixture failure"}', 'provider_error')]
  const requests = new Proxy({}, { get: (_, method) => (...args) => ({ method, args }) })
  const client = { async send({ method, args }) {
    calls.push({ method, args })
    switch (method) {
      case 'createSessionRequest': sessions.push({ id: 'room-1' }); return { SessionCreated: { session: sessions[0] } }
      case 'createSliceRequest': { const slice = { id: `slice-${++created}`, name: args[0].name }; slices.push(slice); return { SliceCreated: { slice } } }
      case 'startSliceRequest': if (collisions-- > 0) throw Error('host port(s) 123 are already in use'); return { SliceStarted: {} }
      case 'bindRoomEnvironmentSliceRequest': return { RoomEnvironmentSlice: {} }
      case 'startRoomEnvironmentRequest': return { RoomEnvironmentUpdated: {} }
      case 'spawnAgentRequest': return { AgentSpawned: { agent: { id: 'agent-1' } } }
      case 'attachToSessionRequest': return { SessionAttached: { attachment: { id: 'attachment-1' } } }
      case 'submitPromptRequest': return { PromptSubmitted: { outcome: { Started: { prompt: { id: 'prompt-1' } } } } }
      case 'getSessionHistoryOutlineRequest': return { SessionHistoryOutline: { agents: [{ agent_id: 'agent-1', turns: [turn] }] } }
      case 'getSessionHistoryBlobContentRequest': return { SessionHistoryBlobContent: { blob_id: args[2], entries: [entry(9, 'Fixture final 👋.\n')] } }
      case 'cancelActivePromptRequest': turn = { ...turn, lifecycle: 'cancelled' }; return { PromptCancelled: {} }
      case 'detachFromSessionRequest': return { SessionDetached: {} }
      case 'deleteSessionRequest': if (cleanupFailure) throw Error('fixture cleanup failure'); sessions.splice(0); return { SessionDeleted: {} }
      case 'deleteSliceRequest': slices.splice(slices.findIndex(slice => slice.id === args[0]), 1); return { SliceDeleted: {} }
      case 'listSessionsRequest': return { SessionsListed: { sessions } }
      case 'listSlicesRequest': return { SlicesListed: { slices } }
      default: throw Error(`unexpected fixture method ${method}`)
    }
  } }
  return { api: { client, requests, helpers }, calls, getTurn: () => turn }
}

test('MP-08 / MP-10 / MP-11 H1 every ExtraInfo ordering across redirect hops stays passive', () => {
  for (const placement of ['before', 'after']) {
    const har = new PassiveHar()
    const extra = accept => ({ method: 'Network.requestWillBeSentExtraInfo', sessionId: 's', params: { requestId: 'r', headers: { Accept: accept, cookie: 'fixture-private' } } })
    const req = (url, redirect = {}) => ({ method: 'Network.requestWillBeSent', sessionId: 's', params: { requestId: 'r', wallTime: 1, timestamp: 1, request: { method: 'GET', url, headers: {} }, ...redirect } })
    if (placement === 'before') { har.consume(extra('text/html;a')); har.consume(extra('text/html;b')) }
    har.consume(req('https://fixture.test/a?token=fixture-private&x=1'))
    har.consume(req('https://fixture.test/b', { redirectHasExtraInfo: true, redirectResponse: { status: 302, headers: {} } }))
    har.consume({ method: 'Network.responseReceived', sessionId: 's', params: { requestId: 'r', hasExtraInfo: true, response: { status: 200, headers: {} } } })
    if (placement === 'after') { har.consume(extra('text/html;a')); har.consume(extra('text/html;b')) }
    assert.deepEqual(har.entries.map(item => item.request.headers[0].value), ['text/html;a', 'text/html;b'])
    assert.equal(har.snapshot().log._capture.missingExtraInfo, 0)
    assert.equal(JSON.stringify(har.snapshot()).includes('fixture-private'), false)
  }
  const calls = [], observers = []
  return observePassiveHar({ send: async (...args) => calls.push(args), subscribe: fn => { observers.push(fn); return () => observers.pop() }, sessionIds: ['s1', 's2'] }).then(observer => {
    assert.deepEqual(calls.map(call => call[0]), ['Network.enable', 'Network.enable'])
    observer.detach(); assert.equal(observers.length, 0)
  })
})

test('MP-08 / MP-10 / MP-11 H1 missing metadata is explicit; HTML Accept is never invented', () => {
  const har = new PassiveHar()
  har.consume({ method: 'Network.requestWillBeSent', params: { requestId: 'r', wallTime: 1, timestamp: 1, request: { method: 'GET', url: 'https://fixture.test/', headers: {} } } })
  har.consume({ method: 'Network.responseReceived', params: { requestId: 'r', hasExtraInfo: true, response: { status: 200, headers: {} } } })
  assert.equal(har.snapshot().log._capture.missingExtraInfo, 1)
  assert.deepEqual(har.entries[0].request.headers, [])
})

test('MP-08 / MP-10 / MP-11 H2 cloned previews/duplicate full blobs are never doubled', () => {
  const summary = entry(9, 'foo', 'provider_output', 0, 6)
  const originals = [structuredClone(summary), entry(9, 'bar', 'provider_output', 3, 6), structuredClone(summary)]
  assert.equal(finalAnswer({ lifecycle: 'completed', summary }, originals, helpers), 'foobar')
  const complete = entry(9, 'Résumé 👋\n')
  assert.equal(finalAnswer({ lifecycle: 'completed', summary: complete }, [structuredClone(complete), structuredClone(complete)], helpers), complete.entry.text)
  assert.throws(() => helpers.assembleSessionHistoryEntry([{ ...complete, fragment_end: complete.entry.text.length, total_chars: complete.entry.text.length }]), /length/)
})

test('MP-08 / MP-10 / MP-11 H7 fragmented error assembles once; incomplete error is not guessed', () => {
  const text = '{"code":"fixture_failure","message":"fixture"}'
  const fragments = [entry(10, text.slice(8), 'provider_error', 8, text.length), entry(10, text.slice(0, 8), 'provider_error', 0, text.length)]
  const args = { turn: { lifecycle: 'failed' }, elapsedMs: 14877, helpers }
  const complete = settlementRecord({ ...args, entries: [...fragments, fragments[0]] })
  assert.equal(complete.providerErrors.length, 1)
  assert.equal(complete.providerErrors[0].error.code, 'fixture_failure')
  const incomplete = settlementRecord({ ...args, entries: [fragments[0]] })
  assert.deepEqual(incomplete.providerErrors[0], { entryIndex: 10, complete: false, error: null })
  assert.equal(incomplete.status, 'RED')
})

test('MP-08 / MP-10 / MP-11 Room recreates owned slices on port collision and proves absence', async () => {
  const f = fixture({ collisions: 2 }), checkpoints = []
  const room = new Round2Room(f.api, { workspace: '/fixture/work', runId: 'fixture-round2', checkpoint: async owned => checkpoints.push(structuredClone(owned)) })
  await room.setup({ verifySlice: async identity => assert.equal(identity.sliceId, 'slice-3') })
  assert.equal(room.owned.portRetries, 2)
  assert.equal(f.calls.filter(call => call.method === 'createSliceRequest').length, 3)
  assert.equal(checkpoints.some(checkpoint => checkpoint.portRetries === 2), true)
  assert.equal((await room.cleanup()).complete, true)
})

test('MP-08 / MP-10 / MP-11 Room bounds collision retries; failed cleanup retains execution environment', async () => {
  const f = fixture({ collisions: 3, cleanupFailure: true })
  const room = new Round2Room(f.api, { workspace: '/fixture/work', runId: 'fixture-round2' })
  await assert.rejects(room.setup(), /already in use/)
  assert.equal(f.calls.filter(call => call.method === 'createSliceRequest').length, 3)
  const deletionCount = f.calls.filter(call => call.method === 'deleteSliceRequest').length
  assert.equal((await room.cleanup()).complete, false)
  assert.equal(f.calls.filter(call => call.method === 'deleteSliceRequest').length, deletionCount)
})

test('MP-08 / MP-10 / MP-11 H2 blob loading deduplicates IDs and excludes summary previews', async () => {
  const f = fixture(), turn = { ...f.getTurn(), entries: [], blobs: [{ blob_id: 'blob-1' }, { blob_id: 'blob-1' }] }
  const entries = await loadTurnHistory(f.api, { sessionId: 'room-1', agentId: 'agent-1', turn })
  assert.equal(entries.length, 1)
  assert.equal(f.calls.filter(call => call.method === 'getSessionHistoryBlobContentRequest').length, 1)
})

test('MP-08 / MP-10 / MP-11 H7 cancellation cause, elapsed time and settlement survive polling', async () => {
  const f = fixture({ lifecycle: 'open' }), identity = { sessionId: 'room-1', agentId: 'agent-1', promptId: 'prompt-1' }
  let clock = 0
  const result = await waitForSettlement(f.api, identity, { maxMs: 2000, now: () => clock, pause: async ms => { clock += ms }, guard: async () => clock >= 1000 ? { cause: 'resource_floor' } : null,
    cancel: async () => f.api.client.send(f.api.requests.cancelActivePromptRequest()) })
  assert.equal(result.turn.lifecycle, 'cancelled')
  assert.equal(result.cancellation.cause, 'resource_floor')
  assert.equal(result.elapsedMs, 1000)
})

test('MP-08 / MP-10 / MP-11 H7 sampler failure cannot hide later provider cancellation', async () => {
  const f = fixture({ lifecycle: 'open' }), identity = { sessionId: 'room-1', agentId: 'agent-1', promptId: 'prompt-1' }
  let clock = 0
  const result = await waitForSettlement(f.api, identity, { maxMs: 1000, now: () => clock, pause: async ms => { clock += ms },
    observe: async () => { throw Error('fixture evaluator failure') }, cancel: async () => f.api.client.send(f.api.requests.cancelActivePromptRequest()) })
  assert.equal(result.turn.lifecycle, 'cancelled')
  assert.equal(result.observationErrors.length, 1)
  assert.equal(result.cancellation.cause, 'wall_time')
})

for (const lifecycle of ['completed', 'failed']) test(`MP-08 / MP-10 / MP-11 shared episode ${lifecycle} exports settlement before grader`, async () => {
  const root = process.env.CHARIOX_ROUND2_TEST_STATE
  assert.ok(path.isAbsolute(root), 'external lane-owned test state required')
  const directory = await mkdtemp(path.join(root, 'episode-'))
  try {
    const f = fixture({ lifecycle }), order = []
    const row = await runRound2Episode({ api: f.api, directory, source: { runtimeCommit: 'fixture-runtime', runnerCommit: 'fixture-runner' },
      roomOptions: { workspace: '/fixture/work', runId: 'fixture-round2' }, roomSetup: { verifySlice: async () => {} },
      provider: { provider: 'codex', model: 'fixture-model', accountProfile: 'fixture-profile' }, prompt: 'Synthetic task', maxMs: 2000, guard: async () => null,
      audit: async () => order.push('audit'), grade: async ({ answer }) => { order.push('grade'); assert.equal(answer, f.getTurn().summary.entry.text); return { synthetic: true } },
      verifyCleanup: async () => ({ complete: true }) })
    assert.equal(row.denominatorIncluded, true)
    assert.equal(row.cleanup.complete, true)
    assert.equal(f.calls.filter(call => call.method === 'submitPromptRequest').length, 1)
    if (lifecycle === 'failed') {
      assert.equal(row.status, 'RED'); assert.equal(row.firstFailingSeam, 'provider_settlement'); assert.deepEqual(order, [])
      await assert.rejects(readFile(path.join(directory, 'answer.txt')), { code: 'ENOENT' })
    } else { assert.equal(row.status, 'scored'); assert.deepEqual(order, ['audit', 'grade']) }
    const exported = JSON.parse(await readFile(path.join(directory, 'run.json'), 'utf8'))
    assert.equal(exported.settlement.lifecycle, lifecycle)
  } finally { await rm(directory, { recursive: true }) }
})

test('MP-08 / MP-10 / MP-11 H2 incomplete complete-turn summary prevents scorer admission', async () => {
  const directory = await mkdtemp(path.join(process.env.CHARIOX_ROUND2_TEST_STATE, 'episode-'))
  try {
    const f = fixture({ incompleteFinal: true })
    let gradeCalled = false
    const row = await runRound2Episode({ api: f.api, directory, source: { runtimeCommit: 'fixture-runtime', runnerCommit: 'fixture-runner' },
      roomOptions: { workspace: '/fixture/work', runId: 'fixture-round2' }, roomSetup: { verifySlice: async () => {} },
      provider: { provider: 'codex', model: 'fixture-model', accountProfile: 'fixture-profile' }, prompt: 'Synthetic task', maxMs: 2000, guard: async () => null,
      audit: async () => {}, grade: async () => { gradeCalled = true }, verifyCleanup: async () => ({ complete: true }) })
    assert.equal(row.status, 'RED')
    assert.equal(row.firstFailingSeam, 'final_response_export')
    assert.equal(row.denominatorIncluded, true)
    assert.equal(gradeCalled, false)
    await assert.rejects(readFile(path.join(directory, 'answer.txt')), { code: 'ENOENT' })
  } finally { await rm(directory, { recursive: true }) }
})

for (const [label, finalText, contract, expected] of [
  ['name', '"Éva Fixture"', { kind: 'name' }, 'Éva Fixture'],
  ['set', '["Éva", "李"]', { kind: 'set' }, ['Éva', '李']],
  ['number', '17\n', { kind: 'number' }, '17'],
  ['records', '{"id":"synthetic-1","answer":[{"name":"Fixture","category":"wrong"}]}', { kind: 'records', fields: ['name', 'category'], taskId: 'synthetic-1' }, [{ name: 'Fixture', category: 'wrong' }]],
  ['invalid', 'The names are Éva and 李.', { kind: 'set' }, null],
]) test(`MP-08 / MP-10 / MP-11 H3/H4 episode ${label} declares policy and preserves official zero`, async () => {
  const directory = await mkdtemp(path.join(process.env.CHARIOX_ROUND2_TEST_STATE, 'policy-episode-'))
  try {
    const f = fixture({ finalText }), policies = []
    let gradeCalls = 0
    const row = await runRound2Episode({ api: f.api, directory, source: { runtimeCommit: 'fixture-runtime', runnerCommit: 'fixture-runner' },
      roomOptions: { workspace: '/fixture/work', runId: 'fixture-round2' }, roomSetup: { verifySlice: async () => {} },
      provider: { provider: 'codex', model: 'fixture-model', accountProfile: 'fixture-profile' }, prompt: 'READ-ONLY synthetic task', answerContract: contract,
      maxMs: 2000, guard: async () => null, audit: async ({ policy }) => policies.push(policy),
      grade: async ({ answer, policy }) => {
        gradeCalls++; policies.push(policy)
        assert.deepEqual(answer, expected)
        return { score: 0, historicalGoldDisagreement: true, synthetic: true }
      }, verifyCleanup: async () => ({ complete: true }) })
    assert.equal(row.runner, 'round2-shared-v2')
    assert.equal(row.policy.runDate, row.startedAt.slice(0, 10))
    assert.equal(row.policy.consent, 'reject_nonessential')
    assert.deepEqual(row.policy.answerContract, contract)
    assert.equal(policies.every(policy => policy === row.policy), true)
    assert.equal(row.denominatorIncluded, true)
    assert.equal(row.cleanup.complete, true)
    assert.equal(f.calls.filter(call => call.method === 'submitPromptRequest').length, 1)
    const submitted = f.calls.find(call => call.method === 'submitPromptRequest').args[3]
    assert.match(submitted, /Always reject non-essential consent/)
    assert.equal(submitted.includes(`${row.policy.runDate} (UTC)`), true)
    const retained = JSON.parse(await readFile(path.join(directory, 'run.json'), 'utf8'))
    assert.deepEqual(retained.policy, row.policy)
    if (expected === null) {
      assert.equal(row.status, 'RED'); assert.equal(row.firstFailingSeam, 'final_response_export'); assert.equal(gradeCalls, 0)
      await assert.rejects(readFile(path.join(directory, 'answer-package.json')), { code: 'ENOENT' })
    } else {
      assert.equal(row.status, 'scored'); assert.equal(row.grade.score, 0); assert.equal(row.grade.historicalGoldDisagreement, true)
      assert.equal(gradeCalls, 1)
      assert.equal(await readFile(path.join(directory, 'answer.txt'), 'utf8'), finalText)
      const packaged = JSON.parse(await readFile(path.join(directory, 'answer-package.json'), 'utf8'))
      assert.deepEqual(packaged.answer, expected)
      assert.deepEqual(packaged.contract, contract)
    }
  } finally { await rm(directory, { recursive: true }) }
})
