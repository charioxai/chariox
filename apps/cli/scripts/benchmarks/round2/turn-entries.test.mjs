// MP-08/MP-10/MP-11: audit original complete events, retaining missing-data RED.
import assert from 'node:assert/strict'
import test from 'node:test'
import { assembleTurnEntries } from './kernel.mjs'
assert.ok(process.env.CHARIOX_FRAGMENT_HELPERS_MODULE)
const helpers = await import(process.env.CHARIOX_FRAGMENT_HELPERS_MODULE)

const text = JSON.stringify({ id: 'call-1', tool: 'slice_browser_text', status: 'completed', output: '日本 😀'.repeat(100) })
const chars = Array.from(text)
const fragment = (start, end) => ({ entry_index: 3, fragment_start: start, fragment_end: end, total_chars: chars.length,
  entry: { kind: 'provider_tool', provider_run_id: 'run-1', merge_key: null, text: chars.slice(start, end).join('') } })

test('MP-08/MP-10 complete fragmented tool JSON audits once with exact Unicode', () => {
  const left = fragment(0, 100), right = fragment(100, chars.length)
  const entries = assembleTurnEntries([right, left, right], helpers)
  assert.equal(entries.length, 1)
  assert.equal(entries[0].entry.text, text)
  assert.deepEqual(JSON.parse(entries[0].entry.text), JSON.parse(text))
})

test('MP-08/MP-10 missing original tool fragments fail instead of becoming an allowed call', () => {
  assert.throws(() => assembleTurnEntries([fragment(0, 100)], helpers))
})
