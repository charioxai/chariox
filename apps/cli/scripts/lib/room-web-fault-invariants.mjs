// MP-08 / MP-10: recovery is more than a reconnected display.
import assert from 'node:assert/strict'
export function controllerFaultAttributed(environment, replay, actions = []) {
  return (environment?.health ?? []).some(item => item.component === 'browser_controller' && (item.state !== 'ready' || item.diagnostic_code))
    // HealthChanged has no component payload. Streamer/browser transitions also
    // produce it, so replay alone cannot identify the failed component.
    || actions.some(action => action.state === 'failed'
      && action.outcome?.code === 'controller_failure')
}
export function assertWebFaultRecovery(before, after) {
  for (const key of ['session_id', 'environment_id']) assert.equal(after.environment[key], before.environment[key], `${key} changed`)
  assert.ok(after.environment.runtime_generation >= before.environment.runtime_generation, 'runtime generation regressed')
  const tabs = value => (value.environment.tabs ?? []).map(tab => tab.tab_id).sort()
  assert.deepEqual(tabs(after), tabs(before), 'stable Tab registry changed')
  const ids = after.page.actions.map(action => action.action_id)
  assert.equal(new Set(ids).size, ids.length, 'duplicate Action identity')
  for (const action of before.page.actions.filter(action => action.state === 'completed')) {
    const retained = after.page.actions.find(item => item.action_id === action.action_id)
    assert.ok(retained, 'completed Action disappeared')
    assert.equal(retained.state, 'completed', 'completed Action was replayed or reopened')
    assert.equal(retained.sequence, action.sequence, 'Action sequence changed')
  }
}

export function assertTuiFaultRecovery(clients, sessionId, attachmentIds) {
  assert.equal(new Set(clients.map(client => client.side)).size, clients.length, 'duplicate TUI observer')
  assert.equal(new Set(clients.map(client => client.attachmentId)).size, clients.length, 'distinct TUIs share an attachment identity')
  for (const client of clients) {
    assert.equal(client.sessionId, sessionId, 'TUI changed Room')
    assert.equal(client.daemonDisconnected, false, 'TUI remains disconnected')
    assert.ok(client.attachmentId && attachmentIds.includes(client.attachmentId), 'TUI attachment is absent from kernel state')
  }
}
