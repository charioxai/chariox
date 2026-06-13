import assert from "node:assert/strict"
import test from "node:test"

import {
  historyOutlineRows,
  historyOutlineText,
} from "./drill-history-outline.mjs"

test("flattens outline entries and blob summaries without user prompts by default", () => {
  const outline = sampleOutline()

  assert.deepEqual(historyOutlineRows(outline).map((row) => row.entry.text), [
    "assistant output",
    "summary text",
    "blob summary",
  ])
  assert.equal(historyOutlineText(outline), "assistant output\nsummary text\nblob summary")
})

test("can include user prompts for restart persistence assertions", () => {
  assert.equal(
    historyOutlineText(sampleOutline(), { includeUserPrompt: true }),
    "user marker\nassistant output\nsummary text\nblob summary",
  )
})

function sampleOutline() {
  return {
    agents: [{
      turns: [{
        user_prompt: { entry: { text: "user marker" } },
        entries: [{ entry: { text: "assistant output" } }],
        summary: { entry: { text: "summary text" } },
        blobs: [{ summary: "blob summary" }],
      }],
    }],
  }
}
