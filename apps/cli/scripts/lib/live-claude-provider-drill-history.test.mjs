import assert from 'node:assert/strict'
import test from 'node:test'

import { claudeProviderOutputContainsMarker } from './live-claude-provider-drill-history.mjs'

test('Claude drill requires the marker in provider output', () => {
  const marker = 'CHARIOX_CLAUDE_DRILL_MARKER'

  assert.equal(claudeProviderOutputContainsMarker([
    { kind: 'user_prompt', text: `Respond with exactly ${marker}` },
  ], marker), false)
  assert.equal(claudeProviderOutputContainsMarker([
    { kind: 'provider_output', text: 'CHARIOX_CLAUDE_' },
    { kind: 'provider_output', text: 'DRILL_MARKER' },
  ], marker), true)
  assert.equal(claudeProviderOutputContainsMarker([
    { kind: 'notice', text: `Provider exited after ${marker}` },
  ], marker), false)
})
