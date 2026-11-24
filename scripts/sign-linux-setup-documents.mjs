#!/usr/bin/env node
import { createHash, createPrivateKey, createPublicKey, sign } from "node:crypto"
import { mkdir, readFile, writeFile } from "node:fs/promises"
import { join } from "node:path"

const [directory, expected, signingKey, output] = process.argv.slice(2)
if (!directory || !/^[a-f0-9]{64}$/.test(expected ?? "") || !signingKey || !output || process.argv.length !== 6) {
  throw new Error("Usage: sign-linux-setup-documents.mjs REQUEST_DIR REQUEST_SHA256 RELEASE_PEM OUTPUT_DIR")
}
const sha256 = bytes => createHash("sha256").update(bytes).digest("hex")
const requestBytes = await readFile(join(directory, "request.json"))
if (sha256(requestBytes) !== expected) throw new Error("signing request checksum mismatch")
const request = JSON.parse(requestBytes)
const manifest = await readFile(join(directory, "manifest.json"))
const install = await readFile(join(directory, "install.sh"))
if (sha256(manifest) !== request.manifestSha256 || sha256(install) !== request.installSha256) throw new Error("signing document checksum mismatch")
const data = JSON.parse(manifest)
const setup = data.files.find(entry => entry.path === "bin/chariox-setup")
if (data.platform !== "linux-x64" || !/^[a-f0-9]{64}$/.test(setup?.sha256 ?? "")) throw new Error("missing Linux Setup manifest entry")
const key = createPrivateKey(await readFile(signingKey))
const publicKey = createPublicKey(key).export({ format: "jwk" })
if (publicKey.crv !== "Ed25519" || Buffer.from(publicKey.x, "base64url").toString("hex") !== request.publicKey) throw new Error("release public pin mismatch")
await mkdir(output)
for (const [file, bytes] of [["manifest.json", manifest], ["install.sh", install], ["setup", Buffer.from("chariox-linux-setup-sha256:" + setup.sha256)]]) {
  await writeFile(join(output, file + ".sig"), sign(null, bytes, key).toString("hex"), { flag: "wx" })
}
