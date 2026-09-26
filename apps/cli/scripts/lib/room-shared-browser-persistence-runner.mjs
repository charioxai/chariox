import assert from "node:assert/strict"

import {
  assertRoomSharedBrowserClickAdvancedOnce,
  assertRoomSharedBrowserPersistence,
} from "./room-shared-browser-persistence.mjs"

const dependencies = [
  "saveSliceState", "verifySavedStateArtifacts", "stopSlice", "assertOwnedResourcesPresent",
  "removeOwnedResources", "assertOwnedResourcesAbsent", "restoreSlice", "waitForSliceRunning",
  "inspectRuntime", "waitForBrowserReady", "waitForEnvironmentReady", "waitForTuiStatus",
  "readRoomState", "verifyBrowserState", "readBrowserClickCount", "continueProviderAction",
  "waitForBrowserClickCountAfter", "readStableBrowserClickCount", "waitForTuiActionProjection",
]

export async function runRoomSharedBrowserPersistence(input) {
  const { before, sliceId, providerAgentId, sourceRuntime, dependencies: adapters } = input
  assert.ok(typeof sliceId === "string" && sliceId.length > 0, "persistence runner requires its slice identity")
  assert.ok(typeof providerAgentId === "string" && providerAgentId === before.providerThread.agentId,
    "persistence runner requires the retained provider agent identity")
  for (const name of dependencies) {
    assert.equal(typeof adapters?.[name], "function", `persistence runner requires ${name}`)
  }

  const browserStateBeforeSave = await adapters.verifyBrowserState({ phase: "before-save" })
  assertRoomBrowserStateProbe(browserStateBeforeSave, "before-save", before, providerAgentId)
  const preSaveState = await adapters.readRoomState(providerAgentId)
  assertRetainedProviderBaseline(before, preSaveState, providerAgentId)
  before.actions = (preSaveState.actionHistory ?? []).map(actionIdentity)

  const saved = await adapters.saveSliceState({ sliceId, mode: "shutdown", scope: "this_slice" })
  assert.equal(saved?.slice?.id, sliceId, "kernel saved a different slice")
  assert.equal(saved.slice.status, "stopped", "SaveSliceState(shutdown) did not stop the slice")
  assert.ok(saved.state?.id && saved.state.source_slice_id === sliceId
    && saved.state.image_ref && saved.state.home_archive_path && saved.state.manifest_path,
  "kernel did not return a complete saved image and home archive")
  assert.equal(saved.slice.saved_state_ref, saved.state.id, "saved slice did not retain its state reference")
  assert.equal(saved.slice.saved_state_status, "saved", "kernel did not mark the saved state active")
  await adapters.verifySavedStateArtifacts(saved.state)

  const stopped = await adapters.stopSlice(sliceId)
  assert.equal(stopped?.slice?.id, sliceId)
  assert.equal(stopped.slice.status, "stopped")
  assert.equal(stopped.slice.saved_state_ref, saved.state.id,
    "StopSlice discarded the saved-state reference")
  assert.equal(stopped.slice.saved_state_status, "saved",
    "StopSlice discarded the saved-state status")
  await adapters.assertOwnedResourcesPresent()
  await adapters.removeOwnedResources()
  await adapters.assertOwnedResourcesAbsent()

  const restored = await adapters.restoreSlice(sliceId)
  assert.equal(restored?.slice?.id, sliceId, "kernel restored a different slice")
  assert.equal(restored.slice.saved_state_ref, saved.state.id,
    "StartSlice did not retain the saved-state reference")
  const slice = await adapters.waitForSliceRunning(sliceId)
  assert.equal(slice?.id, sliceId, "restored slice identity changed")
  assert.equal(slice.status, "running", "restored slice did not reach running")
  assert.equal(slice.saved_state_ref, saved.state.id,
    "running slice lost its saved-state reference")
  const runtime = await adapters.inspectRuntime()
  assert.equal(runtime.runtimeSourceRevision, sourceRuntime.runtimeSourceRevision,
    "restored slice image must retain exact runtime source")
  assert.equal(runtime.installedRuntimeSourceRevision, sourceRuntime.runtimeSourceRevision,
    "restored worker must install exact runtime source")
  assert.equal(runtime.relayPeerProtocolVersion, String(sourceRuntime.protocolVersions.relayPeer),
    "restored slice relay protocol must match current source")

  const browser = await adapters.waitForBrowserReady()
  const expectedTab = before.environment.tabs.find(tab => tab.tabId === before.environment.focusedTabId)
  assert.ok(expectedTab, "pre-save focused tab identity is missing")
  assert.equal(browser?.url, expectedTab.url, "restored Chromium did not reopen the saved Room tab")
  const environment = await adapters.waitForEnvironmentReady()
  assert.equal(environment.environment_id, before.environment.environmentId,
    "Room kernel did not recover the original Browser Environment")
  assert.equal(environment.focused_tab_id, before.environment.focusedTabId,
    "Room kernel did not recover the original focused Browser tab")
  const status = await adapters.waitForTuiStatus(before)

  const browserStateAfterRestore = await adapters.verifyBrowserState({ phase: "after-restore" })
  assertRoomBrowserStateProbe(browserStateAfterRestore, "after-restore", before, providerAgentId)

  const restoredState = await adapters.readRoomState(providerAgentId)
  assertRetainedProviderBaseline(before, restoredState, providerAgentId)
  const clickCountBefore = await adapters.readBrowserClickCount()
  assert.ok(Number.isSafeInteger(clickCountBefore) && clickCountBefore >= 0,
    "restored Browser physical click counter is unavailable")
  const continuation = await adapters.continueProviderAction({
    agent: restoredState.session.agents.find(agent => agent.id === providerAgentId),
    expectedPhysicalEffect: `POINTER_CLICK_COUNT=${clickCountBefore + 1}`,
  })
  const clickCountAfter = await adapters.waitForBrowserClickCountAfter(clickCountBefore)
  const physicalEffect = assertRoomSharedBrowserClickAdvancedOnce(clickCountBefore, clickCountAfter)
  const stableClickCount = await adapters.readStableBrowserClickCount()
  assert.deepEqual(assertRoomSharedBrowserClickAdvancedOnce(clickCountBefore, stableClickCount), physicalEffect,
    "post-restore Browser physical effect did not remain exactly one click")
  const actionProjection = await adapters.waitForTuiActionProjection(continuation)
  assert.deepEqual(assertRoomSharedBrowserClickAdvancedOnce(
    clickCountBefore, await adapters.readStableBrowserClickCount(),
  ), physicalEffect, "post-restore Browser click count changed after TUI projection")

  const after = await adapters.readRoomState(providerAgentId)
  const browserStateEvidence = { beforeSave: browserStateBeforeSave, afterRestore: browserStateAfterRestore }
  assertRoomSharedBrowserPersistence({ ...after, before, continuation, browserStateEvidence })
  return {
    status: "passed",
    sliceSaveRecreatePersistence: "passed",
    roomKernelRestartPersistence: "not_run",
    drillACompleteBrowserStatePersistence: "passed",
    save: { command: "SaveSliceState(shutdown, this_slice)", stateId: saved.state.id, sourceSliceId: saved.state.source_slice_id },
    stop: { command: "StopSlice", sliceId: stopped.slice.id, status: stopped.slice.status },
    removal: { containerRemoved: true, homeVolumeRemoved: true },
    restore: {
      command: "StartSlice(saved_state)", sliceId: slice.id, status: slice.status,
      environmentId: after.environment.environment_id, focusedTabId: after.environment.focused_tab_id,
      browserUrl: browser.url, workerKernelSha256: runtime.workerKernelSha256,
    },
    physicalEffect,
    browserStateEvidence,
    directTui: { statusNoticeId: status.direct.id, actionNoticeId: actionProjection.direct.id },
    relayTui: { statusNoticeId: status.relay.id, actionNoticeId: actionProjection.relay.id },
    continuation: {
      agentId: continuation.agentId, provider: continuation.provider, model: continuation.model,
      accountProfile: continuation.accountProfile, actionId: continuation.actionId,
      actionSequence: continuation.actionSequence, turnId: continuation.settlement.turnId,
      providerThreadId: after.turns.find(turn => turn.turn_id === continuation.settlement.turnId)
        ?.external_provider_session_id,
    },
    rendererSandboxEvidence: {
      beforeSave: browserStateBeforeSave.rendererSandbox,
      afterRestore: browserStateAfterRestore.rendererSandbox,
    },
  }
}

function assertRoomBrowserStateProbe(evidence, phase, before, providerAgentId) {
  assert.equal(evidence?.phase, phase, `Browser state evidence phase must be ${phase}`)
  assert.equal(evidence?.agentId, providerAgentId, "Browser state evidence came from a different Room agent")
  assert.equal(evidence?.providerSessionId, before.providerThread.providerSessionId,
    "Browser state evidence changed provider thread")
  assert.ok(typeof evidence?.turnId === "string" && evidence.turnId.length > 0,
    "Browser state evidence requires an actual provider turn")
  assert.equal(Object.keys(evidence.browserMarkers ?? {}).length, 6,
    "Browser state evidence omitted a persisted API marker")
  assert.ok(Object.values(evidence.browserMarkers).every(marker => marker === "PASS"),
    "Browser state evidence reported a missing or changed API marker")
  assert.equal(evidence.rendererSandbox?.namespace, "active",
    "Browser state evidence did not prove the active renderer namespace sandbox")
  assert.equal(evidence.rendererSandbox?.seccompBpf, "active",
    "Browser state evidence did not prove the active renderer Seccomp-BPF sandbox")
  assert.equal(evidence.graphicalProgram?.commandCompleted, true,
    "slice provider command did not verify the graphical program")
  assert.equal(evidence.graphicalProgram?.visible, true,
    "the graphical program was not observed on the shared desktop")
  if (phase === "after-restore") assert.equal(evidence.graphicalProgram?.retained, true,
    "the graphical program did not survive saved-state restoration")
}

function assertRetainedProviderBaseline(before, state, providerAgentId) {
  assert.equal(state?.session?.id, before.sessionId, "saved Room identity changed before continuation")
  const agent = state.session.agents?.find(candidate => candidate.id === providerAgentId)
  assert.ok(agent, "saved Room no longer contains its provider agent")
  assert.deepEqual({
    id: agent.id, sessionId: agent.session_id, provider: agent.provider, model: agent.model,
    accountProfile: agent.account_profile ?? "default",
  }, before.agent, "saved Room provider identity changed before continuation")
  const originalTurn = state.turns?.find(turn => turn.turn_id === before.providerThread.turnId)
  assert.ok(originalTurn?.lifecycle === "completed"
    && originalTurn.external_provider === before.providerThread.provider
    && originalTurn.external_provider_session_id === before.providerThread.providerSessionId,
  "saved provider thread history was not retained before continuation")
  for (const action of before.actions) {
    const matches = state.actionHistory?.filter(candidate => candidate.action_id === action.actionId) ?? []
    assert.equal(matches.length, 1, `saved ${action.mode} action history was not retained before continuation`)
    assert.deepEqual(actionIdentity(matches[0]), action,
      `saved ${action.mode} action history changed before continuation`)
  }
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
