// MP-08 / MP-10 / MP-11: the actual runner's final-output boundary; no grading.
import assert from 'node:assert/strict'
import test from 'node:test'
import { pathToFileURL } from 'node:url'
import { webGamesFinalOutput } from './webgames-answer.mjs'
const helpers = await import(pathToFileURL(process.env.CHARIOX_FRAGMENT_HELPERS_MODULE))
const entry = (index, text, total = Array.from(text).length) => ({
  entry_index: index, fragment_start: 0, fragment_end: Array.from(text).length, total_chars: total,
  entry: { kind: 'provider_output', text, provider_run_id: 'fixture-run', merge_key: 'fixture-final', timestamp_ms: index },
})

test('MP-08/MP-10 WebGames exports long combined messages without summary-preview conflicts', () => {
  const first = '日本語😀'.repeat(2500), second = '\nsecond event\n'.repeat(700)
  const full = first + second, preview = Array.from(full).slice(0, 16384).join('')
  const summary = entry(1, preview, Array.from(full).length)
  const originals = [entry(1, first), entry(2, second)]
  const turn = { lifecycle: 'completed', summary, entries: [summary] }
  assert.throws(() => helpers.assembleSessionHistoryFinalMessage(turn, [...originals, summary]), /conflicting history fragment metadata/)
  assert.equal(webGamesFinalOutput(turn, originals, helpers), full)
})

test('MP-08/MP-10 WebGames incomplete originals stay an export error', () => {
  const turn = { lifecycle: 'completed', summary: entry(1, 'abc', 6) }
  assert.throws(() => webGamesFinalOutput(turn, [entry(1, 'abc')], helpers), /incomplete final message/)
})

test('MP-08/MP-10 WebGames supports a complete inline final message', () => {
  const summary = entry(1, 'complete inline fixture answer')
  assert.equal(webGamesFinalOutput({ lifecycle: 'completed', summary }, [], helpers), summary.entry.text)
})
