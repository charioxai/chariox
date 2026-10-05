// MP-07 / MP-11: owner signing hook pins the final bytes; synthetic key stays in memory.
import assert from "node:assert/strict"
import { generateKeyPairSync, sign } from "node:crypto"
import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises"
import { join } from "node:path"
import { tmpdir } from "node:os"
import test from "node:test"
import { signSetup } from "./sign-chariox-setup.mjs"
test("MP-07 owner hook checks final bytes against the approved public pin", async t => {
  const root = await mkdtemp(join(await realpath(tmpdir()), "setup-sign-test-")); t.after(() => rm(root, { recursive: true, force: true }))
  const keys = generateKeyPairSync("ed25519"), bytes = Buffer.from("synthetic installer")
  const artifact = join(root, "artifact"), signature = join(root, "output.sig"), supplied = join(root, "supplied.sig"), signer = join(root, "signer")
  await writeFile(artifact, bytes); await writeFile(supplied, sign(null, bytes, keys.privateKey).toString("hex"))
  // Stand-in signing service has public signature bytes only, never the test private key.
  await writeFile(signer, `#!/bin/sh\ncp '${supplied}' "$2"\n`, { mode: 0o755 })
  const publicKeyHex = keys.publicKey.export({ format: "der", type: "spki" }).subarray(-32).toString("hex")
  assert.equal((await signSetup({ artifact, signature, signer, publicKeyHex })).verified, true)
  await writeFile(artifact, "changed after signing")
  await assert.rejects(signSetup({ artifact, signature, signer, publicKeyHex }), /published public pin/)
});
