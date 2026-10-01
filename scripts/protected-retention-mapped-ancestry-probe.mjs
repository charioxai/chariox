// Run only in the verified installed broker namespace. Synthetic metadata only.
import assert from "node:assert/strict"
import { mkdirSync, writeFileSync, chownSync, lstatSync, symlinkSync, chmodSync, unlinkSync, rmdirSync } from "node:fs"
import { join } from "node:path"
import { readPrivateFile } from "../apps/kernel/slice-linux-docker/protected-identity-retention.mjs"
import { DURABLE_LAYOUT_ROOT, readNamespaceEntry, isVerifiedHostAncestor } from "../apps/kernel/slice-linux-docker/protected-namespace-entry.mjs"

const proof = readNamespaceEntry()
const root = join(DURABLE_LAYOUT_ROOT, `.synthetic-retention-${process.pid}`)
const file = join(root, "metadata.json")
const alias = `${root}-alias`
mkdirSync(root, {mode: 0o700})
try {
  chownSync(root, proof.dataUid, proof.dataUid)
  writeFileSync(file, '{"synthetic":true}', {mode: 0o600, flag: "wx"})
  chownSync(file, proof.dataUid, proof.dataUid)
  const bytes = readPrivateFile(file, proof.dataUid)
  try { assert.equal(bytes.toString(), '{"synthetic":true}') } finally { bytes.fill(0) }
  const host = proof.ancestors.find(record => record.hostUid === 0 && record.path !== "/")
  assert.ok(host)
  const metadata = lstatSync(host.path)
  assert.equal(metadata.uid, 65534)
  assert.equal(isVerifiedHostAncestor(host.path, metadata), true)
  assert.equal(isVerifiedHostAncestor(host.path, {...metadata, ino: metadata.ino + 1}), false)
  assert.equal(isVerifiedHostAncestor(`${host.path}/unverified-synthetic`, metadata), false)
  symlinkSync(root, alias)
  assert.throws(() => readPrivateFile(join(alias, "metadata.json"), proof.dataUid))
  chmodSync(file, 0o640)
  assert.throws(() => readPrivateFile(file, proof.dataUid))
  chmodSync(file, 0o600)
  chownSync(file, 0, 0)
  assert.throws(() => readPrivateFile(file, proof.dataUid))
  console.log(JSON.stringify({mappedAncestryRead:true, exactInodeRequired:true, unverifiedAncestorRefused:true, symlinkRefused:true, nonPrivateModeRefused:true, foreignFileOwnerRefused:true, privateIdentityUsed:false}))
} finally {
  try { unlinkSync(alias) } catch (error) { if (error.code !== "ENOENT") throw error }
  unlinkSync(file)
  rmdirSync(root)
}
