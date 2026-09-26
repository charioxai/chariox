import assert from "node:assert/strict"
import test from "node:test"

import { runRoomSharedBrowserPersistence } from "./room-shared-browser-persistence-runner.mjs"

test("persistence runner orders product save/removal/restore before a fresh same-thread Browser action", async () => {
  const input = makeInput()
  const result = await runRoomSharedBrowserPersistence(input)
  assert.equal(result.sliceSaveRecreatePersistence, "passed")
  assert.equal(result.roomKernelRestartPersistence, "not_run")
  assert.equal(result.drillACompleteBrowserStatePersistence, "passed")
  assert.deepEqual(input.events, [
    "browser-state:before-save", "room-before",
    "save:shutdown:this_slice", "verify-saved-artifacts", "stop", "resources-present", "remove",
    "resources-absent", "restore", "slice-running", "runtime", "browser-ready", "environment-ready",
    "tui-status", "browser-state:after-restore", "room-restored", "physical-before",
    "provider-continuation:POINTER_CLICK_COUNT=3",
    "physical-after", "physical-stable", "tui-action", "physical-stable", "room-after",
  ])
})

test("persistence runner will not restore until owned container and volume are absent", async () => {
  const input = makeInput({ resourcesAbsent: async () => { throw new Error("owned resources remain") } })
  await assert.rejects(() => runRoomSharedBrowserPersistence(input), /owned resources remain/)
  assert.equal(input.events.includes("restore"), false)
})

for (const [label, clickCount, message] of [
  ["unchanged", 2, /did not change/],
  ["duplicate", 4, /more than once/],
]) {
  test(`persistence runner rejects ${label} physical Browser mutation`, async () => {
    const input = makeInput({ clickCountAfter: clickCount })
    await assert.rejects(() => runRoomSharedBrowserPersistence(input), message)
    assert.equal(input.events.includes("room-after"), false)
  })
}

function makeInput(overrides = {}) {
  const events = []
  const before = {
    sessionId: "room-1",
    environment: {
      environmentId: "environment-1", runtimeGeneration: 8, focusedTabId: "tab-1",
      tabs: [{ tabId: "tab-1", url: "http://fixture/click", title: "Click fixture", focused: true }],
    },
    agent: { id: "agent-1", sessionId: "room-1", provider: "codex", model: "gpt-6-luna", accountProfile: "profile-1" },
    providerThread: {
      agentId: "agent-1", provider: "codex", turnId: "turn-1", providerSessionId: "thread-1",
      providerTurnId: "provider-turn-1", lifecycle: "completed",
    },
    actions: [
      { actionId: "browser-before", sequence: 1, actorId: "agent:agent-1", runtimeGeneration: 8,
        mode: "browser", kind: "click", state: "completed", targets: [{ kind: "browser_tab", id: "tab-1" }] },
      { actionId: "computer-before", sequence: 2, actorId: "user:web", runtimeGeneration: 8,
        mode: "computer", kind: "pointer_click", state: "completed", targets: [{ kind: "desktop" }] },
    ],
  }
  const browserBefore = {
    action_id: "browser-before", sequence: 1, actor_id: "agent:agent-1", runtime_generation: 8,
    mode: "browser", kind: "click", state: "completed", targets: [{ kind: "browser_tab", id: "tab-1" }],
  }
  const computerBefore = {
    action_id: "computer-before", sequence: 2, actor_id: "user:web", runtime_generation: 8,
    mode: "computer", kind: "pointer_click", state: "completed", targets: [{ kind: "desktop" }],
  }
  const continuation = {
    agentId: "agent-1", provider: "codex", model: "gpt-6-luna", accountProfile: "profile-1",
    actionId: "browser-after", actionSequence: 3, mode: "browser", actionKind: "click",
    settlement: { turnId: "turn-2", promptId: "prompt-2", lifecycle: "completed", agentIdle: true },
  }
  const roomBefore = {
    environment: {
      session_id: "room-1", environment_id: "environment-1", runtime_generation: 9,
      lifecycle: "ready", focused_tab_id: "tab-1",
      tabs: [{ tab_id: "tab-1", url: "http://fixture/click", title: "Click fixture", focused: true }],
    },
    session: {
      id: "room-1", agents: [{ id: "agent-1", session_id: "room-1", provider: "codex",
        model: "gpt-6-luna", account_profile: "profile-1" }],
    },
    turns: [
      { turn_id: "turn-1", lifecycle: "completed", external_provider: "codex",
        external_provider_session_id: "thread-1", external_provider_turn_id: "provider-turn-1" },
      { turn_id: "state-turn-before", prompt_id: "state-prompt-before", lifecycle: "completed",
        external_provider: "codex", external_provider_session_id: "thread-1" },
    ],
    actionHistory: [browserBefore, computerBefore],
  }
  const roomRestored = {
    ...roomBefore,
    turns: [...roomBefore.turns, { turn_id: "state-turn-after", prompt_id: "state-prompt-after",
      lifecycle: "completed", external_provider: "codex", external_provider_session_id: "thread-1" }],
  }
  const roomAfter = {
    ...roomRestored,
    turns: [...roomRestored.turns, { turn_id: "turn-2", prompt_id: "prompt-2", lifecycle: "completed",
      external_provider: "codex", external_provider_session_id: "thread-1", external_provider_turn_id: "provider-turn-2" }],
    actionHistory: [...roomBefore.actionHistory, {
      action_id: "browser-after", sequence: 3, actor_id: "agent:agent-1", runtime_generation: 9,
      mode: "browser", kind: "click", state: "completed", targets: [{ kind: "browser_tab", id: "tab-1" }],
    }],
  }
  const savedState = {
    id: "saved-1", source_slice_id: "slice-1", image_ref: "image-1",
    home_archive_path: "/saved/home.tgz", manifest_path: "/saved/manifest.json",
  }
  const slice = (status) => ({ id: "slice-1", status, saved_state_ref: "saved-1", saved_state_status: "saved" })
  const dependencies = {
    async saveSliceState(request) {
      events.push(`save:${request.mode}:${request.scope}`)
      assert.equal(request.sliceId, "slice-1")
      return { slice: slice("stopped"), state: savedState }
    },
    async verifySavedStateArtifacts() { events.push("verify-saved-artifacts") },
    async stopSlice() { events.push("stop"); return { slice: slice("stopped") } },
    async assertOwnedResourcesPresent() { events.push("resources-present") },
    async removeOwnedResources() { events.push("remove") },
    async assertOwnedResourcesAbsent() {
      events.push("resources-absent")
      await overrides.resourcesAbsent?.()
    },
    async restoreSlice() { events.push("restore"); return { slice: slice("starting") } },
    async waitForSliceRunning() { events.push("slice-running"); return slice("running") },
    async inspectRuntime() {
      events.push("runtime")
      return { runtimeSourceRevision: "source-1", installedRuntimeSourceRevision: "source-1",
        relayPeerProtocolVersion: "4", workerKernelSha256: "worker-sha" }
    },
    async waitForBrowserReady() { events.push("browser-ready"); return { url: "http://fixture/click" } },
    async waitForEnvironmentReady() { events.push("environment-ready"); return { environment_id: "environment-1", focused_tab_id: "tab-1" } },
    async waitForTuiStatus() { events.push("tui-status"); return { direct: { id: "local-status" }, relay: { id: "remote-status" } } },
    async readRoomState() {
      const final = events.includes("provider-continuation:POINTER_CLICK_COUNT=3")
      const restored = events.includes("browser-state:after-restore")
      events.push(final ? "room-after" : restored ? "room-restored" : "room-before")
      return final ? roomAfter : restored ? roomRestored : roomBefore
    },
    async verifyBrowserState({ phase }) {
      events.push(`browser-state:${phase}`)
      return {
        phase, agentId: "agent-1", providerSessionId: "thread-1",
        turnId: phase === "before-save" ? "state-turn-before" : "state-turn-after",
        browserMarkers: { cookie: "PASS", auth: "PASS", localStorage: "PASS", indexedDB: "PASS",
          cacheStorage: "PASS", serviceWorker: "PASS" },
        rendererSandbox: { namespace: "active", seccompBpf: "active" },
        graphicalProgram: { commandCompleted: true, visible: true, retained: phase === "after-restore" },
      }
    },
    async readBrowserClickCount() {
      const count = events.includes("physical-after") ? overrides.clickCountAfter ?? 3 : 2
      events.push(events.includes("physical-after") ? "physical-stable" : "physical-before")
      return count
    },
    async continueProviderAction(input) {
      events.push(`provider-continuation:${input.expectedPhysicalEffect}`)
      assert.equal(input.agent.id, "agent-1")
      return continuation
    },
    async waitForBrowserClickCountAfter() {
      events.push("physical-after")
      return overrides.clickCountAfter ?? 3
    },
    async readStableBrowserClickCount() {
      events.push("physical-stable")
      return overrides.clickCountAfter ?? 3
    },
    async waitForTuiActionProjection() {
      events.push("tui-action")
      return { direct: { id: "local-action" }, relay: { id: "remote-action" } }
    },
  }
  return {
    before,
    sliceId: "slice-1",
    providerAgentId: "agent-1",
    sourceRuntime: { runtimeSourceRevision: "source-1", protocolVersions: { relayPeer: 4 } },
    dependencies,
    events,
  }
}
