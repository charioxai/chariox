// MP-08/MP-10/MP-11: distinguish provider admission failures from scored browser losses.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { providerFailureFlags, requiresCampaignPause } from './provider-availability.mjs'

test('MP-08/MP-10 exact product weekly-exhaustion wording is recognized', () => {
  const flags = providerFailureFlags('selected codex account cannot start new work because its provider usage is exhausted (weekly); refresh usage or select another account')
  assert.equal(flags.usageExhausted, true)
  assert.equal(flags.weeklyWindow, true)
  assert.equal(flags.unauthorized, false)
  assert.equal(providerFailureFlags('weekly public report').usageExhausted, false)
})
test('MP-08/MP-10/MP-11 failed provider or judge pauses new tasks, scored loss continues', () => {
  const loss = { cleanupValid: true, turnLifecycle: 'completed', harnessValid: true, judgeValid: true, judgeVerdict: 'NOT SUCCESS' }
  assert.equal(requiresCampaignPause(loss), false)
  for (const marker of [{ providerError: true }, { providerUsageExhausted: true }, { providerUnauthorized: true },
    { turnLifecycle: 'failed' }, { turnLifecycle: 'cancelled' }, { judgeFailure: { exitCode: 1 } }, { cleanupValid: false }]) {
    assert.equal(requiresCampaignPause({ ...loss, ...marker }), true)
  }
})
