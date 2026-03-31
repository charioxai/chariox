import test from "node:test"
import assert from "node:assert/strict"

import {
  executeSlashCommand,
  parseSlashCommand,
  shouldClearCommandCenterForSlashCommand,
} from "./commands.js"

test("parseSlashCommand parses session commands with action and value", () => {
  assert.deepEqual(parseSlashCommand("/session attach abc123"), {
    kind: "session",
    raw: "/session attach abc123",
    action: "attach",
    args: ["abc123"],
    value: "abc123",
  })
})

test("parseSlashCommand preserves raw attachment command input", () => {
  assert.deepEqual(parseSlashCommand('/attach "foo bar.txt"'), {
    kind: "attachment",
    raw: '/attach "foo bar.txt"',
  })
})

test("shouldClearCommandCenterForSlashCommand only clears selector-backed commands", () => {
  assert.equal(shouldClearCommandCenterForSlashCommand(parseSlashCommand("/model openai/gpt-5")!), true)
  assert.equal(shouldClearCommandCenterForSlashCommand(parseSlashCommand("/session list")!), false)
  assert.equal(shouldClearCommandCenterForSlashCommand(parseSlashCommand("/stop")!), false)
})

test("executeSlashCommand dispatches to the matching handler", async () => {
  const calls: string[] = []
  const command = await executeSlashCommand("/view split", {
    onExit: () => calls.push("exit"),
    onWaiting: () => calls.push("waiting"),
    onStop: () => calls.push("stop"),
    onAttachment: () => calls.push("attachment"),
    onSession: () => calls.push("session"),
    onProvider: () => calls.push("provider"),
    onModel: () => calls.push("model"),
    onVariant: () => calls.push("variant"),
    onView: () => calls.push("view"),
    onAgent: () => calls.push("agent"),
  })

  assert.deepEqual(calls, ["view"])
  assert.deepEqual(command, {
    kind: "view",
    raw: "/view split",
    value: "split",
  })
})

test("executeSlashCommand returns null for non-command input", async () => {
  const command = await executeSlashCommand("hello", {
    onExit: () => undefined,
    onWaiting: () => undefined,
    onStop: () => undefined,
    onAttachment: () => undefined,
    onSession: () => undefined,
    onProvider: () => undefined,
    onModel: () => undefined,
    onVariant: () => undefined,
    onView: () => undefined,
    onAgent: () => undefined,
  })

  assert.equal(command, null)
})
