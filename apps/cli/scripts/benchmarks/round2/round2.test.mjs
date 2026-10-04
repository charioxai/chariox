// MP-08 / MP-10 / MP-11. Synthetic fixtures only; no expected benchmark answers.
import assert from 'node:assert/strict'
import test from 'node:test'
import { pathToFileURL } from 'node:url'
const fragments = await import(pathToFileURL(process.env.CHARIOX_FRAGMENT_HELPERS_MODULE).href)
import { PassiveHar } from './har.mjs'
import { finalAnswer, settlementRecord } from './export.mjs'
import { sampleExistingPages } from './webmall-freshness.mjs'

const entry = (index, text, start = 0, total = Array.from(text).length, kind = 'provider_output', metadata = {}) => ({
  entry_index: index, fragment_start: start, fragment_end: start + Array.from(text).length,
  total_chars: total, entry: { kind, text, provider_run_id: 'run-1', merge_key: 'final-1', timestamp_ms: index, ...metadata },
})
const event = (method, params, sessionId = 'session-1') => ({ method: `Network.${method}`, params, sessionId })
const request = (url = 'https://fixture.test/a', headers = {}) => ({ requestId: 'r', request: { url, method: 'GET', headers }, wallTime: 1, timestamp: 1, type: 'Document' })

test('MP-08 / MP-10 / MP-11 H1 actual ExtraInfo Accept and header allowlist', () => {
  const har = new PassiveHar()
  har.consume(event('requestWillBeSentExtraInfo', { requestId: 'r', headers: { Accept: 'text/html', Cookie: 'fixture-private', Authorization: 'fixture-private', 'X-Private': 'fixture-private' } }))
  har.consume(event('requestWillBeSent', request()))
  har.consume(event('responseReceived', { requestId: 'r', hasExtraInfo: true, response: { status: 200, headers: {} } }))
  assert.deepEqual(har.entries[0].request.headers, [{ name: 'accept', value: 'text/html' }])
  assert.equal(JSON.stringify(har.entries).includes('fixture-private'), false)
  assert.equal(har.entries[0]._requestMetadata.headersSource, 'Network.requestWillBeSentExtraInfo')
})

test('MP-08 / MP-10 / MP-11 H1 redirect/session correlation skips requests without ExtraInfo', () => {
  const har = new PassiveHar()
  har.consume(event('requestWillBeSent', request()))
  har.consume(event('requestWillBeSentExtraInfo', { requestId: 'r', headers: { Accept: 'text/html;second' } }))
  har.consume(event('requestWillBeSent', { ...request('https://fixture.test/b'), redirectHasExtraInfo: false, redirectResponse: { status: 302, headers: { Location: 'https://fixture.test/b' } } }))
  har.consume(event('responseReceived', { requestId: 'r', hasExtraInfo: true, response: { status: 200, headers: {} } }))
  har.consume(event('requestWillBeSent', request(), 'session-2'))
  har.consume(event('responseReceived', { requestId: 'r', hasExtraInfo: true, response: { status: 200, headers: {} } }, 'session-2'))
  har.consume(event('requestWillBeSentExtraInfo', { requestId: 'r', headers: { Accept: 'application/json' } }, 'session-2'))
  assert.deepEqual(har.entries.map(x => x.request.headers), [[], [{ name: 'accept', value: 'text/html;second' }], [{ name: 'accept', value: 'application/json' }]])
  assert.deepEqual(har.entries.map(x => x._requestMetadata.redirectIndex), [0, 1, 0])
})

test('MP-08 / MP-10 / MP-11 H2 Unicode/out-of-order/duplicate fragments reconstruct final once', () => {
  const text = 'Résumé 👩‍💻\né and 日本語.\n'
  const chars = Array.from(text), total = chars.length
  const summary = entry(9, chars.slice(0, 4).join(''), 0, total)
  const chunks = [entry(9, chars.slice(4, 11).join(''), 4, total), entry(9, chars.slice(11).join(''), 11, total), summary]
  assert.equal(finalAnswer({ lifecycle: 'completed', summary }, [entry(8, 'commentary'), ...chunks, chunks[0], summary], fragments), text)
})

test('MP-08 / MP-10 / MP-11 H2 combined summary reconstructs original message entries', () => {
  const summary = entry(9, 'hello ', 0, 12)
  assert.equal(finalAnswer({ lifecycle: 'completed', summary }, [entry(8, 'commentary', 0, 10, 'provider_output', { merge_key: 'commentary' }), entry(10, '🌍.\n!!?'), entry(9, 'hello ')], fragments), 'hello 🌍.\n!!?')
})

test('MP-08 / MP-10 / MP-11 H2 missing/conflicting final text fails export', () => {
  const summary = entry(9, 'abc', 0, 6)
  assert.throws(() => finalAnswer({ lifecycle: 'completed', summary }, [summary], fragments), /incomplete/)
  assert.throws(() => finalAnswer({ lifecycle: 'completed', summary }, [entry(9, 'abc', 0, 6), entry(9, 'XYZ', 0, 6), entry(9, 'def', 3, 6)], fragments), /conflict/)
  assert.throws(() => finalAnswer({ lifecycle: 'completed' }, [entry(9, '.')], fragments), /summary/)
})

test('MP-08 / MP-10 / MP-11 H7 failure records settlement seam, identities and structured error', () => {
  const record = settlementRecord({ turn: { lifecycle: 'failed', prompt_id: 'prompt-1', turn_id: 'turn-1' }, entries: [entry(12, JSON.stringify({ code: 'provider_unavailable', message: 'fixture failure', authorization: 'fixture-private' }), 0, undefined, 'provider_error')], elapsedMs: 14877, sessionId: 'room-1', agentId: 'agent-1' })
  assert.equal(record.firstFailingSeam, 'provider_settlement')
  assert.equal(record.status, 'RED')
  assert.equal(record.denominatorIncluded, true)
  assert.equal(record.promptId, 'prompt-1')
  assert.equal(record.turnId, 'turn-1')
  assert.equal(record.elapsedMs, 14877)
  assert.equal(record.providerErrors[0].error.code, 'provider_unavailable')
  assert.equal(JSON.stringify(record).includes('fixture-private'), false)
})

test('MP-08 / MP-10 / MP-11 H7 absent error is unknown and resource cancellation remains distinct', () => {
  const failed = settlementRecord({ turn: { lifecycle: 'failed' }, entries: [], elapsedMs: 3 })
  assert.equal(failed.failureCause, 'unknown')
  const cancelled = settlementRecord({ turn: { lifecycle: 'cancelled' }, entries: [], elapsedMs: 8, cancellation: { cause: 'resource_floor', cleanupReceipt: { ownedProcessesStopped: true } } })
  assert.equal(cancelled.failureCause, 'resource_floor')
  assert.deepEqual(cancelled.cancellation.cleanupReceipt, { ownedProcessesStopped: true })
  assert.equal(cancelled.denominatorIncluded, true)
})

test('MP-08 / MP-10 / MP-11 H6 synthetic external navigation pumps before cached URL gate, across tabs', async () => {
  const makePage = () => ({ cached: 'https://shop.fixture/', current: 'https://frontend.fixture/', submitted: 'fixture submission', calls: [], url() { return this.cached }, async title() { this.calls.push('title'); this.cached = this.current; return 'fixture' } })
  const pages = [makePage(), makePage()]
  const validate = page => ({ done: page.url() === 'https://frontend.fixture/', text: page.url() === 'https://frontend.fixture/' ? page.submitted : null })
  assert.equal(validate(pages[0]).done, false)
  const rows = await sampleExistingPages({ pages: () => pages }, validate)
  assert.deepEqual(rows.map(x => x.result), pages.map(() => ({ done: true, text: 'fixture submission' })))
  assert.deepEqual(pages.map(x => x.calls), [['title'], ['title']])
})
