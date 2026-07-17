import assert from "node:assert/strict"
import test from "node:test"

import {
  formatExtensionAuthorityBoundaryDetail,
  formatExtensionGrantRuntimeDetail,
  formatExtensionGrantPlacement,
  formatExtensionGrantPlacementSummary,
  hasActiveHomeProxyExtensionGrants,
  shouldShowRemoteExtensionManifestSync,
  shouldShowWorkerExtensionGrantSync,
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
  assert.equal(formatExtensionGrantPlacement([{ kind: "mcp" }], false), "home-local")
  assert.equal(formatExtensionGrantPlacement([{ source: "worker", kind: "mcp" }], false), "worker-local")
  assert.equal(
    formatExtensionGrantPlacement([
      { source: "home", kind: "mcp" },
      { source: "worker", kind: "script" },
    ], false),
    "home-local; worker-local",
  )
  assert.equal(formatExtensionGrantPlacement([{ source: "worker", kind: "mcp" }], true), "worker-local")
  assert.equal(
    formatExtensionGrantPlacement([
      { source: "home", kind: "mcp" },
      { source: "worker", kind: "mcp" },
    ], true),
    "worker-local; active tools home-proxy",
  )
  assert.equal(hasActiveHomeProxyExtensionGrants([{ source: "worker", kind: "mcp" }]), false)
})

test("extension sync lane visibility follows active source grants and pending final revokes", () => {
  assert.equal(shouldShowRemoteExtensionManifestSync([], { state: "synced" }), false)
  assert.equal(shouldShowWorkerExtensionGrantSync([], { state: "synced" }), false)
  assert.equal(shouldShowRemoteExtensionManifestSync([], { state: "failed", pending_revoke: true }), true)
  assert.equal(shouldShowWorkerExtensionGrantSync([], { state: "failed", pending_revoke: true }), true)
  assert.equal(shouldShowRemoteExtensionManifestSync([{ source: "home", kind: "script" }], null), true)
  assert.equal(shouldShowWorkerExtensionGrantSync([{ source: "worker", kind: "script" }], null), true)
})

test("extension grant placement summary formats shell and inspect count styles", () => {
  assert.equal(
    formatExtensionGrantPlacementSummary([{ kind: "skill" }], { remote: false }),
    "1 grant (home-local; skill 1)",
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
    "home-local: definition, credentials, and execution stay on the home kernel",
  )
  assert.equal(
    formatExtensionGrantRuntimeDetail([{ source: "worker", kind: "mcp" }], false),
    "worker-local: definition, credentials, and execution stay on the worker kernel",
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

test("extension authority boundary detail explains call validation and credentials", () => {
  assert.equal(
    formatExtensionAuthorityBoundaryDetail([{ kind: "mcp" }], false),
    "home resolves, validates, and executes locally; credentials stay on home",
  )
  assert.equal(
    formatExtensionAuthorityBoundaryDetail([{ source: "worker", kind: "mcp" }], false),
    "worker resolves, validates, and executes locally; credentials stay on worker",
  )
  assert.equal(
    formatExtensionAuthorityBoundaryDetail([{ kind: "script" }], true),
    "home validates every call; credentials never leave home",
  )
  assert.equal(
    formatExtensionAuthorityBoundaryDetail([{ kind: "skill" }], true),
    "passive content only; executable helpers require separate tool grants",
  )
  assert.equal(
    formatExtensionAuthorityBoundaryDetail([{ kind: "mcp" }, { kind: "skill" }], true),
    "home validates every call; credentials never leave home",
  )
})
