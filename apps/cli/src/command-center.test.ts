import assert from "node:assert/strict"
import test from "node:test"

import {
  buildCommandCenterItems,
  nextCommandCenterIndex,
  shouldSubmitExactCommandCenterMatch,
} from "./command-center.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs } from "./provider-command-catalog.js"

test("buildCommandCenterItems shows root slash commands", () => {
  const items = buildCommandCenterItems("/", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.some((item) => item.kind === "group" && item.label === "/provider"), true)
  assert.equal(items.some((item) => item.kind === "group" && item.label === "/opencode"), true)
  assert.equal(items.some((item) => item.kind === "group" && item.label === "/model"), true)
  assert.equal(items.some((item) => item.kind === "group" && item.label === "/variant"), true)
  assert.equal(items.some((item) => item.kind === "group" && item.label === "/view"), true)
  assert.equal(items.some((item) => item.kind === "group" && item.label === "/config"), true)
  assert.equal(items.some((item) => item.kind === "command" && item.label === "/exit"), true)
})

test("buildCommandCenterItems includes config subcommands", () => {
  const items = buildCommandCenterItems("/config", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.some((item) => item.kind === "command" && item.value === "/config show"), true)
  assert.equal(items.some((item) => item.kind === "command" && item.value === "/config path"), true)
  assert.equal(items.some((item) => item.kind === "group" && item.value === "/config managed-io "), true)
})

test("buildCommandCenterItems filters model options", () => {
  const items = buildCommandCenterItems("/model gpt", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "codex",
    focusedProvider: "codex",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.every((item) => item.kind === "model"), true)
  assert.equal(items.some((item) => item.value === "codex/gpt-5.4"), true)
  assert.equal(items.some((item) => item.value === "openai/gpt-5.4"), false)
})

test("buildCommandCenterItems filters variant options", () => {
  const items = buildCommandCenterItems("/variant med", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.kind, "variant")
  assert.equal(items[0]?.value, "medium")
})

test("buildCommandCenterItems closes exact trailing-space commands", () => {
  const items = buildCommandCenterItems("/agent delete ", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.deepEqual(items, [])
})

test("buildCommandCenterItems shows the delete agent command", () => {
  const items = buildCommandCenterItems("/agent del", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.label, "delete")
})

test("buildCommandCenterItems keeps the scoped parent visible for grouped commands", () => {
  const items = buildCommandCenterItems("/agent", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.label, "/agent")
  assert.equal(items.some((item) => item.label === "spawn"), true)
})

test("buildCommandCenterItems keeps the parent group visible while filtering scoped agent commands", () => {
  const items = buildCommandCenterItems("/agent sp", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.label, "spawn")
  assert.equal(items.some((item) => item.kind === "group" && item.label === "/agent"), true)
})

test("buildCommandCenterItems keeps provider options open after space", () => {
  const items = buildCommandCenterItems("/provider ", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.some((item) => item.kind === "provider" && item.value === "opencode"), true)
  assert.equal(items.some((item) => item.kind === "provider" && item.value === "codex"), true)
})

test("buildCommandCenterItems keeps the parent group visible while filtering provider commands", () => {
  const items = buildCommandCenterItems("/provider st", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.some((item) => item.label === "status"), true)
  assert.equal(items.some((item) => item.kind === "group" && item.label === "/provider"), true)
})

test("buildCommandCenterItems shows multi-agent view options", () => {
  const items = buildCommandCenterItems("/view spl", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.label, "split")
  assert.equal(items[0]?.value, "/view split")
})

test("buildCommandCenterItems includes workflow subcommands", () => {
  const items = buildCommandCenterItems("/workflow", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })
  const labels = new Set(items.map((item) => item.label))

  assert.equal(labels.has("list"), true)
  assert.equal(labels.has("show"), true)
  assert.equal(labels.has("new"), true)
  assert.equal(labels.has("node"), true)
  assert.equal(labels.has("edge"), true)
  assert.equal(labels.has("endpoint"), true)
  assert.equal(labels.has("/workflow"), true)
})

test("buildCommandCenterItems drills into workflow node subcommands", () => {
  const items = buildCommandCenterItems("/workflow node", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.some((item) => item.label === "add"), true)
  assert.equal(items.some((item) => item.label === "add all"), true)
  assert.equal(items.some((item) => item.label === "remove"), true)
  assert.equal(items.some((item) => item.kind === "group" && item.label === "instructions"), true)
})

test("buildCommandCenterItems exposes workflow add node all shorthand", () => {
  const items = buildCommandCenterItems("/workflow add", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.label, "add node all")
  assert.equal(items[0]?.value, "/workflow add node all")
})

test("nextCommandCenterIndex selects exact parent groups instead of preserving stale child indexes", () => {
  const items = buildCommandCenterItems("/workflow", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.kind, "group")
  assert.equal(items[0]?.label, "/workflow")
  assert.equal(items[2]?.kind, "command")
  assert.equal(nextCommandCenterIndex(2, items, "/workflow"), 0)
  assert.equal(nextCommandCenterIndex(2, items, "/workflow "), 0)
})

test("nextCommandCenterIndex preserves selection when the same exact query is resynced", () => {
  const items = buildCommandCenterItems("/workflow", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items[0]?.kind, "group")
  assert.equal(items[2]?.kind, "command")
  assert.equal(nextCommandCenterIndex(2, items, "/workflow", "/workflow"), 2)
})

test("buildCommandCenterItems exposes the focused provider namespace", () => {
  const items = buildCommandCenterItems("/codex", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "codex",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.some((item) => item.kind === "group" && item.label === "/codex"), true)
})

test("buildCommandCenterItems lets root slash search surface parent groups from matching subcommands", () => {
  const items = buildCommandCenterItems("/reauth", {
    providerCatalog: fallbackProviderCatalog(),
    providerCommandCatalogs: fallbackProviderCommandCatalogs(),
    currentProvider: "opencode",
    focusedProvider: "opencode",
    currentModel: "openai/gpt-5.4",
    currentVariant: "high",
  })

  assert.equal(items.some((item) => item.kind === "group" && item.label === "/provider"), true)
})

test("shouldSubmitExactCommandCenterMatch submits leaf commands but not parent groups", () => {
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

  assert.equal(shouldSubmitExactCommandCenterMatch({
    id: "agent-list",
    label: "list",
    description: "List all agents in the session",
    kind: "command",
    value: "/agent list",
  }, "/agent list"), true)
})
