import assert from "node:assert/strict"
import { generateKeyPairSync } from "node:crypto"
import { chmod, cp, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import { spawnSync } from "node:child_process"
import { test } from "node:test"

import { rawPublicKey, sha, signingRequestBytes } from "./release-signing-documents.mjs"

const scripts = fileURLToPath(new URL(".", import.meta.url))
const run = (script, args, env = {}) =>
  spawnSync(process.execPath, [join(scripts, script), ...args], { encoding: "utf8", env: { ...process.env, SOURCE_DATE_EPOCH: "946684800", ...env } })
const commit = "a".repeat(40)
const tree = "b".repeat(40)

async function throwawayKey(root, name) {
  const { privateKey } = generateKeyPairSync("ed25519")
  const path = join(root, `${name}.throwaway.pem`)
  await writeFile(path, privateKey.export({ format: "pem", type: "pkcs8" }), { mode: 0o600 })
  return { path, pin: rawPublicKey(privateKey).toString("base64") }
}

async function filesUnder(root, current = root) {
  const found = new Map()
  for (const entry of await readdir(current, { withFileTypes: true })) {
    const path = join(current, entry.name)
    if (entry.isDirectory()) for (const [name, bytes] of await filesUnder(root, path)) found.set(name, bytes)
    else found.set(path.slice(root.length + 1), await readFile(path))
  }
  return found
}

// A staged boot release from the real packager, with its signing request.
async function stageBoot(root, name = "boot") {
  const source = join(root, "source")
  const output = join(root, name)
  const request = join(root, `${name}-request.json`)
  const bin = join(root, "bin")
  if (!(await readdir(root)).includes("source")) {
    await mkdir(source)
    await mkdir(bin)
    await writeFile(join(source, "file"), "x\n")
    for (const git of [["init", "-q"], ["add", "."], ["-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-q", "-m", "t"]]) {
      assert.equal(spawnSync("git", ["-C", source, ...git]).status, 0)
    }
    for (const binary of ["kernel", "bootstrap", "worker", "relay"]) await writeFile(join(bin, binary), `${binary}\n`)
  }
  const common = ["--kernel", join(bin, "kernel"), "--bootstrap", join(bin, "bootstrap"), "--worker", join(bin, "worker"), "--relay", join(bin, "relay"), "--source", source, "--output", output]
  return { common, output, request, source }
}

test("boot split flow yields byte-identical output to signing in place, and nothing large crosses", async () => {
  const root = await mkdtemp(join(tmpdir(), "signsplit-boot-"))
  try {
    const release = await throwawayKey(root, "release")
    const staged = await stageBoot(root)
    const stage = run("package-boot-release.mjs", [...staged.common, "--signing-request", staged.request])
    assert.equal(stage.status, 0, stage.stderr)
    const { requestDigest } = JSON.parse(stage.stdout)
    const requestBytes = await readFile(staged.request)
    assert.ok(requestBytes.length < 4096, "signing request must stay small")
    assert.equal(sha(requestBytes), requestDigest)

    const signedDirectory = join(root, "signed")
    const sign = run("sign-digests-on-mac.mjs", ["--request", `${staged.request}@${requestDigest}`, "--release-key", release.path, "--output", signedDirectory, "--test-allow-non-mac"])
    assert.equal(sign.status, 0, sign.stderr)
    const returned = [...(await filesUnder(signedDirectory)).values()].reduce((total, bytes) => total + bytes.length, 0)
    assert.ok(returned < 8192, "signer output must stay small")

    const assemble = run("assemble-signed-release.mjs", ["--kind", "boot", "--tree", staged.output, "--request", staged.request, "--signed", signedDirectory, "--release-public-key", release.pin])
    assert.equal(assemble.status, 0, assemble.stderr)

    const direct = await stageBoot(root, "boot-direct")
    const inPlace = run("package-boot-release.mjs", [...direct.common, "--signing-key", release.path])
    assert.equal(inPlace.status, 0, inPlace.stderr)
    const [split, whole] = [await filesUnder(staged.output), await filesUnder(direct.output)]
    assert.deepEqual([...split.keys()].sort(), [...whole.keys()].sort())
    for (const [name, bytes] of whole) assert.ok(bytes.equals(split.get(name)), `${name} differs between the split flow and in-place signing`)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("the signer refuses off the Mac, with non-throwaway keys, and on any request digest mismatch", async () => {
  const root = await mkdtemp(join(tmpdir(), "signsplit-refuse-"))
  try {
    const release = await throwawayKey(root, "release")
    const staged = await stageBoot(root)
    const { requestDigest } = JSON.parse(run("package-boot-release.mjs", [...staged.common, "--signing-request", staged.request]).stdout)
    const args = (extra = []) => ["--request", `${staged.request}@${requestDigest}`, "--release-key", release.path, "--output", join(root, `out-${Math.random()}`), ...extra]

    assert.match(run("sign-digests-on-mac.mjs", args()).stderr, /coordinator Mac only/)
    const real = join(root, "release-key.pem")
    await cp(release.path, real)
    assert.match(run("sign-digests-on-mac.mjs", ["--request", `${staged.request}@${requestDigest}`, "--release-key", real, "--output", join(root, "o1"), "--test-allow-non-mac"]).stderr, /only accepts keys named \*\.throwaway\.pem/)
    const wrong = `sha256:${"0".repeat(64)}`
    assert.match(run("sign-digests-on-mac.mjs", ["--request", `${staged.request}@${wrong}`, "--release-key", release.path, "--output", join(root, "o2"), "--test-allow-non-mac"]).stderr, /signing request digest mismatch/)

    const tampered = join(root, "tampered-request.json")
    const bytes = await readFile(staged.request)
    await writeFile(tampered, bytes.toString().replace(commit.slice(0, 1), "f"))
    const result = run("sign-digests-on-mac.mjs", ["--request", `${tampered}@${requestDigest}`, "--release-key", release.path, "--output", join(root, "o3"), "--test-allow-non-mac"])
    assert.match(result.stderr, /signing request digest mismatch/)
    assert.deepEqual((await readdir(root)).filter((name) => name.startsWith("o")), [])
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

// Managed releases: a synthetic staged rootfs with the two directory entries.
async function stageManaged(root) {
  const staged = join(root, "managed")
  const lib = join(staged, "rootfs/usr/lib/chariox")
  await mkdir(join(lib, "slice-image"), { recursive: true })
  await mkdir(join(lib, "slice-build-context"), { recursive: true })
  await mkdir(join(staged, "rootfs/usr/local/bin"), { recursive: true })
  await writeFile(join(lib, "slice-image/slice-image.json"), "{}\n")
  await writeFile(join(lib, "slice-build-context/Cargo.toml"), "[workspace]\n")
  const names = ["chariox-kernel", "chariox-managed-bootstrap", "chariox-app-package", "chariox-app-wasm-worker"]
  for (const name of names) await writeFile(join(staged, "rootfs/usr/local/bin", name), `${name}\n`)
  await writeFile(join(lib, "build-attestation.json"), "attestation placeholder\n")
  const { sha256File, sha256Tree } = await import("./release-tree-digest.mjs")
  const digests = Object.fromEntries(await Promise.all(names.map(async (name) => [name, await sha256File(join(staged, "rootfs/usr/local/bin", name))])))
  const relay = sha(Buffer.from("relay"))
  const attestationBytes = Buffer.from(JSON.stringify({ schemaVersion: 1, sourceCommit: commit, sourceTree: tree, target: "x86_64-unknown-linux-gnu", artifacts: [
    { name: "chariox-kernel", sha256: digests["chariox-kernel"] }, { name: "chariox-managed-bootstrap", sha256: digests["chariox-managed-bootstrap"] }, { name: "chariox-relay", sha256: relay },
    { name: "chariox-app-package", sha256: digests["chariox-app-package"] }, { name: "chariox-app-wasm-worker", sha256: digests["chariox-app-wasm-worker"] },
  ] }))
  await writeFile(join(lib, "build-attestation.json"), attestationBytes)
  const sliceImage = await sha256Tree(join(lib, "slice-image"))
  const sliceBuildContext = await sha256Tree(join(lib, "slice-build-context"))
  const entries = [
    ...names.map((name) => ({ name, path: `/usr/local/bin/${name}`, sha256: digests[name] })),
    { name: "chariox-slice-image", path: "/usr/lib/chariox/slice-image", sha256: sliceImage },
    { name: "chariox-slice-build-context", path: "/usr/lib/chariox/slice-build-context", sha256: sliceBuildContext },
    { name: "chariox-build-attestation", path: "/usr/lib/chariox/build-attestation.json", sha256: sha(attestationBytes) },
    { name: "chariox-build-attestation-signature", path: "/usr/lib/chariox/build-attestation.sig", sha256: null },
    { name: "chariox-builder-public-key", path: "/usr/lib/chariox/builder-public-key", sha256: null },
  ]
  const request = join(root, "managed-request.json")
  const bytes = signingRequestBytes("managed", { sourceCommit: commit, sourceTree: tree, attestationBytes, entries, sliceBuildContext, sliceImage })
  await writeFile(request, bytes)
  return { staged, request, requestDigest: sha(bytes) }
}

test("managed split flow signs with separate builder and release keys and refuses every tampering", async () => {
  const root = await mkdtemp(join(tmpdir(), "signsplit-managed-"))
  try {
    const [release, builder, other] = await Promise.all(["release", "builder", "other"].map((name) => throwawayKey(root, name)))
    const { staged, request, requestDigest } = await stageManaged(root)
    const signedDirectory = join(root, "signed")
    const sign = run("sign-digests-on-mac.mjs", ["--request", `${request}@${requestDigest}`, "--release-key", release.path, "--builder-key", builder.path, "--output", signedDirectory, "--test-allow-non-mac"])
    assert.equal(sign.status, 0, sign.stderr)
    const receipt = JSON.parse(await readFile(join(signedDirectory, "SIGNING_RECEIPT.json")))
    assert.equal(receipt.testMode, true)
    assert.equal(receipt.keys.builder.fingerprint, sha(Buffer.from(builder.pin, "base64")))
    const pins = ["--release-public-key", release.pin, "--builder-public-key", builder.pin]
    const assemble = (stagedTree, signed = signedDirectory, extra = pins) =>
      run("assemble-signed-release.mjs", ["--kind", "managed", "--tree", stagedTree, "--request", request, "--signed", signed, ...extra])

    const copy = async (name) => {
      const path = join(root, name)
      await cp(staged, path, { recursive: true })
      return path
    }
    // wrong key: the pinned builder key is another key
    assert.match(assemble(await copy("t-key"), signedDirectory, ["--release-public-key", release.pin, "--builder-public-key", other.pin]).stderr, /not the pinned builder key/)
    // tampered artifact names the artifact
    const tamperedArtifact = await copy("t-artifact")
    await writeFile(join(tamperedArtifact, "rootfs/usr/local/bin/chariox-kernel"), "evil\n")
    assert.match(assemble(tamperedArtifact).stderr, /artifact chariox-kernel \(\/usr\/local\/bin\/chariox-kernel\) has digest/)
    // tampered slice image directory
    const tamperedImage = await copy("t-image")
    await writeFile(join(tamperedImage, "rootfs/usr/lib/chariox/slice-image/slice-image.json"), "{\"x\":1}\n")
    assert.match(assemble(tamperedImage).stderr, /artifact chariox-slice-image/)
    // tampered returned manifest or signature
    for (const file of ["release-manifest.json", "attestation.sig", "release-manifest.sig"]) {
      const tamperedSigned = join(root, `t-${file}`)
      await cp(signedDirectory, tamperedSigned, { recursive: true })
      const path = join(tamperedSigned, "managed", file)
      await writeFile(path, `${(await readFile(path, "utf8")).replace(/[A-Za-z]/, (c) => (c === "A" ? "B" : "A"))}`)
      assert.match(assemble(await copy(`t-${file}-tree`), tamperedSigned).stderr, /does not match the digest in SIGNING_RECEIPT\.json/, file)
    }
    // wrong request digest in the receipt
    const wrongReceipt = join(root, "t-receipt")
    await cp(signedDirectory, wrongReceipt, { recursive: true })
    const receiptPath = join(wrongReceipt, "SIGNING_RECEIPT.json")
    await writeFile(receiptPath, (await readFile(receiptPath, "utf8")).replace(requestDigest, `sha256:${"1".repeat(64)}`))
    assert.match(assemble(await copy("t-receipt-tree"), wrongReceipt).stderr, /signing request digest mismatch/)

    // the good path embeds all five signed files and a second assembly refuses
    const good = await copy("good")
    const done = assemble(good)
    assert.equal(done.status, 0, done.stderr)
    for (const name of ["build-attestation.sig", "builder-public-key", "release-manifest.json", "release-manifest.sig", "release-public-key"]) {
      assert.ok((await readFile(join(good, "rootfs/usr/lib/chariox", name))).length > 0, name)
    }
    assert.match(assemble(good).stderr, /already exists/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
