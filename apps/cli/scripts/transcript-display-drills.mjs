import assert from "node:assert/strict"
import path from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")

const displayUrl = pathToFileURL(path.join(cliRoot, "dist", "transcript-display.js")).href
const responsePanesUrl = pathToFileURL(path.join(cliRoot, "dist", "response-panes.js")).href
const historyViewportUrl = pathToFileURL(path.join(cliRoot, "dist", "history-viewport.js")).href

const { applyTranscriptDisplayState, setTranscriptBlobCollapsed } = await import(displayUrl)
const { selectResponsePaneAgents } = await import(responsePanesUrl)
const { computeAnchoredScrollTop } = await import(historyViewportUrl)

function baseTurnEntries(turnId = 1) {
  return [
    { id: 1, role: "user", text: "Audit the transcript UI", turnId },
    { id: 2, role: "reasoning", text: "Thinking through the state transitions", turnId },
    {
      id: 3,
      role: "tool",
      text: "**bash** · COMPLETED\n\n**Command**\n```bash\n$ git status\n```",
      turnId,
      mergeKey: `tool-${turnId}`,
      sourceText: JSON.stringify({
        id: `tool-${turnId}`,
        tool: "bash",
        status: "completed",
        input: { command: "git status" },
      }),
    },
    { id: 4, role: "assistant", text: "The transcript UI is updated.", turnId },
  ]
}

function logStep(name, details) {
  console.log(`[drill] ${name}`, details ? JSON.stringify(details) : "")
}

const summaryOnly = applyTranscriptDisplayState(baseTurnEntries())
assert.deepEqual(
  summaryOnly.filter((entry) => !entry.hidden).map((entry) => [entry.role, entry.text]),
  [
    ["user", "Audit the transcript UI"],
    ["turn_toggle", "click to expand"],
    ["assistant", "The transcript UI is updated."],
  ],
)
logStep("summary_only_turn", {
  visible: summaryOnly.filter((entry) => !entry.hidden).map((entry) => entry.role),
})

const expandedTurn = applyTranscriptDisplayState(baseTurnEntries(), [1])
assert.deepEqual(
  expandedTurn.filter((entry) => !entry.hidden).map((entry) => entry.role),
  ["user", "turn_toggle", "reasoning", "tool", "assistant"],
)
assert.equal(expandedTurn.find((entry) => entry.id === 3)?.blobCollapsed, true)
logStep("expanded_turn", {
  visible: expandedTurn.filter((entry) => !entry.hidden).map((entry) => entry.role),
  tool_summary: expandedTurn.find((entry) => entry.id === 3)?.blobSummary,
})

const expandedBlob = setTranscriptBlobCollapsed(expandedTurn, 3, [1], false)
assert.equal(expandedBlob.find((entry) => entry.id === 3)?.blobCollapsed, false)
logStep("expanded_blob", {
  tool_collapsed: expandedBlob.find((entry) => entry.id === 3)?.blobCollapsed,
})

const anchoredScrollTop = computeAnchoredScrollTop(12, 90, 320, 80)
assert.equal(anchoredScrollTop, 78)
logStep("anchored_scroll", { scroll_top: anchoredScrollTop })

const splitSelection = selectResponsePaneAgents(
  [{ id: "agent-a" }, { id: "agent-b" }, { id: "agent-c" }],
  "agent-c",
  true,
  3,
)
assert.deepEqual(splitSelection.visibleAgents.map((agent) => agent.id), ["agent-a", "agent-b", "agent-c"])

const paneEntries = {
  "agent-a": applyTranscriptDisplayState(baseTurnEntries(1), []),
  "agent-b": applyTranscriptDisplayState(baseTurnEntries(2), [2]),
  "agent-c": applyTranscriptDisplayState(baseTurnEntries(3), []),
}
assert.equal(paneEntries["agent-a"].find((entry) => entry.role === "turn_toggle")?.text, "click to expand")
assert.equal(paneEntries["agent-b"].find((entry) => entry.role === "turn_toggle")?.text, "click to collapse")
assert.equal(paneEntries["agent-c"].find((entry) => entry.role === "turn_toggle")?.text, "click to expand")
logStep("split_panes", {
  visible_agents: splitSelection.visibleAgents.map((agent) => agent.id),
  toggle_labels: Object.fromEntries(
    Object.entries(paneEntries).map(([agentId, entries]) => [agentId, entries.find((entry) => entry.role === "turn_toggle")?.text ?? null]),
  ),
})

console.log("transcript display drills passed")
