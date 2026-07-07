import assert from "node:assert/strict"
import test from "node:test"

import {
  formatWorkspaceLiveSyncModeLabel,
  nextQueuedPromptSelectionId,
  normalizePromptOrigin,
  terminalRecordTranscriptProjection,
} from "./index.js"

test("kernel client root barrel exposes shared runtime projection helpers", () => {
  assert.equal(normalizePromptOrigin(" External "), "external")
  assert.equal(nextQueuedPromptSelectionId([{ promptId: "a" }, { promptId: "b" }], "a", 1), "b")
  assert.equal(
    formatWorkspaceLiveSyncModeLabel("tracked"),
    "tracked (selected workspace/worktree only; other repositories unrestricted)",
  )
  assert.equal(
    terminalRecordTranscriptProjection({
      kind: "provider_output",
    }, "ok", {
      isProviderIdleStatus: () => false,
      shouldRenderProviderStatus: () => true,
    }).role,
    "assistant",
  )
})
