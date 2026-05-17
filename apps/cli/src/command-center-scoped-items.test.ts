import assert from "node:assert/strict"
import test from "node:test"

import { buildScopedCommandCenterItems } from "./command-center-scoped-items.js"
import { fallbackProviderCommandCatalogs } from "./provider-command-catalog.js"

const context = {
  focusedProvider: "opencode" as const,
  providerCommandCatalogs: fallbackProviderCommandCatalogs(),
}

test("command center scoped items keep the scoped parent visible while filtering children", () => {
  const items = buildScopedCommandCenterItems("/agent sp", context) ?? []

  assert.equal(items.some((item) => item.kind === "group" && item.value === "/agent "), true)
  assert.equal(items[0]?.value, "/agent spawn ")
})

test("command center scoped items close exact trailing-space leaf commands", () => {
  assert.deepEqual(buildScopedCommandCenterItems("/agent delete ", context), [])
})

test("command center scoped items ignore inputs outside known scopes", () => {
  assert.equal(buildScopedCommandCenterItems("/unknown", context), null)
})
