// MP-08 / MP-10: fail closed on unrelated Actions and invalid backlog gates.
import assert from 'node:assert/strict'
import test from 'node:test'
import { assertProviderFaultBacklog, findRoomFaultAction } from './room-fault-boundary.mjs'

test('MP-08 / MP-10 operation boundary ignores unrelated work and rejects duplicate/reused keys', () => {
  const before = { page: { actions: [{ action_id: 'old', idempotency_key: 'old-key' }] } }
  const own = { action_id: 'own', idempotency_key: 'case-key', state: 'running' }
  const snapshot = { page: { actions: [{ action_id: 'foreign', idempotency_key: 'foreign-key', state: 'completed' }, own] } }
  assert.equal(findRoomFaultAction(before, snapshot, 'case-key', 'completed'), null)
  assert.equal(findRoomFaultAction(before, snapshot, 'case-key', 'running'), own)
  assert.throws(() => findRoomFaultAction(before, { page: { actions: [own, own] } }, 'case-key', 'running'), /duplicate/)
  assert.throws(() => findRoomFaultAction(before, before, 'old-key', undefined), /predates/)
})

function fixture(phase = 'beginning') {
  return { phase, agentId: 'agent', activePromptId: 'active', queuedPromptId: 'queued', queuedOutcome: 'Queued',
    snapshot: { session: {
      prompt_states: { agent: { active_prompt: { id: 'active' }, queued_prompts: [{ id: 'queued' }] } } },
      agent_activity: { agent: { active_turn: { prompt_id: 'active', provider_run_id: 'run' } } } },
    progressObserved: true, completedPromptId: 'committed', turns: [{ prompt_id: 'committed', lifecycle: 'completed' }] }
}
test('MP-08 / MP-10 beginning, middle and commit require the same active/backlog authority', () => {
  for (const phase of ['beginning', 'middle', 'commit']) {
    assert.equal(assertProviderFaultBacklog(fixture(phase)).providerRunId, 'run')
    for (const mutate of [
      value => value.queuedOutcome = 'Started',
      value => value.snapshot.session.prompt_states.agent.active_prompt.id = 'queued',
      value => value.snapshot.session.prompt_states.agent.queued_prompts = [],
      value => value.snapshot.session.prompt_states.agent.queued_prompts.push({ id: 'queued' }),
      value => value.snapshot.agent_activity.agent.active_turn.provider_run_id = null,
      value => value.snapshot.agent_activity.agent.active_turn.prompt_id = 'foreign',
    ]) {
      const value = fixture(phase); mutate(value)
      assert.throws(() => assertProviderFaultBacklog(value))
    }
  }
})
test('MP-08 / MP-10 progress and committed history cannot be inferred from a delay', () => {
  assert.throws(() => assertProviderFaultBacklog({ ...fixture('middle'), progressObserved: false }), /progress/)
  assert.throws(() => assertProviderFaultBacklog({ ...fixture('commit'), completedPromptId: 'active' }), /separate/)
  assert.throws(() => assertProviderFaultBacklog({ ...fixture('commit'), turns: [] }), /predecessor/)
})
