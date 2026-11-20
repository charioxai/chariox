import assert from "node:assert/strict"
import test from "node:test"

import {
  formatExtensionAuthorityBoundaryDetail,
  formatExtensionGrantRuntimeDetail,
  formatExtensionGrantPlacement,
  formatExtensionGrantPlacementSummary,
  hasActiveHomeProxyExtensionGrants,
} from "./extension-grant-placement.js"

test("MP-10 extension placement contains executable grants only", () => {
  assert.equal(hasActiveHomeProxyExtensionGrants([{ kind: "connector" }]), true)
  assert.equal(hasActiveHomeProxyExtensionGrants([{ kind: "mcp" }]), true)
  assert.equal(formatExtensionGrantPlacement([{ kind: "connector" }], true), "active tools home-proxy")
  assert.equal(formatExtensionGrantPlacement([{ kind: "script" }], true), "active tools home-proxy")
  assert.equal(
    formatExtensionGrantPlacement([{ kind: "mcp" }, { kind: "connector" }], true),
    "active tools home-proxy",
  )
  assert.equal(formatExtensionGrantPlacement([{ kind: "mcp" }], false), "worker-local")
})

test("extension grant placement summary formats shell and inspect count styles", () => {
  assert.equal(
    formatExtensionGrantPlacementSummary([{ kind: "connector" }], { remote: false }),
    "1 grant (worker-local; connector 1)",
  )
  assert.equal(
    formatExtensionGrantPlacementSummary([{ kind: "mcp" }, { kind: "connector" }], { remote: true }),
    "2 grants (active tools home-proxy; mcp 1, connector 1)",
  )
  assert.equal(
    formatExtensionGrantPlacementSummary(
      [{ kind: "mcp" }, { kind: "connector" }],
      { remote: true, countSeparator: "=" },
    ),
    "2 grants (active tools home-proxy; mcp=1, connector=1)",
  )
  assert.equal(
    formatExtensionGrantPlacementSummary(
      [{ kind: "connector" }, { kind: "script" }],
      { remote: true, countSeparator: "=" },
    ),
    "2 grants (active tools home-proxy; script=1, connector=1)",
  )
})

test("extension grant runtime detail explains authority and execution", () => {
  assert.equal(
    formatExtensionGrantRuntimeDetail([{ kind: "mcp" }], false),
    "worker-local: definition and execution stay on the selected kernel",
  )
  assert.equal(
    formatExtensionGrantRuntimeDetail([{ kind: "script" }], true),
    "home-proxy: home owns definition, grants, credentials, and execution; worker receives projected tools",
  )
  assert.equal(
    formatExtensionGrantRuntimeDetail([{ kind: "connector" }], true),
    "home-proxy: home owns definition, grants, credentials, and execution; worker receives projected tools",
  )
  assert.equal(
    formatExtensionGrantRuntimeDetail([{ kind: "mcp" }, { kind: "connector" }], true),
    "home-proxy: home owns definition, grants, credentials, and execution; worker receives projected tools",
  )
})

test("extension authority boundary detail explains call validation and credentials", () => {
  assert.equal(
    formatExtensionAuthorityBoundaryDetail([{ kind: "mcp" }], false),
    "definition and execution stay on the selected kernel",
  )
  assert.equal(
    formatExtensionAuthorityBoundaryDetail([{ kind: "script" }], true),
    "home validates every call; credentials never leave home",
  )
  assert.equal(
    formatExtensionAuthorityBoundaryDetail([{ kind: "connector" }], true),
    "home validates every call; credentials never leave home",
  )
  assert.equal(
    formatExtensionAuthorityBoundaryDetail([{ kind: "mcp" }, { kind: "connector" }], true),
    "home validates every call; credentials never leave home",
  )
})
