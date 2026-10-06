import assert from "node:assert/strict"

// MP-03/MP-10/MP-11: prove actual Docker ownership instead of adapting a managed
// host publication into a local DEV bind. Never inspect volume contents.
export function assertLocalDevOwnedWorkspace({ mounts, labels, containerName, sliceId, ownerUid }) {
  const workspace = mounts.filter(mount => mount.Destination === "/workspace")
  assert.equal(workspace.length, 1, "local DEV must have one workspace mount")
  assert.equal(workspace[0].Type, "volume", "local DEV workspace must be a volume")
  assert.equal(workspace[0].Name, `${containerName}-workspace`, "local DEV workspace volume identity changed")
  assert.equal(workspace[0].RW, true, "local DEV workspace must be writable")
  assert.equal(labels?.["org.chariox.local.owner-slice"], sliceId, "foreign slice workspace")
  assert.equal(labels?.["org.chariox.local.owner-uid"], String(ownerUid), "foreign user workspace")
}
