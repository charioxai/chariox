import assert from "node:assert/strict"
import test from "node:test"

import { commandTreeFromTerminalCommandCatalog } from "./terminal-command-catalog.js"

test("terminal command catalog projection preserves routing metadata", () => {
  const tree = commandTreeFromTerminalCommandCatalog({
    revision: "test",
    nodes: [{
      id: "workflow",
      label: "/workflow",
      description: "Manage workflows",
      value: "/workflow ",
      kind: "group",
      execution_target: "kernel",
      surfaces: ["session", "workflow_screen"],
      search_aliases: ["automation"],
      intents: ["manage workflows"],
      examples: ["/workflow list"],
      dynamic_source: null,
      children: [{
        id: "workflow-list",
        label: "list",
        description: "List workflows",
        value: "/workflow list",
        kind: "command",
        execution_target: "kernel",
        surfaces: ["session"],
        search_aliases: [],
        intents: [],
        examples: [],
        dynamic_source: null,
        children: [],
      }],
    }],
  })

  assert.deepEqual(tree, [{
    id: "workflow",
    label: "/workflow",
    description: "Manage workflows",
    value: "/workflow ",
    kind: "group",
    executionTarget: "kernel",
    surfaces: ["session", "workflow_screen"],
    intents: ["manage workflows"],
    examples: ["/workflow list"],
    searchAliases: ["automation"],
    children: [{
      id: "workflow-list",
      label: "list",
      description: "List workflows",
      value: "/workflow list",
      kind: "command",
      executionTarget: "kernel",
      surfaces: ["session"],
    }],
  }])
})

test("terminal command catalog projection filters recursively by surface and execution target", () => {
  const tree = commandTreeFromTerminalCommandCatalog({
    revision: "test",
    nodes: [{
      id: "mixed",
      label: "/mixed",
      description: "Mixed commands",
      value: "/mixed ",
      kind: "group",
      execution_target: "kernel",
      surfaces: ["session"],
      children: [
        catalogNode("session", "/mixed session", "session", "kernel"),
        catalogNode("waiting", "/mixed waiting", "room_list", "kernel"),
        catalogNode("local", "/mixed local", "room_list", "terminal_local"),
        catalogNode("prompt", "/mixed prompt", "room_list", "prompt_prefix"),
      ],
    }, {
      id: "empty-group",
      label: "/empty",
      description: "No commands for this client",
      value: "/empty ",
      kind: "group",
      execution_target: "kernel",
      surfaces: ["room_list"],
      children: [catalogNode("empty-prompt", "/empty prompt", "room_list", "prompt_prefix")],
    }],
  }, {
    surface: "room_list",
    executionTargets: ["kernel", "terminal_local"],
  })

  assert.deepEqual(tree.map((node) => ({
    id: node.id,
    children: node.children?.map((child) => child.id),
  })), [{ id: "mixed", children: ["waiting", "local"] }])
})

function catalogNode(
  id: string,
  value: string,
  surface: "session" | "room_list",
  executionTarget: "kernel" | "terminal_local" | "prompt_prefix",
) {
  return {
    id,
    label: id,
    description: id,
    value,
    kind: "command" as const,
    execution_target: executionTarget,
    surfaces: [surface],
  }
}
