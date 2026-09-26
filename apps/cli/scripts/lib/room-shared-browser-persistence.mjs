import assert from "node:assert/strict"

export function assertRoomSharedBrowserPersistence(input) {
  const { before, environment, session, turns, actionHistory, continuation, browserStateEvidence } = input
  const sessionId = before.sessionId
  assert.equal(environment?.session_id, sessionId, "restored Room environment session changed")
  assert.equal(environment?.lifecycle, "ready", "restored Room environment is not ready")
  assert.equal(environment?.environment_id, before.environment.environmentId,
    "restored Room environment identity changed")
  assert.ok(Number.isSafeInteger(environment?.runtime_generation),
    "restored Room runtime generation is missing")
  assert.equal(environment?.focused_tab_id, before.environment.focusedTabId,
    "restored Browser focused tab changed")
  const tabs = environment?.tabs ?? []
  assert.deepEqual(tabs.map(tab => ({
    tabId: tab.tab_id, url: tab.url, title: tab.title, focused: tab.focused,
  })).sort((left, right) => left.tabId.localeCompare(right.tabId)),
  before.environment.tabs.map(({ tabId, url, title, focused }) => ({ tabId, url, title, focused })),
  "restored Browser tab state changed")

  assert.equal(session?.id, sessionId, "restored Room identity changed")
  const agent = session.agents?.find(candidate => candidate.id === before.agent.id)
  assert.ok(agent, "restored provider agent is absent from its Room")
  const agentIdentity = {
    id: agent.id,
    sessionId: agent.session_id,
    provider: agent.provider,
    model: agent.model,
    accountProfile: agent.account_profile ?? "default",
  }
  assert.deepEqual(agentIdentity, before.agent, "restored provider agent or profile changed")
  assert.equal(continuation?.agentId, agent.id, "continued provider action used a different agent")
  assert.equal(continuation?.provider, before.agent.provider)
  assert.equal(continuation?.model, before.agent.model)
  assert.equal(continuation?.accountProfile ?? "default", before.agent.accountProfile)
  assert.equal(continuation?.mode, "browser", "post-restore continuation must use structured Browser")
  assert.equal(continuation?.actionKind, "click")

  const providerTurns = (turns ?? []).filter(turn => turn.lifecycle === "completed"
    && turn.external_provider === before.providerThread.provider
    && typeof turn.external_provider_session_id === "string")
  assert.equal(providerTurns.length, 4,
    "persistence drill requires the original, two same-thread state proofs, and post-restore action turns")
  const originalTurn = providerTurns.find(turn => turn.turn_id === before.providerThread.turnId)
  const beforeStateTurn = providerTurns.find(turn => turn.turn_id === browserStateEvidence?.beforeSave?.turnId)
  const afterStateTurn = providerTurns.find(turn => turn.turn_id === browserStateEvidence?.afterRestore?.turnId)
  const continuedTurn = providerTurns.find(turn => turn.turn_id === continuation.settlement?.turnId)
  assert.ok(originalTurn, "original provider turn was not retained after restore")
  assert.ok(beforeStateTurn, "pre-save browser-state proof was not retained after restore")
  assert.ok(afterStateTurn, "post-restore browser-state proof did not complete")
  assert.ok(continuedTurn, "post-restore provider turn did not complete")
  assert.equal(new Set([originalTurn.turn_id, beforeStateTurn.turn_id, afterStateTurn.turn_id,
    continuedTurn.turn_id]).size, 4, "each persistence proof must use its own provider turn")
  assert.notEqual(beforeStateTurn.turn_id, originalTurn.turn_id,
    "pre-save state proof must use a new turn on the same provider thread")
  assert.notEqual(afterStateTurn.turn_id, beforeStateTurn.turn_id,
    "post-restore state proof must use a new turn on the same provider thread")
  assert.notEqual(continuedTurn.turn_id, afterStateTurn.turn_id,
    "post-restore action must use a new turn after the state proof")
  for (const turn of [originalTurn, beforeStateTurn, afterStateTurn, continuedTurn]) {
    assert.equal(turn.external_provider_session_id, before.providerThread.providerSessionId,
      "provider thread identity changed across restore")
  }
  assert.equal(continuedTurn.prompt_id, continuation.settlement.promptId,
    "post-restore provider history does not match the submitted prompt")

  const history = actionHistory ?? []
  const priorActions = before.actions.map(action => {
    const matches = history.filter(candidate => candidate.action_id === action.actionId)
    assert.equal(matches.length, 1, `saved ${action.mode} action history was not retained exactly once`)
    return actionIdentity(matches[0])
  })
  assert.deepEqual(priorActions, before.actions, "pre-save Browser/Computer action history changed")

  const action = history.find(candidate => candidate.action_id === continuation.actionId)
  assert.ok(action, "post-restore Browser action is absent from kernel history")
  assert.equal(history.filter(candidate => candidate.action_id === continuation.actionId).length, 1,
    "post-restore Browser action must appear exactly once")
  assert.equal(action.actor_id, `agent:${agent.id}`)
  assert.equal(action.sequence, continuation.actionSequence)
  assert.ok(action.sequence > Math.max(...before.actions.map(item => item.sequence)),
    "post-restore Browser action must follow preserved Room action history")
  assert.equal(action.runtime_generation, environment.runtime_generation)
  assert.equal(action.mode, "browser")
  assert.equal(action.kind, "click")
  assert.equal(action.state, "completed")
  assert.deepEqual(action.targets, [{ kind: "browser_tab", id: before.environment.focusedTabId }])
  const previousSequence = Math.max(...before.actions.map(item => item.sequence))
  const continuedBrowserActions = history.filter(candidate => candidate.actor_id === `agent:${agent.id}`
    && candidate.sequence > previousSequence && candidate.mode === "browser" && candidate.kind === "click"
    && candidate.targets?.some(target => target.kind === "browser_tab"
      && target.id === before.environment.focusedTabId))
  assert.equal(continuedBrowserActions.length, 1,
    "post-restore continuation must mutate the focused Browser tab exactly once")
  assert.equal(continuedBrowserActions[0].action_id, continuation.actionId)
  return true
}

export function roomSharedBrowserClickCount(text) {
  assert.equal(typeof text, "string", "Browser click effect text must be a string")
  const matches = [...text.matchAll(/POINTER_CLICK_COUNT=(\d+)/g)]
  assert.equal(matches.length, 1, "Browser fixture must expose exactly one physical click count")
  const count = Number(matches[0][1])
  assert.ok(Number.isSafeInteger(count), "Browser physical click count is invalid")
  return count
}

export function assertRoomSharedBrowserClickAdvancedOnce(before, after) {
  assert.ok(Number.isSafeInteger(before) && before >= 0, "pre-continuation Browser click count is invalid")
  assert.ok(Number.isSafeInteger(after) && after >= 0, "post-continuation Browser click count is invalid")
  assert.equal(after, before + 1,
    after === before
      ? "post-restore Browser click did not change the physical fixture state"
      : "post-restore Browser click changed physical fixture state more than once")
  return { before, after, delta: after - before }
}

function actionIdentity(action) {
  return {
    actionId: action.action_id,
    sequence: action.sequence,
    actorId: action.actor_id,
    runtimeGeneration: action.runtime_generation,
    mode: action.mode,
    kind: action.kind,
    state: action.state,
    targets: action.targets,
  }
}
