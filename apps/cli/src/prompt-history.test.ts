import assert from "node:assert/strict"
import test from "node:test"

import {
  navigatePromptHistory,
  promptHistoryDirectionForKey,
  pushPromptHistoryEntry,
} from "./prompt-history.js"

test("pushPromptHistoryEntry appends normalized prompts and skips consecutive duplicates", () => {
  assert.deepEqual(
    pushPromptHistoryEntry(["git status"], "git diff\n"),
    ["git status", "git diff"],
  )
  assert.deepEqual(
    pushPromptHistoryEntry(["git status"], "git status"),
    ["git status"],
  )
})

test("pushPromptHistoryEntry keeps slash commands in prompt-area history", () => {
  assert.deepEqual(
    pushPromptHistoryEntry(["fix the failing test"], "/session list\n"),
    ["fix the failing test", "/session list"],
  )
})

test("pushPromptHistoryEntry keeps the full session prompt history", () => {
  let entries: string[] = []
  for (let index = 1; index <= 150; index += 1) {
    entries = pushPromptHistoryEntry(entries, `prompt ${index}`)
  }

  assert.equal(entries.length, 150)
  assert.equal(entries[0], "prompt 1")
  assert.equal(entries.at(-1), "prompt 150")
})

test("navigatePromptHistory walks backward through prompt history and restores the draft on exit", () => {
  const first = navigatePromptHistory({
    entries: ["git status", "git diff", "git log"],
    currentText: "draft prompt",
    navigationIndex: null,
    navigationDraft: null,
    direction: "previous",
  })

  assert.deepEqual(first, {
    text: "git log",
    navigationIndex: 2,
    navigationDraft: "draft prompt",
  })

  const second = navigatePromptHistory({
    entries: ["git status", "git diff", "git log"],
    currentText: first.text,
    navigationIndex: first.navigationIndex,
    navigationDraft: first.navigationDraft,
    direction: "previous",
  })

  assert.deepEqual(second, {
    text: "git diff",
    navigationIndex: 1,
    navigationDraft: "draft prompt",
  })

  const third = navigatePromptHistory({
    entries: ["git status", "git diff", "git log"],
    currentText: second.text,
    navigationIndex: second.navigationIndex,
    navigationDraft: second.navigationDraft,
    direction: "next",
  })

  assert.deepEqual(third, {
    text: "git log",
    navigationIndex: 2,
    navigationDraft: "draft prompt",
  })

  const fourth = navigatePromptHistory({
    entries: ["git status", "git diff", "git log"],
    currentText: third.text,
    navigationIndex: third.navigationIndex,
    navigationDraft: third.navigationDraft,
    direction: "next",
  })

  assert.deepEqual(fourth, {
    text: "draft prompt",
    navigationIndex: null,
    navigationDraft: null,
  })
})

test("promptHistoryDirectionForKey yields to the command center and modifiers", () => {
  assert.equal(promptHistoryDirectionForKey({
    attached: true,
    promptFocused: true,
    commandCenterOpen: true,
    keyName: "up",
  }), null)
  assert.equal(promptHistoryDirectionForKey({
    attached: true,
    promptFocused: true,
    commandCenterOpen: false,
    keyName: "up",
    ctrl: true,
  }), null)
  assert.equal(promptHistoryDirectionForKey({
    attached: true,
    promptFocused: true,
    commandCenterOpen: false,
    keyName: "up",
  }), "previous")
  assert.equal(promptHistoryDirectionForKey({
    attached: true,
    promptFocused: true,
    commandCenterOpen: false,
    keyName: "down",
  }), "next")
})
