import assert from "node:assert/strict"
import test from "node:test"

import type { CommandCenterItem } from "./command-center-types.js"
import {
  commandCenterCompletionText,
  commandCenterExecutionCommand,
  nextCommandCenterIndex,
  shouldBypassCommandCenterSubmitSelection,
  shouldSubmitExactCommandCenterMatch,
} from "./command-center-selection.js"

const commandItem: CommandCenterItem = {
  id: "agent-list",
  label: "list",
  description: "List agents",
  kind: "command",
  value: "/agent list",
}

test("command center completion text expands provider, model, and variant items", () => {
  assert.equal(commandCenterCompletionText({
    id: "provider-codex",
    label: "codex",
    description: "Codex",
    kind: "provider",
    value: "codex",
  }), "/provider codex")
  assert.equal(commandCenterCompletionText({
    id: "model-codex",
    label: "codex/gpt-5.4",
    description: "Model",
    kind: "model",
    value: "codex/gpt-5.4",
  }), "/model codex/gpt-5.4")
  assert.equal(commandCenterCompletionText({
    id: "variant-high",
    label: "high",
    description: "Variant",
    kind: "variant",
    value: "high",
  }), "/variant high")
})

test("command center execution command distinguishes executable and expandable groups", () => {
  assert.equal(commandCenterExecutionCommand(commandItem), "/agent list")
  assert.equal(commandCenterExecutionCommand({
    id: "workflow",
    label: "/workflow",
    description: "Workflow",
    kind: "group",
    value: "/workflow ",
  }), null)
  assert.equal(commandCenterExecutionCommand({
    id: "workflow-open",
    label: "open",
    description: "Open workflow",
    kind: "group",
    value: "/workflow",
  }), "/workflow")
})

test("command center index selects exact parent groups instead of preserving stale child indexes", () => {
  const items: CommandCenterItem[] = [
    {
      id: "workflow",
      label: "/workflow",
      description: "Workflow commands",
      kind: "group",
      value: "/workflow ",
    },
    commandItem,
    {
      id: "workflow-new",
      label: "new",
      description: "Create a workflow",
      kind: "command",
      value: "/workflow new ",
    },
  ]

  assert.equal(nextCommandCenterIndex(2, items, "/workflow"), 0)
  assert.equal(nextCommandCenterIndex(2, items, "/workflow "), 0)
  assert.equal(nextCommandCenterIndex(2, items, "/workflow", "/workflow"), 2)
})

test("command center exact submit matching submits leaf commands but not parent groups", () => {
  assert.equal(shouldSubmitExactCommandCenterMatch({
    id: "session-attach",
    label: "attach",
    description: "Attach to an existing session",
    kind: "command",
    value: "/session attach ",
  }, "/session attach"), true)

  assert.equal(shouldSubmitExactCommandCenterMatch({
    id: "workflow",
    label: "/workflow",
    description: "Inspect, edit, and run workflows",
    kind: "group",
    value: "/workflow ",
  }, "/workflow"), false)

  assert.equal(shouldSubmitExactCommandCenterMatch(commandItem, "/agent list"), true)
})

test("command center submit selection bypasses session alias prompts", () => {
  assert.equal(shouldBypassCommandCenterSubmitSelection("/session docs"), true)
  assert.equal(shouldBypassCommandCenterSubmitSelection("/session attach docs"), false)
  assert.equal(shouldBypassCommandCenterSubmitSelection("/workflow docs"), false)
})
