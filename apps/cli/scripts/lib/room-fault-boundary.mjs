// MP-08 / MP-10: a fault result is valid only for the operation it targeted.
import assert from 'node:assert/strict'

export function findRoomFaultAction(before, snapshot, idempotencyKey, state) {
  assert.ok(typeof idempotencyKey === 'string' && idempotencyKey.length > 0, 'fault operation key missing')
  const priorIds = new Set(before.page.actions.map(action => action.action_id))
  const matches = snapshot.page.actions.filter(action => action.idempotency_key === idempotencyKey)
  assert.ok(matches.length <= 1, 'duplicate fault operation key')
  const action = matches[0]
  if (!action) return null
  assert.ok(!priorIds.has(action.action_id), 'fault operation predates this case')
  return action.state === state ? action : null
}

export function assertProviderFaultBacklog({ phase, snapshot, agentId, activePromptId, queuedPromptId,
  queuedOutcome, completedPromptId = null, turns = [], progressObserved = false }) {
  assert.ok(['beginning', 'middle', 'commit'].includes(phase), 'unsupported provider fault boundary')
  assert.equal(queuedOutcome, 'Queued', 'successor started instead of entering backlog')
  assert.ok(activePromptId && queuedPromptId && activePromptId !== queuedPromptId, 'distinct active/backlog identities required')
  const state = snapshot.session.prompt_states?.[agentId]
  assert.equal(state?.active_prompt?.id, activePromptId, 'target prompt is no longer active')
  const pendingIds = (state?.queued_prompts ?? []).map(prompt => prompt.id)
  assert.equal(pendingIds.filter(id => id === queuedPromptId).length, 1, 'accepted backlog is missing or duplicated')
  assert.ok(!pendingIds.includes(activePromptId), 'active prompt also appears in backlog')
  const turn = snapshot.agent_activity?.[agentId]?.active_turn
  assert.equal(turn?.prompt_id, activePromptId, 'active turn differs from the prompt queue')
  assert.ok(turn.provider_run_id, 'active provider authority missing')
  if (phase === 'middle') assert.equal(progressObserved, true, 'middle boundary lacks observed operation progress')
  if (phase === 'commit') {
    assert.ok(completedPromptId && completedPromptId !== activePromptId && completedPromptId !== queuedPromptId,
      'commit needs a completed predecessor and a separate active backlog gate')
    assert.equal(turns.filter(turn => turn.prompt_id === completedPromptId && turn.lifecycle === 'completed').length, 1,
      'committed predecessor missing or duplicated')
  }
  // Keep the receipt value-free: prompts, provider payloads and auth stay private.
  return { phase, activePromptId, queuedPromptId, pendingIds, providerRunId: turn.provider_run_id,
    completedPromptId, progressObserved }
}
