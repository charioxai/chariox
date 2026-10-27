import assert from "node:assert/strict"

export async function validatePrebuiltSliceImage(image, sourceIdentity, inspect) {
  if (!image?.trim()) return null
  const records = await inspect(image.trim())
  assert.equal(records.length, 1, "expected exactly one prebuilt slice image")
  const record = records[0]
  assert.ok(record?.Id, "prebuilt slice image identity is missing")
  const labels = record.Config?.Labels ?? {}
  assert.equal(labels["io.chariox.runtime-source-revision"], sourceIdentity.runtimeSourceRevision,
    "prebuilt slice image must contain the exact current runtime source")
  assert.equal(labels["io.chariox.relay-peer-protocol-version"], String(sourceIdentity.protocolVersions.relayPeer),
    "prebuilt slice image relay protocol must match current source")
  return record.Id
}
