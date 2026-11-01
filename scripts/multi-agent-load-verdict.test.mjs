import assert from 'node:assert/strict'
import test from 'node:test'
import { laneChecks, verdictOf } from './drills/kernel-responsiveness/multi-agent-verdict.mjs'

// The other KR-06 checks are healthy, as in a run whose generators all work.
const healthy = { plain_call_p99_under_load_ms: { value: 3, limit: 100 }, load_errors: { value: 0, limit: 0 } }
const verdict = lanes => verdictOf({ ...healthy, ...laneChecks(lanes) })
const working = { launch: 'RunStarted', prompt_cycles: 12 }
const failedLaunch = { launch_error: 'LaunchProviderRun: provider_unavailable: no adapter', prompt_cycles: 0 }

test('GREEN when every requested lane launched and completed prompt cycles', () => {
  const { verdict: v, red } = verdict([working, working, working])
  assert.ok(v === 'GREEN', red.join(' '))
})

test('RED when every requested launch fails', () => {
  const { verdict: v, red } = verdict([failedLaunch, failedLaunch])
  assert.ok(v === 'RED')
  assert.deepEqual(red, ['lanes_launch_failed', 'lanes_without_prompt_cycle'])
})

test('RED when one requested launch fails', () => {
  const { verdict: v, red } = verdict([working, failedLaunch, working])
  assert.ok(v === 'RED')
  assert.ok(red.includes('lanes_launch_failed'))
})

test('RED when a launched lane never completes a prompt cycle', () => {
  const { verdict: v, red } = verdict([working, { launch: 'RunStarted', prompt_cycles: 0 }])
  assert.ok(v === 'RED')
  assert.deepEqual(red, ['lanes_without_prompt_cycle'])
})

test('a readers-only run with no lanes has nothing to fail', () => {
  assert.ok(verdict([]).verdict === 'GREEN')
})
