#!/usr/bin/env node

import { createHash, createPublicKey, verify } from "node:crypto"
import { createReadStream } from "node:fs"
import { lstat, readFile, readdir, realpath } from "node:fs/promises"
import { basename, dirname, join, relative, resolve, sep } from "node:path"

const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex")
const BUILD_TARGET = "x86_64-unknown-linux-gnu"
const SLICE_RELAY_PATH = "/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/chariox-relay"
function fail(message) {
  throw new Error(message)
}

async function readRegularFile(path, label, maxBytes) {
  const metadata = await lstat(path).catch((error) => fail(`${label} cannot be read: ${error.message}`))
  if (metadata.isSymbolicLink() || !metadata.isFile() || metadata.size > maxBytes) {
    fail(`${label} must be a bounded regular file`)
  }
  return readFile(path)
}

async function rejectLinkedOrSpecialEntries(current, label) {
  for (const entry of await readdir(current, { withFileTypes: true })) {
    const path = join(current, entry.name)
    const metadata = await lstat(path).catch((error) =>
      fail(`${label} cannot be inspected: ${error.message}`),
    )
    if (metadata.isSymbolicLink()) fail(`${label} contains a symbolic link`)
    if (metadata.isDirectory()) {
      await rejectLinkedOrSpecialEntries(path, label)
    } else if (!metadata.isFile()) {
      fail(`${label} contains an unsupported file type`)
    }
  }
}

// Tree artifacts already authenticate all their descendants.
// Outside them the release contains only declared artifacts, signature metadata,
// and the directories needed to reach those entries.
async function verifyReleaseRootInventory(rootfs, artifacts, treePaths) {
  const files = new Set([
    "usr/lib/chariox/release-manifest.json",
    "usr/lib/chariox/release-manifest.sig",
    "usr/lib/chariox/release-public-key",
    ...artifacts.map((artifact) => artifact.path.slice(1)),
  ])
  const trees = new Set(treePaths.map((path) => path.slice(1)))
  const directories = new Set(trees)
  for (const path of files) {
    for (let parent = dirname(path); parent !== "."; parent = dirname(parent)) {
      directories.add(parent)
    }
  }
  async function inspect(directory, prefix = "") {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = prefix ? `${prefix}/${entry.name}` : entry.name
      if (entry.isDirectory() && directories.has(path)) {
        if (!trees.has(path)) await inspect(join(directory, entry.name), path)
      } else if (!entry.isFile() || !files.has(path) || trees.has(path)) {
        fail(`release root contains an undeclared entry: ${path}`)
      }
    }
  }
  await inspect(rootfs)
}

async function sha256File(path) {
  const hash = createHash("sha256")
  for await (const chunk of createReadStream(path)) hash.update(chunk)
  return `sha256:${hash.digest("hex")}`
}

async function updateTreeHash(root, current, hash) {
  for (const entry of await readdir(current, { withFileTypes: true }).then((entries) =>
    entries.sort((left, right) => left.name < right.name ? -1 : left.name > right.name ? 1 : 0),
  )) {
    const path = join(current, entry.name)
    const pathFromRoot = relative(root, path).split(sep).join("/")
    const metadata = await lstat(path)
    const mode = metadata.mode & 0o7777
    if (entry.isDirectory()) {
      hash.update(`directory:${Buffer.byteLength(pathFromRoot)}:${pathFromRoot}:${mode}:`)
      await updateTreeHash(root, path, hash)
      continue
    }
    if (!entry.isFile()) fail("release artifact tree contains an unsupported file type")
    hash.update(`file:${Buffer.byteLength(pathFromRoot)}:${pathFromRoot}:${mode}:${metadata.size}:`)
    for await (const chunk of createReadStream(path)) hash.update(chunk)
  }
}

async function sha256Tree(root) {
  const metadata = await lstat(root).catch((error) =>
    fail(`release artifact tree cannot be read: ${error.message}`),
  )
  if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
    fail("release artifact tree must be a directory, not a symlink")
  }
  const hash = createHash("sha256")
  hash.update(`directory:1:.:${metadata.mode & 0o7777}:`)
  await updateTreeHash(root, root, hash)
  return `sha256:${hash.digest("hex")}`
}

function decodePublicKey(bytes, label) {
  const text = bytes.toString("utf8").trim()
  const raw = Buffer.from(text, "base64")
  if (raw.length !== 32 || raw.toString("base64") !== text) fail(`${label} is not canonical base64`)
  return raw
}

function artifactPath(rootfs, absolutePath) {
  if (!absolutePath.startsWith("/") || absolutePath.includes("..")) fail("release artifact path is unsafe")
  const path = resolve(rootfs, absolutePath.slice(1))
  const rootPrefix = `${resolve(rootfs)}${sep}`
  if (!path.startsWith(rootPrefix)) fail("release artifact path escapes the image root")
  return path
}

function validateObjectKeys(value, expected, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) fail(`${label} is invalid`)
  const actual = Object.keys(value).sort()
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    fail(`${label} contains unsupported fields`)
  }
}

async function verifyBuilderAttestation(
  rootfs,
  canonicalRootfs,
  manifest,
  verifiedArtifactDigests,
  trustedBuilderPublicKeyPath,
) {
  if (!trustedBuilderPublicKeyPath) {
    fail("Release verification requires an independently supplied trusted builder public key")
  }
  const canonicalTrustedBuilderKey = await realpath(trustedBuilderPublicKeyPath).catch((error) =>
    fail(`trusted builder public key cannot be resolved: ${error.message}`),
  )
  if (
    canonicalTrustedBuilderKey === canonicalRootfs ||
    canonicalTrustedBuilderKey.startsWith(`${canonicalRootfs}${sep}`)
  ) {
    fail("trusted builder public key must be supplied outside the image root")
  }

  const trustedBuilderRawKey = decodePublicKey(
    await readRegularFile(canonicalTrustedBuilderKey, "trusted builder public key", 1024),
    "trusted builder public key",
  )
  const packagedBuilderRawKey = decodePublicKey(
    await readRegularFile(join(rootfs, "/usr/lib/chariox/builder-public-key"), "packaged builder public key", 1024),
    "packaged builder public key",
  )
  if (!trustedBuilderRawKey.equals(packagedBuilderRawKey)) {
    fail("packaged builder public key is not independently trusted")
  }

  const attestationPath = join(rootfs, "/usr/lib/chariox/build-attestation.json")
  const attestationBytes = await readRegularFile(attestationPath, "builder attestation", 64 * 1024)
  let attestation
  try {
    attestation = JSON.parse(attestationBytes)
  } catch {
    fail("builder attestation is invalid JSON")
  }
  if (!Buffer.from(JSON.stringify(attestation)).equals(attestationBytes)) {
    fail("builder attestation is not canonical JSON")
  }
  validateObjectKeys(
    attestation,
    ["artifacts", "schemaVersion", "sourceCommit", "sourceTree", "target"],
    "builder attestation",
  )
  // A release with the App artifact set attests its two App binaries after the runtime binaries.
  const attestsAppBinaries = verifiedArtifactDigests.has("chariox-app-package")
  if (
    attestation.schemaVersion !== 1 ||
    attestation.sourceCommit !== manifest.sourceCommit ||
    attestation.sourceTree !== manifest.sourceTree ||
    attestation.target !== BUILD_TARGET ||
    !Array.isArray(attestation.artifacts) ||
    attestation.artifacts.length !== (attestsAppBinaries ? 5 : 3)
  ) {
    fail("builder attestation source identity or target does not match the signed release")
  }

  const relayPath = artifactPath(rootfs, SLICE_RELAY_PATH)
  await readRegularFile(relayPath, "builder-attested slice relay", Number.MAX_SAFE_INTEGER)
  const expectedArtifacts = [
    ["chariox-kernel", verifiedArtifactDigests.get("chariox-kernel")],
    ["chariox-managed-bootstrap", verifiedArtifactDigests.get("chariox-managed-bootstrap")],
    ["chariox-relay", await sha256File(relayPath)],
    ...(attestsAppBinaries ? [
      ["chariox-app-package", verifiedArtifactDigests.get("chariox-app-package")],
      ["chariox-app-wasm-worker", verifiedArtifactDigests.get("chariox-app-wasm-worker")],
    ] : []),
  ]
  for (const [index, artifact] of attestation.artifacts.entries()) {
    validateObjectKeys(artifact, ["name", "sha256"], "builder attestation artifact")
    const [expectedName, expectedDigest] = expectedArtifacts[index]
    if (
      artifact.name !== expectedName ||
      !/^sha256:[a-f0-9]{64}$/.test(artifact.sha256 ?? "") ||
      artifact.sha256 !== expectedDigest
    ) {
      fail("builder attestation artifacts do not match the signed release")
    }
  }

  const signatureText = (
    await readRegularFile(
      join(rootfs, "/usr/lib/chariox/build-attestation.sig"),
      "builder attestation signature",
      1024,
    )
  ).toString("utf8").trim()
  const signature = Buffer.from(signatureText, "base64")
  if (signature.length !== 64 || signature.toString("base64") !== signatureText) {
    fail("builder attestation signature is not canonical base64")
  }
  const builderPublicKey = createPublicKey({
    key: Buffer.concat([ED25519_SPKI_PREFIX, trustedBuilderRawKey]),
    format: "der",
    type: "spki",
  })
  if (!verify(null, attestationBytes, builderPublicKey, signature)) {
    fail("builder attestation signature is invalid")
  }
}

async function verifyRootfs(rootfs, expectedDigest, trustedKeyPath, trustedBuilderPublicKeyPath) {
  if (!/^sha256:[a-f0-9]{64}$/.test(expectedDigest)) fail("expected release digest is invalid")
  const rootfsMetadata = await lstat(rootfs).catch((error) => fail(`image root cannot be read: ${error.message}`))
  if (rootfsMetadata.isSymbolicLink() || !rootfsMetadata.isDirectory()) {
    fail("image root must be a directory, not a symlink")
  }
  await rejectLinkedOrSpecialEntries(rootfs, "image root")
  const canonicalRootfs = await realpath(rootfs)
  const canonicalTrustedKey = await realpath(trustedKeyPath).catch((error) =>
    fail(`trusted release public key cannot be resolved: ${error.message}`),
  )
  if (
    canonicalTrustedKey === canonicalRootfs ||
    canonicalTrustedKey.startsWith(`${canonicalRootfs}${sep}`)
  ) {
    fail("trusted release public key must be supplied outside the image root")
  }
  const manifestPath = join(rootfs, "usr/lib/chariox/release-manifest.json")
  const signaturePath = join(rootfs, "usr/lib/chariox/release-manifest.sig")
  const packagedKeyPath = join(rootfs, "usr/lib/chariox/release-public-key")
  const manifestBytes = await readRegularFile(manifestPath, "release manifest", 64 * 1024)
  const actualDigest = `sha256:${createHash("sha256").update(manifestBytes).digest("hex")}`
  if (actualDigest !== expectedDigest) fail("release manifest digest does not match the expected image release")

  const trustedRawKey = decodePublicKey(
    await readRegularFile(canonicalTrustedKey, "trusted release public key", 1024),
    "trusted release public key",
  )
  const packagedRawKey = decodePublicKey(
    await readRegularFile(packagedKeyPath, "packaged release public key", 1024),
    "packaged release public key",
  )
  if (!trustedRawKey.equals(packagedRawKey)) fail("packaged release public key is not trusted")
  const signatureText = (
    await readRegularFile(signaturePath, "release signature", 1024)
  ).toString("utf8").trim()
  const signature = Buffer.from(signatureText, "base64")
  if (signature.length !== 64 || signature.toString("base64") !== signatureText) {
    fail("release signature is not canonical base64")
  }
  const publicKey = createPublicKey({
    key: Buffer.concat([ED25519_SPKI_PREFIX, trustedRawKey]),
    format: "der",
    type: "spki",
  })
  if (!verify(null, manifestBytes, publicKey, signature)) fail("release signature is invalid")

  let manifest
  try {
    manifest = JSON.parse(manifestBytes)
  } catch {
    fail("release manifest is invalid JSON")
  }
  validateObjectKeys(manifest, ["artifacts", "schemaVersion", "sourceCommit", "sourceTree"], "release manifest")
  if (manifest.schemaVersion !== 2 || !/^[a-f0-9]{40}$/.test(manifest.sourceCommit) || !/^[a-f0-9]{40}$/.test(manifest.sourceTree) || !Array.isArray(manifest.artifacts) || !manifest.artifacts.length) fail("release manifest schema is unsupported")
  const seen = new Set(), paths = new Set(), trees = [], verifiedArtifactDigests = new Map()
  for (const artifact of manifest.artifacts) {
    validateObjectKeys(artifact, ["name", "path", "sha256"], "release artifact")
    if (typeof artifact.name !== "string" || !artifact.name || seen.has(artifact.name) || paths.has(artifact.path) || !/^sha256:[a-f0-9]{64}$/.test(artifact.sha256)) fail("release artifact is invalid or duplicated")
    const path = artifactPath(rootfs, artifact.path)
    const metadata = await lstat(path)
    const digest = metadata.isDirectory() ? await sha256Tree(path) : await readRegularFile(path, "release artifact", Number.MAX_SAFE_INTEGER).then(() => sha256File(path))
    if (digest !== artifact.sha256) fail(`release artifact ${artifact.name} is corrupted`)
    if (metadata.isDirectory()) trees.push(artifact.path)
    seen.add(artifact.name); paths.add(artifact.path); verifiedArtifactDigests.set(artifact.name, digest)
  }
  await verifyReleaseRootInventory(rootfs, manifest.artifacts, trees)
  await verifyBuilderAttestation(rootfs, canonicalRootfs, manifest, verifiedArtifactDigests, trustedBuilderPublicKeyPath)
}
try {
  if (process.argv.length !== 6) fail("usage: verify-rootfs <rootfs> <expected-release-digest> <trusted-release-public-key> <trusted-builder-public-key>")
  await verifyRootfs(...process.argv.slice(2))
} catch (error) { process.stderr.write(`verify-rootfs: ${error.message}\n`); process.exitCode = 1 }
