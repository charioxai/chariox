import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable } from "@opentui/core"
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
