import assert from "node:assert/strict"
import test from "node:test"
import { toggleFocusedTranscriptDetails, toggleTranscriptDetails } from "./transcript-details-controller.js"
import type { TranscriptEntry } from "./cli-types.js"

// MP-08 / MP-10, #987: focus determines the detail target in split mode.
test("detail shortcut targets the focused auxiliary pane", () => {
  const calls: string[] = []
  const deps = { focusedAgentId: () => "b", visibleAgentId: () => "a", primary: () => { calls.push("primary") }, auxiliary: (id: string) => { calls.push(id) } }
  toggleFocusedTranscriptDetails(deps)
  toggleFocusedTranscriptDetails({ ...deps, focusedAgentId: () => "a" })
  assert.deepEqual(calls, ["b", "primary"])
})

test("bulk details materialize sibling placeholders in one commit", async () => {
  let entries = [1, 2].map(id => ({ id, role: "tool", text: "placeholder", blobCollapsible: true, blobCollapsed: true, historyBlobAgentId: "a", historyBlobId: "blob-" + id })) as TranscriptEntry[]
  let commits = 0
  const calls: readonly string[][] = []
  toggleTranscriptDetails({ entries: () => entries, collapsedTurns: () => [], commit: (_current, next) => { entries = next; commits++ }, load: async (agent, ids) => {
    assert.equal(agent, "a")
    ;(calls as string[][]).push([...ids])
    return ids.map((blob_id, i) => ({ blob_id, entries: [{ entry_index: i + 1, fragment_start: 0, fragment_end: 6, total_chars: 6, entry: { timestamp_ms: i + 1, kind: "provider_reasoning", text: "loaded" + i } }] }))
  } })
  await new Promise(resolve => setImmediate(resolve))
  assert.deepEqual(calls, [["blob-1", "blob-2"]])
  assert.equal(commits, 2) // one loading projection, one final projection
  assert.equal(entries.length, 2)
  assert.ok(entries.every(entry => !entry.historyBlobId && entry.historyBlobLoaded))
})
