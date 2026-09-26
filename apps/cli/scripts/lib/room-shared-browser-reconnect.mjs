import assert from "node:assert/strict"

export function roomSharedBrowserReconnectSnapshot(input) {
  const {
    sessionId,
    environment,
    session,
    providerAgentId,
    providerEvidence,
    computerActionId,
    computerActorId,
    provider,
    model,
    accountProfile,
  } = input

  assert.equal(environment?.session_id, sessionId, "reconnected Room environment session changed")
  assert.equal(environment?.lifecycle, "ready", "reconnected Room environment is not ready")
  assert.ok(typeof environment.environment_id === "string" && environment.environment_id,
    "reconnected Room environment identity is missing")
  assert.ok(Number.isSafeInteger(environment.runtime_generation),
    "reconnected Room runtime generation is missing")
  assert.ok(typeof environment.focused_tab_id === "string" && environment.focused_tab_id,
    "reconnected Room has no focused Browser tab")
  const focusedTab = environment.tabs?.find(tab => tab.tab_id === environment.focused_tab_id)
  assert.ok(focusedTab, "reconnected Room focused Browser tab is absent from its tab list")

  assert.equal(session?.id, sessionId, "reconnected provider session identity changed")
  const agent = session.agents?.find(candidate => candidate.id === providerAgentId)
  assert.ok(agent, "reconnected provider agent is absent from its Room")
  assert.equal(agent.session_id, sessionId)
  assert.equal(agent.provider, provider, "reconnected provider changed")
  assert.equal(agent.model, model, "reconnected provider model changed")
  assert.equal(agent.account_profile ?? "default", accountProfile ?? "default",
    "reconnected provider account profile changed")
  assert.equal(providerEvidence?.agentId, agent.id, "Web Browser action agent identity changed")
  assert.equal(providerEvidence?.provider, agent.provider)
  assert.equal(providerEvidence?.model, agent.model)
  assert.equal(providerEvidence?.accountProfile ?? "default", agent.account_profile ?? "default")
  assert.equal(providerEvidence?.mode, "browser", "shared Browser reconnect case requires a structured Browser action")
  assert.equal(providerEvidence?.actorId, `agent:${agent.id}`)

  const providerTurns = input.turns?.filter(turn => turn.lifecycle === "completed"
    && turn.external_provider === provider
    && typeof turn.external_provider_session_id === "string"
    && turn.external_provider_session_id.length > 0) ?? []
  assert.equal(providerTurns.length, 1,
    "shared Browser reconnect requires exactly one completed provider turn with a thread identity")
  const providerTurn = providerTurns[0]

  const history = input.actionHistory ?? []
  const providerActions = history.filter(action => action.action_id === providerEvidence.actionId)
  const computerActions = history.filter(action => action.action_id === computerActionId)
  assert.equal(providerActions.length, 1, "structured Browser action must appear exactly once in kernel history")
  assert.equal(computerActions.length, 1, "Web Computer takeover action must appear exactly once in kernel history")
  const browserAction = providerActions[0]
  const computerAction = computerActions[0]
  assert.equal(browserAction.actor_id, `agent:${agent.id}`)
  assert.equal(browserAction.mode, "browser")
  assert.equal(browserAction.kind, "click")
  assert.equal(browserAction.state, "completed")
  assert.equal(browserAction.runtime_generation, environment.runtime_generation)
  assert.equal(browserAction.targets?.length, 1)
  assert.deepEqual(browserAction.targets?.[0], { kind: "browser_tab", id: environment.focused_tab_id })
  assert.equal(computerAction.actor_id, computerActorId)
  assert.match(computerAction.actor_id, /^user:.+$/, "Web takeover action must remain human attributed")
  assert.equal(computerAction.mode, "computer")
  assert.equal(computerAction.kind, "pointer_click")
  assert.equal(computerAction.state, "completed")
  assert.equal(computerAction.runtime_generation, environment.runtime_generation)
  assert.ok(computerAction.targets?.some(target => target.kind === "desktop"),
    "Web Computer takeover action must target the shared desktop")
  assert.ok(browserAction.sequence < computerAction.sequence,
    "human Computer takeover must follow the structured Browser action")

  return {
    sessionId,
    environment: {
      environmentId: environment.environment_id,
      runtimeGeneration: environment.runtime_generation,
      lifecycle: environment.lifecycle,
      focusedTabId: environment.focused_tab_id,
      tabs: environment.tabs.map(tab => ({
        tabId: tab.tab_id,
        url: tab.url,
        title: tab.title,
        documentRevision: tab.document_revision,
        focused: tab.focused,
      })).sort((left, right) => left.tabId.localeCompare(right.tabId)),
    },
    agent: {
      id: agent.id,
      sessionId: agent.session_id,
      provider: agent.provider,
      model: agent.model,
      accountProfile: agent.account_profile ?? "default",
    },
    providerThread: {
      agentId: agent.id,
      provider: providerTurn.external_provider,
      turnId: providerTurn.turn_id,
      providerSessionId: providerTurn.external_provider_session_id,
      providerTurnId: providerTurn.external_provider_turn_id ?? null,
      lifecycle: providerTurn.lifecycle,
    },
    actions: [browserAction, computerAction].map(action => ({
      actionId: action.action_id,
      sequence: action.sequence,
      actorId: action.actor_id,
      runtimeGeneration: action.runtime_generation,
      mode: action.mode,
      kind: action.kind,
      state: action.state,
      targets: action.targets,
    })),
  }
}

export function assertRoomSharedBrowserReconnectPreserved(before, after) {
  assert.equal(after.sessionId, before.sessionId, "Room identity changed across relay reconnect")
  assert.deepEqual(after.environment, before.environment, "Browser Environment or tab state changed across relay reconnect")
  assert.deepEqual(after.agent, before.agent, "provider agent or account profile changed across relay reconnect")
  assert.deepEqual(after.providerThread, before.providerThread, "provider thread/history identity changed across relay reconnect")
  assert.deepEqual(after.actions, before.actions, "Browser/Computer action history changed across relay reconnect")
  return true
}

export function roomSharedBrowserStatusNotice(entries, baselineIds, snapshot) {
  const baseline = new Set(baselineIds)
  const environmentId = snapshot.environment.environmentId
  const tabId = snapshot.environment.focusedTabId
  return entries.find(entry => !baseline.has(entry.id)
    && entry.text.startsWith(`Room environment ${environmentId}\n`)
    && entry.text.includes("lifecycle=ready")
    && entry.text.includes(`tab=${tabId} `)) ?? null
}
