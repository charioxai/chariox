import assert from "node:assert/strict"
import test from "node:test"
import path from "node:path"

import {
  derivePromptContentChangeDecision,
} from "./prompt-content-change-policy.js"

test("prompt content change policy records detached edits as prompt snapshots", () => {
  assert.deepEqual(derivePromptContentChangeDecision({
    attached: false,
    currentText: "/session new",
    previousSnapshot: "",
    programmaticMutation: false,
    dropPending: false,
    promptHistoryActive: false,
    sessionId: null,
    cwd: process.cwd(),
  }), {
    kind: "detached",
    nextSnapshot: "/session new",
    commandCenterText: "/session new",
  })
})

test("prompt content change policy ignores programmatic echoes", () => {
  assert.equal(derivePromptContentChangeDecision({
    attached: true,
    currentText: "draft",
    previousSnapshot: "previous",
    programmaticMutation: true,
    dropPending: false,
    promptHistoryActive: true,
    sessionId: "s1",
    cwd: process.cwd(),
  }).kind, "programmatic")
})

test("prompt content change policy persists normal text edits and resets history navigation", () => {
  assert.deepEqual(derivePromptContentChangeDecision({
    attached: true,
    currentText: "next draft",
    previousSnapshot: "draft",
    programmaticMutation: false,
    dropPending: false,
    promptHistoryActive: true,
    sessionId: "s1",
    cwd: process.cwd(),
  }), {
    kind: "text",
    nextSnapshot: "next draft",
    commandCenterText: "next draft",
    syncAttachmentText: "next draft",
    resetPromptHistory: true,
    persistDraft: {
      sessionId: "s1",
      text: "next draft",
    },
  })
})

test("prompt content change policy identifies dropped attachment paths", () => {
  const droppedPath = path.join(process.cwd(), "package.json")
  const decision = derivePromptContentChangeDecision({
    attached: true,
    currentText: `attach ${JSON.stringify(droppedPath)}`,
    previousSnapshot: "attach ",
    programmaticMutation: false,
    dropPending: false,
    promptHistoryActive: false,
    sessionId: "s1",
    cwd: process.cwd(),
  })

  assert.equal(decision.kind, "drop")
  if (decision.kind !== "drop") {
    return
  }
  assert.equal(decision.nextPromptText, "attach ")
  assert.equal(decision.insertAt, "attach ".length)
  assert.equal(decision.files[0]?.path, droppedPath)
  assert.deepEqual(decision.persistDraft, {
    sessionId: "s1",
    text: "attach ",
  })
})
