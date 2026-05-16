import assert from "node:assert/strict"
import test from "node:test"

import {
  cursorIsOnFirstPromptLine,
  cursorIsOnLastPromptLine,
  extractPromptHistoryEntries,
  extractPromptInputHistoryEntries,
  isProgrammaticPromptContentEcho,
  navigatePromptHistory,
  promptHistoryDirectionForKey,
  resolvePromptHistoryKeyNavigation,
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

test("extractPromptHistoryEntries rebuilds full prompts from fragmented session history", () => {
  assert.deepEqual(
    extractPromptHistoryEntries([
      {
        entry_index: 1,
        fragment_start: 0,
        fragment_end: 7,
        total_chars: 11,
        entry: { kind: "user_prompt", text: "git sta" },
      },
      {
        entry_index: 1,
        fragment_start: 7,
        fragment_end: 11,
        total_chars: 11,
        entry: { kind: "user_prompt", text: "tus\n" },
      },
      {
        entry_index: 2,
        fragment_start: 0,
        fragment_end: 5,
        total_chars: 5,
        entry: { kind: "provider_output", text: "done\n" },
      },
      {
        entry_index: 3,
        fragment_start: 0,
        fragment_end: 8,
        total_chars: 8,
        entry: { kind: "user_prompt", text: "git log\n" },
      },
    ]),
    ["git status", "git log"],
  )
})

test("extractPromptInputHistoryEntries merges prompts and slash commands by kernel sequence", () => {
  assert.deepEqual(
    extractPromptInputHistoryEntries([
      {
        sequence: 3,
        timestamp_ms: 30,
        session_id: "session-1",
        source_attachment_id: "cli-2",
        kind: "prompt",
        text: "git diff\n",
      },
      {
        sequence: 2,
        timestamp_ms: 20,
        session_id: "session-1",
        source_attachment_id: "cli-1",
        kind: "command",
        text: "/agent list\n",
      },
      {
        sequence: 1,
        timestamp_ms: 10,
        session_id: "session-1",
        source_attachment_id: "cli-1",
        kind: "prompt",
        text: "git status",
      },
    ]),
    ["git status", "/agent list", "git diff"],
  )
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

test("navigatePromptHistory restores a saved draft even if navigation index was cleared", () => {
  const restored = navigatePromptHistory({
    entries: ["git status", "git diff"],
    currentText: "git diff",
    navigationIndex: null,
    navigationDraft: "draft prompt",
    direction: "next",
  })

  assert.deepEqual(restored, {
    text: "draft prompt",
    navigationIndex: null,
    navigationDraft: null,
  })
})

test("isProgrammaticPromptContentEcho preserves history navigation on setText echoes", () => {
  assert.equal(
    isProgrammaticPromptContentEcho({
      currentText: "git log",
      previousSnapshot: "git log",
      programmaticMutation: false,
      dropPending: false,
    }),
    true,
  )
  assert.equal(
    isProgrammaticPromptContentEcho({
      currentText: "git log with edit",
      previousSnapshot: "git log",
      programmaticMutation: false,
      dropPending: false,
    }),
    false,
  )
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

test("promptHistoryDirectionForKey yields multiline prompts to cursor navigation", () => {
  assert.equal(promptHistoryDirectionForKey({
    attached: true,
    promptFocused: true,
    commandCenterOpen: false,
    keyName: "up",
    currentText: "first\nsecond",
    cursorOffset: "first\nsecond".length,
  }), null)
  assert.equal(promptHistoryDirectionForKey({
    attached: true,
    promptFocused: true,
    commandCenterOpen: false,
    keyName: "up",
    currentText: "first\nsecond",
    cursorOffset: 2,
  }), "previous")
  assert.equal(promptHistoryDirectionForKey({
    attached: true,
    promptFocused: true,
    commandCenterOpen: false,
    keyName: "down",
    currentText: "first\nsecond",
    cursorOffset: 2,
  }), null)
  assert.equal(promptHistoryDirectionForKey({
    attached: true,
    promptFocused: true,
    commandCenterOpen: false,
    keyName: "down",
    currentText: "first\nsecond",
    cursorOffset: "first\nsecond".length,
  }), "next")
})

test("prompt cursor line helpers clamp offsets", () => {
  assert.equal(cursorIsOnFirstPromptLine("first\nsecond", -10), true)
  assert.equal(cursorIsOnFirstPromptLine("first\nsecond", 999), false)
  assert.equal(cursorIsOnLastPromptLine("first\nsecond", -10), false)
  assert.equal(cursorIsOnLastPromptLine("first\nsecond", 999), true)
})

test("resolvePromptHistoryKeyNavigation ignores next with no active navigation", () => {
  assert.equal(resolvePromptHistoryKeyNavigation({
    attached: true,
    promptFocused: true,
    commandCenterOpen: false,
    keyName: "down",
    currentText: "",
    cursorOffset: 0,
    navigationIndex: null,
    navigationDraft: null,
  }), null)
  assert.equal(resolvePromptHistoryKeyNavigation({
    attached: true,
    promptFocused: true,
    commandCenterOpen: false,
    keyName: "down",
    currentText: "",
    cursorOffset: 0,
    navigationIndex: 0,
    navigationDraft: null,
  }), "next")
  assert.equal(resolvePromptHistoryKeyNavigation({
    attached: true,
    promptFocused: true,
    commandCenterOpen: false,
    keyName: "up",
    currentText: "",
    cursorOffset: 0,
    navigationIndex: null,
    navigationDraft: null,
  }), "previous")
})
