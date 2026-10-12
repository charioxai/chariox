import assert from "node:assert/strict"
import test from "node:test"

import type { CommandCenterItem } from "./command-center-types.js"
import {
  commandCenterCompletionText,
  commandCenterExecutionCommand,
  nextCommandCenterIndex,
  promptAddsArgumentsToCommandCenterItem,
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

test("command center completion text expands provider, account, model, and variant items", () => {
  assert.equal(commandCenterCompletionText({
    id: "agent-review-agent",
    label: "@Review Agent",
    description: "Agent",
    kind: "agent",
    value: "Review Agent",
  }), '@"Review Agent" ')
  assert.equal(commandCenterCompletionText({
    id: "provider-codex",
    label: "codex",
    description: "Codex",
    kind: "provider",
    value: "codex",
  }), "/provider codex")
  assert.equal(commandCenterCompletionText({
    id: "account-secondary",
    label: "Validation",
    description: "Account",
    kind: "account",
    value: "secondary",
  }), "/account secondary")
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

test("command center treats a command followed by arguments as the typed command", () => {
  const dev: CommandCenterItem = {
    id: "app-dev",
    label: "dev",
    description: "Pack an App source directory",
    kind: "command",
    value: "/app dev ",
  }
  assert.equal(promptAddsArgumentsToCommandCenterItem(dev, "/app dev stop"), true)
  assert.equal(promptAddsArgumentsToCommandCenterItem(dev, '/app dev "./My App" --key k'), true)
  assert.equal(promptAddsArgumentsToCommandCenterItem(dev, "/app dev "), false)
  assert.equal(promptAddsArgumentsToCommandCenterItem(dev, "/app dev   "), false)
  assert.equal(promptAddsArgumentsToCommandCenterItem(dev, "/app dev"), false)
  assert.equal(promptAddsArgumentsToCommandCenterItem(dev, "/app devs"), false)
  // A command without a trailing space takes arguments after a space.
  assert.equal(promptAddsArgumentsToCommandCenterItem(commandItem, "/agent list --all"), true)
  assert.equal(promptAddsArgumentsToCommandCenterItem(commandItem, "/agent listing"), false)
  assert.equal(promptAddsArgumentsToCommandCenterItem(commandItem, "/agent list"), false)
  assert.equal(shouldSubmitExactCommandCenterMatch(commandItem, "/agent list --all"), true)
  // Suggestions keep completing; typed group arguments must survive.
  assert.equal(promptAddsArgumentsToCommandCenterItem({ ...dev, kind: "group" }, "/app dev stop"), true)
  assert.equal(promptAddsArgumentsToCommandCenterItem({ ...dev, kind: "model" }, "/app dev stop"), false)
})

test("command center submit selection bypasses session alias prompts", () => {
  assert.equal(shouldBypassCommandCenterSubmitSelection("/session docs"), true)
  assert.equal(shouldBypassCommandCenterSubmitSelection("/session attach docs"), false)
  assert.equal(shouldBypassCommandCenterSubmitSelection("/workflow docs"), false)
})

// MP-08/MP-10/MP-11: real hosted typed /machine list was replaced by a group entry.
test("typed machine and agent commands bypass their palette group", () => {
  for (const [prefix, prompt] of [
    ["/machine ", "/machine list"],
    ["/machine ", "/machine kernels machine-worker"],
    ["/agent ", "/agent spawn review --kernel kernel-worker"],
  ] as const) {
    const group: CommandCenterItem = {
      id: prefix, label: prefix, description: "Command group", kind: "group", value: prefix,
    }
    assert.equal(promptAddsArgumentsToCommandCenterItem(group, prompt), true)
    assert.equal(shouldSubmitExactCommandCenterMatch(group, prompt), true)
    assert.equal(shouldSubmitExactCommandCenterMatch(group, prefix), false)
  }
})

// MP-08 / MP-10 / MP-11: live /machine list must not select the first kernels suggestion.
test("typed exact command wins over a stale highlighted suggestion", () => {
  const items: CommandCenterItem[] = [
    {...commandItem, id: "kernels", value: "/machine kernels "},
    {...commandItem, id: "list", value: "/machine list"},
    {...commandItem, id: "group", kind: "group", value: "/machine "},
  ]
  assert.equal(nextCommandCenterIndex(0, items, "/machine list", "/account fleet-opencode"), 1)
  assert.equal(nextCommandCenterIndex(0, items, "/machine kernels worker", "/machine list"), 0)
})
