import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { createHash, generateKeyPairSync, verify } from "node:crypto"
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import test from "node:test"
import { PLATFORMS, verifyBundle } from "./release-bundle.mjs"

const root = fileURLToPath(new URL("../", import.meta.url))
const sha256 = bytes => createHash("sha256").update(bytes).digest("hex")

test("Linux Setup is prepared, signed using only documents, then assembled and verified locally", async t => {
  const scratch = await mkdtemp(join(tmpdir(), "prepare-linux-setup-"))
  t.after(() => rm(scratch, { recursive: true, force: true }))
  const { privateKey, publicKey } = generateKeyPairSync("ed25519")
  const pin = publicKey.export({ format: "der", type: "spki" }).subarray(-32).toString("hex")
  const pem = join(scratch, "test-signing.pem")
  await writeFile(pem, privateKey.export({ format: "pem", type: "pkcs8" }), { mode: 0o600 })
  const executables = join(scratch, "executables"), output = join(scratch, "output")
  await mkdir(join(executables, "bin"), { recursive: true })
  for (const name of PLATFORMS["linux-x64"].bin) {
    await writeFile(join(executables, "bin", name), "#!/bin/sh\nexit 0\n", { mode: 0o755 })
  }
  const version = "0.1.0-staging.1"
  const releaseBase = "https://setup-assets.example.test/download"
  const control = "https://setup-validation.example.test"
  const run = (...args) => execFileSync(process.execPath, args, { cwd: root, timeout: 120_000 })
  const common = ["--version", version, "--public-key", pin, "--output", output]
  run("scripts/prepare-linux-setup.mjs", "prepare", ...common, "--executables", executables,
    "--release-base", releaseBase, "--control-plane-url", control)
  const docs = join(output, "signing"), signed = join(scratch, "signed")
  assert.deepEqual((await readdir(docs)).sort(), ["install.sh", "manifest.json", "request.json"])
  const request = sha256(await readFile(join(docs, "request.json")))
  assert.throws(() => run("scripts/sign-linux-setup-documents.mjs", docs, "0".repeat(64), pem, signed), /checksum mismatch/)
  run("scripts/sign-linux-setup-documents.mjs", docs, request, pem, signed)
  run("scripts/prepare-linux-setup.mjs", "assemble", ...common, "--signed", signed)
  const name = `chariox-${version}-linux-x64`, setup = `chariox-setup-${version}-linux-x64`
  const assets = join(output, "assets")
  assert.deepEqual((await readdir(assets)).sort(),
    ["install.sh", setup, setup + ".sig", name + ".manifest.json", name + ".manifest.sig", name + ".tar.gz"].sort())
  const bytes = await readFile(join(assets, setup)), signature = await readFile(join(assets, setup + ".sig"), "utf8")
  assert.ok(bytes.includes(Buffer.from(control)))
  assert.ok(verify(null, Buffer.from("chariox-linux-setup-sha256:" + sha256(bytes)), publicKey, Buffer.from(signature, "hex")))
  const bootstrap = await readFile(join(assets, "install.sh"), "utf8")
  assert.ok(bootstrap.includes(pin))
  assert.ok(bootstrap.includes(releaseBase))
  assert.match(bootstrap, /Signed macOS Setup is not available/)
  assert.ok(bootstrap.includes(`version='${version}'`))
  assert.equal((await verifyBundle(join(output, name), pin)).version, version)
})
