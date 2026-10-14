import test from 'node:test'
import assert from 'node:assert/strict'
import { summarizeBriefTransition, foreignNativeSession, formatContextValidationFailure } from './context-switch-validation-brief.mjs'

test('MP-08 MP-10 MP-11 detects durable watermark regression and absent folds', () => {
  const before = { brief: { coveredThroughSequence: 22, sha256: 'one' } }
  assert.equal(summarizeBriefTransition(before, { brief: { coveredThroughSequence: 21, sha256: 'two' } }).watermarkRegressed, true)
  assert.equal(summarizeBriefTransition(before, {}).watermarkRegressed, true)
  assert.equal(summarizeBriefTransition({}, {}).hasStoredBrief, false)
})

test('MP-08 MP-10 MP-11 missing session identity cannot prove absence of foreign reuse', () => {
  assert.deepEqual(foreignNativeSession({ provider: 'codex', nativeId: 'a' }, { provider: 'opencode' }), { verified: false, reused: null })
  assert.equal(foreignNativeSession({ provider: 'codex', nativeId: 'a' }, { provider: 'opencode', nativeId: 'a' }).reused, true)
  assert.equal(foreignNativeSession({ provider: 'codex', nativeId: 'a' }, { provider: 'opencode', nativeId: 'b' }).reused, false)
})

test('MP-08 MP-10 MP-11 request failure never retains Playwright header call logs', () => {
  const failure = 'Error: request timed out\nCall log:\n - cookie: __Host-chariox_cloud_session=synthetic'
  assert.equal(formatContextValidationFailure(failure), 'Error: request timed out')
  assert.equal(formatContextValidationFailure('Error: __Host-chariox_cloud_session=synthetic'), 'Error: [redacted validation cookie]')
})
