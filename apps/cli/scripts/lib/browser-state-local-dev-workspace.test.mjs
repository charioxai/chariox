import assert from "node:assert/strict"
import test from "node:test"
import { assertLocalDevOwnedWorkspace } from "./browser-state-local-dev-workspace.mjs"

const fixture = () => ({
  mounts: [{ Type: "volume", Destination: "/workspace", Name: "chariox-slice-m20-fixture-workspace", RW: true }],
  labels: { "org.chariox.local.owner-slice": "slice-fixture", "org.chariox.local.owner-uid": "1000" },
  containerName: "chariox-slice-m20-fixture", sliceId: "slice-fixture", ownerUid: 1000,
})
test("MP-03/MP-10/MP-11 real local workspace rejects foreign ownership and host binds", () => {
  assert.doesNotThrow(() => assertLocalDevOwnedWorkspace(fixture()))
  for (const mutate of [
    f => f.mounts[0].Type = "bind", f => f.mounts[0].Name = "foreign",
    f => f.mounts[0].RW = false, f => f.mounts.push({ ...f.mounts[0] }), f => f.mounts = [],
    f => f.labels["org.chariox.local.owner-slice"] = "foreign",
    f => f.labels["org.chariox.local.owner-uid"] = "1001",
  ]) {
    const f = fixture(); mutate(f)
    assert.throws(() => assertLocalDevOwnedWorkspace(f))
  }
})
