import assert from "node:assert/strict"
import test from "node:test"

import { createAgentPaneTranscriptRetentionController } from "./agent-pane-transcript-retention-controller.js"
import type { TranscriptEntry } from "./cli-types.js"

test("agent pane transcript retention trims entries and cleans tool state", () => {
  const deletedTools: Array<{ agentId: string; mergeKey: string }> = []
  const controller = createAgentPaneTranscriptRetentionController({
    maxEntries: 2,
    maxChars: 100,
    deleteToolForMergeKey: (agentId, mergeKey) => {
      deletedTools.push({ agentId, mergeKey })
    },
  })

  const trimmed = controller.trimLiveEntries("agent-1", [
    entry(1, "tool", "old", "tool-1"),
    entry(2, "assistant", "middle"),
    entry(3, "assistant", "new"),
  ])

  assert.deepEqual(trimmed, [
    entry(2, "assistant", "middle"),
    entry(3, "assistant", "new"),
  ])
  assert.deepEqual(deletedTools, [{ agentId: "agent-1", mergeKey: "tool-1" }])
})

function entry(
  id: number,
  role: TranscriptEntry["role"],
  text: string,
  mergeKey?: string,
): TranscriptEntry {
  return {
    id,
    role,
    text,
    ...(mergeKey ? { mergeKey } : {}),
  }
}
