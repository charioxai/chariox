import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './perf-budget-tighten-only.mjs'

const WEB = 'perf-budgets/web.json'
const AGENT = 'apps/kernel/agent_context_budget.json'
const budget = max => ({ max, unit: 'bytes', gate: 'a check', measured: 'a measure' })
const web = budgets => `${JSON.stringify(budgets, null, 2)}\n`
const base = () => ({ [WEB]: web({ initial_js_brotli_bytes: budget(600), reload_long_tasks: budget(0) }), [AGENT]: web({ claude_tools_listed: 12 }) })

test('a tightened budget and a new budget pass', () => {
  const r = makeRepo(base())
  r.write(WEB, web({ initial_js_brotli_bytes: budget(590), reload_long_tasks: budget(0), room_switch_busy_ms: budget(400) }))
  r.commit('tighten')
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a raised web budget is red and names the line that allows it', () => {
  const r = makeRepo(base())
  r.write(WEB, web({ initial_js_brotli_bytes: budget(700), reload_long_tasks: budget(0) }))
  r.commit('grow the bundle')
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /perf-budgets\/web\.json:2: budget web\.initial_js_brotli_bytes is raised from 600 to 700 without a "budget raised: web\.initial_js_brotli_bytes: <why>" line/)
  r.dispose()
})

test('a raise with its "budget raised:" line passes, and the line covers only the budget it names', () => {
  const r = makeRepo(base())
  r.write(WEB, web({ initial_js_brotli_bytes: budget(700), reload_long_tasks: budget(1) }))
  r.commit('grow the bundle\n\nbudget raised: web.initial_js_brotli_bytes: the Files window ships in the first paint')
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.doesNotMatch(result.text, /initial_js_brotli_bytes/)
  assert.match(result.text, /budget web\.reload_long_tasks is raised from 0 to 1/)
  r.write(WEB, web({ initial_js_brotli_bytes: budget(700), reload_long_tasks: budget(0) }))
  r.commit('keep long tasks at zero')
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a removed budget counts as raised', () => {
  const r = makeRepo(base())
  r.write(WEB, web({ initial_js_brotli_bytes: budget(600) }))
  r.commit('drop a budget')
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /budget web\.reload_long_tasks \(0\) is removed/)
  r.dispose()
})

test('the agent context budget only tightens too', () => {
  const r = makeRepo(base())
  r.write(AGENT, web({ claude_tools_listed: 13 }))
  r.commit('add a tool')
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /budget agent_context_budget\.claude_tools_listed is raised from 12 to 13/)
  r.dispose()
})

test('a budget without its unit, gate or measurement is red', () => {
  const r = makeRepo(base())
  r.write('perf-budgets/kernel.json', web({ attach_p95_ms: { max: 50 } }))
  r.commit('add a kernel budget')
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /perf-budgets\/kernel\.json:0: budget attach_p95_ms lacks unit, gate, measured/)
  r.dispose()
})
