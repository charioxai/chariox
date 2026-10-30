import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable, type TextRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { createSplitPaneFooterRenderState, renderSplitPaneFooters, type SplitPaneFooterRenderOptions } from "./split-pane-footer-renderer.js"
import { STATUS_BADGE_WIDTH } from "./runtime.js"

test("MP-08/MP-10 footer grows for a waiting deadline and clears it on completion", async () => {
  const harness = await createTestRenderer({ width: 100, height: 8, useThread: false })
  const box = new BoxRenderable(harness.renderer, { width: 100 })
  harness.renderer.root.add(box)
  const agent = { id: "agent-1", name: "agent", state: "Idle", provider: "codex" }
  const session = { id: "session-1", agents: [agent], agent_tasks: [], agent_wakes: [] } as unknown as NonNullable<SplitPaneFooterRenderOptions["session"]>
  const options: SplitPaneFooterRenderOptions = {
    renderer: harness.renderer, state: createSplitPaneFooterRenderState(), primaryBox: box,
    auxiliaryBoxes: [], showAgentFooters: true, maxAgentsPerScreen: 1, session,
    visibleAgents: [agent as unknown as SplitPaneFooterRenderOptions["visibleAgents"][number]],
    metaagentTasks: [], focusedAgentId: "agent-1", providerRun: null,
    currentProviderSelection: { provider: "codex", model: "gpt-6-luna", effort: "low" },
    agentActivityLabels: {}, streamingAgentId: null, agentBusyLatch: () => false,
    sessionConfigValues: {}, agentLocationLabel: () => null, providerAccountLabel: () => "",
    badgeWidth: STATUS_BADGE_WIDTH, animationFrame: 0,
  }
  try {
    renderSplitPaneFooters(options)
    await harness.renderOnce()
    assert.ok(harness.captureCharFrame().includes("IDLE"))
    session.agent_tasks = [{ agent_id: "agent-1", state: "waiting", reason: "timer", obligations: [], wait: { deadline_ms: Date.parse("2026-10-19T22:34:56Z") } }] as unknown as NonNullable<typeof session.agent_tasks>
    renderSplitPaneFooters(options)
    await harness.renderOnce()
    assert.ok(harness.captureCharFrame().includes("WAITING UNTIL 22:34:56Z"), harness.captureCharFrame())
    session.agent_tasks = []
    renderSplitPaneFooters(options)
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    assert.ok(frame.includes("IDLE"))
    assert.ok(!frame.includes("22:34:56Z"))
  } finally {
    harness.renderer.destroy()
  }
})

// MP-08 / MP-10 / MP-11 #971: render current work beside historical goals.
for (const historical of ["done", "cancelled"] as const) {
  for (const current of ["working", "blocked"] as const) {
    test(`split pane renders ${current} after a ${historical} goal in both footer slots`, () => {
      const text = () => ({ content: "", visible: false } as unknown as TextRenderable)
      const slot = () => ({
        parts: { texts: [text()], dividers: [] }, taskParts: {},
        badgeTexts: Array.from({ length: 48 }, text), goalText: text(),
      })
      const primary = slot(), auxiliary = slot()
      const box = { requestRender() {} } as SplitPaneFooterRenderOptions["primaryBox"]
      const agents = ["a", "b"].map(id => ({ id, state: "Idle", provider: "codex", account_profile: "default" }))
      const options = {
        renderer: {}, state: { primary, auxiliaries: [auxiliary] },
        primaryBox: box, auxiliaryBoxes: [box], showAgentFooters: true, maxAgentsPerScreen: 2,
        visibleAgents: agents, session: { id: "room", agents, agent_tasks: agents.flatMap(({ id }) => [
          { task_id: `old-${id}`, agent_id: id, prompt_id: "old", state: historical, revision: 2, obligations: [],
            goal: { id: `goal:old-${id}`, objective: "Previous goal", check_in_minutes: 1,
              active_elapsed_ms: 0, waiting_elapsed_ms: 0, check_count: 1 } },
          { task_id: `new-${id}`, agent_id: id, prompt_id: "new", state: current, revision: 1,
            obligations: [{ status: "open" }], reason: current === "blocked" ? "waiting for owner" : null },
        ]) },
        metaagentTasks: [], focusedAgentId: "a", providerRun: null, currentProviderSelection: { model: "default", effort: "" },
        agentActivityLabels: {}, streamingAgentId: null, agentBusyLatch: () => false,
        sessionConfigValues: {}, agentLocationLabel: () => null, providerAccountLabel: () => "Default",
        badgeWidth: 48, animationFrame: 0,
      } as unknown as SplitPaneFooterRenderOptions
      renderSplitPaneFooters(options)
      for (const footer of [primary, auxiliary]) {
        const badge = footer.badgeTexts.map(cell => cell.content).join("").trim()
        assert.match(badge, new RegExp(`^${current.toUpperCase()}`))
        assert.match(badge, /1 obligations/)
        assert.equal(footer.goalText.visible, true)
        assert.match(String(footer.goalText.content), new RegExp(`GOAL ${historical.toUpperCase()}: Previous goal`))
      }
    })
  }
}

// MP-08 / MP-09 / MP-10 / MP-11 EA2-80: the real footer reads the kernel outcome.
for (const kind of ["interrupted_by_restart", "cancelled", "completed", "other"] as const) {
  test(`EA2-80 footer renders ${kind} without turning cancellation into an error`, async () => {
    const harness = await createTestRenderer({ width: 120, height: 10, useThread: false })
    const box = new BoxRenderable(harness.renderer, { width: 120 })
    harness.renderer.root.add(box)
    const agent = { id: "agent-1", state: "Idle", provider: "codex" }
    const reason = kind === "interrupted_by_restart" ? "The kernel restarted while this turn was running." : ""
    const activity = { status: kind === "interrupted_by_restart" || kind === "other" ? "error" : "idle", busy: false,
      prompt_status: "none", last_completed_turn: { turn_id: "turn-1", outcome: { kind, reason, retryable: true } } }
    const options = {
      renderer: harness.renderer, state: createSplitPaneFooterRenderState(), primaryBox: box,
      auxiliaryBoxes: [], showAgentFooters: true, maxAgentsPerScreen: 1,
      session: { id: "room", agents: [agent], agent_activity: { "agent-1": activity } }, visibleAgents: [agent],
      metaagentTasks: [], focusedAgentId: "agent-1", providerRun: null,
      currentProviderSelection: { provider: "codex", model: "gpt-6-luna", effort: "low" },
      agentActivityLabels: {}, streamingAgentId: null, agentBusyLatch: () => false,
      sessionConfigValues: {}, agentLocationLabel: () => null, providerAccountLabel: () => "",
      badgeWidth: STATUS_BADGE_WIDTH, animationFrame: 0,
    } as unknown as SplitPaneFooterRenderOptions
    try {
      renderSplitPaneFooters(options)
      await harness.renderOnce()
      const frame = harness.captureCharFrame()
      assert.ok(frame.includes(kind + (reason ? ` · ${reason}` : "")), frame)
      assert.equal(frame.includes("/continue"), kind === "interrupted_by_restart" || kind === "other", frame)
      if (kind === "cancelled") { assert.ok(frame.includes("IDLE"), frame); assert.ok(!frame.includes("ERROR"), frame) }
      options.focusedAgentId = "other-agent"
      renderSplitPaneFooters(options)
      await harness.renderOnce()
      assert.ok(!harness.captureCharFrame().includes("/continue"), "a command addresses the focused agent only")
      options.focusedAgentId = "agent-1"
      options.agentBusyLatch = () => true
      renderSplitPaneFooters(options)
      await harness.renderOnce()
      assert.ok(!harness.captureCharFrame().includes("/continue"), "a pending local submission hides Continue")
      options.agentBusyLatch = () => false
      activity.busy = true
      activity.status = "working"
      renderSplitPaneFooters(options)
      await harness.renderOnce()
      assert.ok(!harness.captureCharFrame().includes("/continue"))
    } finally { harness.renderer.destroy() }
  })
}


// MP-08 / MP-09 / MP-10 / MP-11: diagnostics must leave room for the transcript and retry.
test("EA2-80 narrow footer bounds multiline provider errors and keeps Continue visible", async () => {
  const harness = await createTestRenderer({ width: 40, height: 16, useThread: false })
  const box = new BoxRenderable(harness.renderer, { width: 40 })
  harness.renderer.root.add(box)
  const agent = { id: "agent-1", state: "Idle", provider: "codex" }
  const reason = ("Provider diagnostic 漢字 🐛 with extra detail\n".repeat(200)) + "END OF DIAGNOSTIC"
  const outcome = { kind: "other", reason, retryable: true }
  const options = {
    renderer: harness.renderer, state: createSplitPaneFooterRenderState(), primaryBox: box,
    auxiliaryBoxes: [], showAgentFooters: true, maxAgentsPerScreen: 1,
    session: { id: "room", agents: [agent], agent_activity: { "agent-1": {
      busy: false, prompt_status: "none", last_completed_turn: { turn_id: "turn-1", outcome },
    } } }, visibleAgents: [agent], metaagentTasks: [], focusedAgentId: "agent-1", providerRun: null,
    currentProviderSelection: { provider: "codex", model: "gpt-6-luna", effort: "low" },
    agentActivityLabels: {}, streamingAgentId: null, agentBusyLatch: () => false,
    sessionConfigValues: {}, agentLocationLabel: () => null, providerAccountLabel: () => "",
    badgeWidth: STATUS_BADGE_WIDTH, animationFrame: 0,
  } as unknown as SplitPaneFooterRenderOptions
  try {
    renderSplitPaneFooters(options)
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    assert.ok(frame.includes("Continue: /continue"), frame)
    assert.ok(frame.includes("..."), frame)
    assert.equal(options.state.primary.outcomeText!.height, 1, "diagnostic occupies one line")
    assert.ok(!options.state.primary.outcomeText!.plainText.includes("\n"), "collapse diagnostic newlines")
    assert.equal(outcome.reason, reason, "history/state retains the complete diagnostic")
  } finally { harness.renderer.destroy() }
})
