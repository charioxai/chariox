import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance } from "./cli-types.js"
import type { SplitPaneFooterRenderOptions } from "./split-pane-footer-renderer.js"
import { createSplitPaneFooterRenderController } from "./split-pane-footer-render-controller.js"

test("split-pane footer render controller renders current footer projection", () => {
  const rendered: SplitPaneFooterRenderOptions[] = []
  let attached = true
  let workflowActive = false
  let visibleAgents: Array<AgentInstance | null | undefined> = [agent("agent-a")]
  const controller = createSplitPaneFooterRenderController({
    renderer: "renderer" as unknown as SplitPaneFooterRenderOptions["renderer"],
    state: { primary: { parts: {}, taskParts: {}, badgeTexts: [] }, auxiliaries: [] },
    primaryBox: () => "primary" as unknown as SplitPaneFooterRenderOptions["primaryBox"],
    auxiliaryBoxes: () => ["aux"] as unknown as SplitPaneFooterRenderOptions["auxiliaryBoxes"],
    isAttached: () => attached,
    workflowScreenActive: () => workflowActive,
    maxAgentsPerScreen: () => 2,
    visibleAgents: () => visibleAgents,
    metaagentTasks: () => [],
    focusedAgentId: () => "agent-a",
    providerRun: () => null,
    currentProviderSelection: () => ({ model: "default", effort: "" }),
    agentActivityLabels: () => ({ "agent-a": "working" }),
    hasPromptWorkByAgent: () => ({ "agent-a": true }),
    streamingAgentId: () => "agent-a",
    agentBusyLatch: (agentId) => agentId === "agent-a",
    sessionConfigValues: () => ({ sandbox: "workspace-write" }),
    agentLocationLabel: (value) => value?.id ?? null,
    badgeWidth: 8,
    animationFrame: () => 4,
    renderFooters: (options) => {
      rendered.push(options)
    },
  })

  controller.render()
  attached = false
  workflowActive = true
  visibleAgents = []
  controller.render()

  assert.equal(rendered.length, 2)
  const first = rendered[0]!
  assert.equal(first.showAgentFooters, true)
  assert.equal(first.visibleAgents[0]?.id, "agent-a")
  assert.deepEqual(first.metaagentTasks, [])
  assert.equal(first.focusedAgentId, "agent-a")
  assert.deepEqual(first.currentProviderSelection, { model: "default", effort: "" })
  assert.deepEqual(first.agentActivityLabels, { "agent-a": "working" })
  assert.deepEqual(first.hasPromptWorkByAgent, { "agent-a": true })
  assert.equal(first.agentBusyLatch("agent-a"), true)
  assert.deepEqual(first.sessionConfigValues, { sandbox: "workspace-write" })
  assert.equal(first.agentLocationLabel(agent("agent-a")), "agent-a")
  assert.equal(first.badgeWidth, 8)
  assert.equal(first.animationFrame, 4)
  assert.equal(rendered[1]!.showAgentFooters, false)
})

function agent(id: string): AgentInstance {
  return { id } as AgentInstance
}
