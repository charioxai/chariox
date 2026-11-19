import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'
import { budgetChecks, laneChecks, rssSlopeAfterWarmup, verdictOf } from './drills/kernel-responsiveness/multi-agent-verdict.mjs'

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

test('RED when completed turns leave their tasks working, GREEN with one turn in flight', () => {
  const settled = { ...working, unsettled_tasks: 1 }
  assert.ok(verdict([settled, settled]).verdict === 'GREEN')
  const piling = verdict([settled, { ...working, unsettled_tasks: 40 }])
  assert.ok(piling.verdict === 'RED')
  assert.deepEqual(piling.red, ['lanes_with_unsettled_tasks'])
})

test('the RSS slope ignores the start-up ramp before the warm-up', () => {
  // 100 MB/min for the first 10 minutes, then flat at 1 MB/min.
  const samples = []
  for (let t = 0; t <= 1800; t += 5) samples.push({ t_s: t, rss_kb: t <= 600 ? 1024 * (100 * t / 60) : 1024 * (1000 + (t - 600) / 60) })
  assert.ok(rssSlopeAfterWarmup(samples, 600) === 1)
  assert.ok(rssSlopeAfterWarmup(samples, 0) > 10)
  assert.ok(rssSlopeAfterWarmup(samples, 1800) === null)
})

const budgets = { outline_p95_ms: { max: 200 }, session_state_p95_ms: { max: 100 }, attach_view_p95_ms: { max: 300 }, stalled_operations: { max: 0 } }
const latency = { GetSessionHistoryOutline: { p95_ms: 40 }, GetSessionState: { p95_ms: 20 }, AttachSessionView: { p95_ms: 60 } }

test('GREEN when the outline, state and attach p95 are within perf-budgets/kernel.json and nothing stalled', () => {
  const { verdict: v, red } = verdictOf(budgetChecks(budgets, latency, 0))
  assert.ok(v === 'GREEN', red.join(' '))
})

test('RED when a p95 is over its budget, a probe never ran, or the kernel logged a stalled operation', () => {
  assert.deepEqual(verdictOf(budgetChecks(budgets, { ...latency, GetSessionHistoryOutline: { p95_ms: 201 } }, 0)).red, ['budget_outline_p95_ms'])
  assert.deepEqual(verdictOf(budgetChecks(budgets, { GetSessionState: { p95_ms: 20 }, AttachSessionView: { p95_ms: 60 } }, 0)).red, ['budget_outline_p95_ms'])
  assert.deepEqual(verdictOf(budgetChecks(budgets, latency, 1)).red, ['budget_stalled_operations'])
})

test('the real budget file names every budget the drill gates', () => {
  const real = JSON.parse(readFileSync(new URL('../perf-budgets/kernel.json', import.meta.url), 'utf8'))
  assert.doesNotThrow(() => budgetChecks(real, latency, 0))
})
