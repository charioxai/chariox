import assert from "node:assert/strict"
import test from "node:test"

import {
  formatExtensionGrantRuntimeDetail,
  formatExtensionGrantPlacement,
  formatExtensionGrantPlacementSummary,
  hasActiveHomeProxyExtensionGrants,
} from "./extension-grant-placement.js"

test("extension grant placement distinguishes active home proxy tools from passive skills", () => {
  assert.equal(hasActiveHomeProxyExtensionGrants([{ kind: "skill" }]), false)
  assert.equal(hasActiveHomeProxyExtensionGrants([{ kind: "mcp" }]), true)
  assert.equal(formatExtensionGrantPlacement([{ kind: "skill" }], true), "skills snapshot")
  assert.equal(formatExtensionGrantPlacement([{ kind: "script" }], true), "active tools home-proxy")
  assert.equal(
    formatExtensionGrantPlacement([{ kind: "mcp" }, { kind: "skill" }], true),
    "active tools home-proxy; skills snapshot",
  )
  assert.equal(formatExtensionGrantPlacement([{ kind: "mcp" }], false), "worker-local")
})

test("extension grant placement summary formats shell and inspect count styles", () => {
  assert.equal(
    formatExtensionGrantPlacementSummary([{ kind: "skill" }], { remote: false }),
    "1 grant (worker-local; skill 1)",
  )
  assert.equal(
    formatExtensionGrantPlacementSummary([{ kind: "mcp" }, { kind: "skill" }], { remote: true }),
    "2 grants (active tools home-proxy; skills snapshot; mcp 1, skill 1)",
  )
  assert.equal(
    formatExtensionGrantPlacementSummary(
      [{ kind: "mcp" }, { kind: "skill" }],
      { remote: true, countSeparator: "=" },
    ),
    "2 grants (active tools home-proxy; skills snapshot; mcp=1, skill=1)",
  )
  assert.equal(
    formatExtensionGrantPlacementSummary(
      [{ kind: "skill" }, { kind: "script" }],
      { remote: true, countSeparator: "=" },
    ),
    "2 grants (active tools home-proxy; skills snapshot; script=1, skill=1)",
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
    formatExtensionGrantRuntimeDetail([{ kind: "skill" }], true),
    "skill snapshot: home projects passive content; executable helpers require a separate tool grant",
  )
  assert.equal(
    formatExtensionGrantRuntimeDetail([{ kind: "mcp" }, { kind: "skill" }], true),
    "home-proxy tools execute on home with home-owned grants and credentials; skills are passive snapshots",
  )
})
