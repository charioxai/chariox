// MP-08 / MP-10: reject superficially connected recovery with lost state.
import assert from 'node:assert/strict'
import test from 'node:test'
import { assertWebFaultRecovery, controllerFaultAttributed } from './room-web-fault-invariants.mjs'
const before = { environment: { session_id: 'room', environment_id: 'env', runtime_generation: 1, tabs: [{ tab_id: 'tab' }] }, page: { actions: [{ action_id: 'a', state: 'completed', sequence: 1 }] } }
test('recovery requires same Room, Tabs and completed Action ledger', () => {
  assertWebFaultRecovery(before, structuredClone(before))
  for (const edit of [v => v.environment.environment_id = 'other', v => v.environment.tabs = [], v => v.page.actions = [], v => v.page.actions.push(v.page.actions[0]), v => v.page.actions[0].state = 'running']) {
    const value = structuredClone(before); edit(value)
    assert.throws(() => assertWebFaultRecovery(before, value))
  }
})
test('a controller component name alone is not evidence of fault attribution', () => {
  const healthy = { health: [{ component: 'browser_controller', state: 'ready', diagnostic_code: null }] }
  assert.equal(controllerFaultAttributed(healthy, { Events: { events: [] } }), false)
  assert.equal(controllerFaultAttributed(healthy, { Events: { events: [{ kind: 'HealthChanged' }] } }), true)
  assert.equal(controllerFaultAttributed({ health: [{ component: 'browser_controller', state: 'failed' }] }), true)
})
