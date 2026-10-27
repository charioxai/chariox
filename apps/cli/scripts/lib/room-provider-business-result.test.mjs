// MP-08/MP-10: both official provider envelopes must prove the same tool values.
import assert from 'node:assert/strict'
import test from 'node:test'
import { roomProviderToolOutput } from './room-provider-tool-record.mjs'
import { roomProviderBusinessResult } from './room-provider-business-result.mjs'

test('OpenCode object-wrapped JSON result preserves completed vendor business data', () => {
  const rows = [
    { vendor: 'Acme', price: 200, warranty: 2 },
    { vendor: 'Beta', price: 160, warranty: 1 },
    { vendor: 'Gamma', price: 250, warranty: 3 },
  ]
  const codex = { structuredContent: rows }
  const opencode = { structuredContent: { result: JSON.stringify(rows) } }
  for (const envelope of [codex, opencode]) {
    assert.deepEqual(roomProviderBusinessResult(roomProviderToolOutput(JSON.stringify(envelope))), { rows })
  }
  assert.equal(roomProviderBusinessResult({ result: 'not JSON' }), null)
})
