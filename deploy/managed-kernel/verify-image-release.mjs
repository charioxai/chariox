#!/usr/bin/env node

import { createHash, createPublicKey, verify } from "node:crypto"
import { createReadStream } from "node:fs"
import { lstat, readFile, realpath } from "node:fs/promises"
import { basename, join, resolve, sep } from "node:path"

const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex")
const EXPECTED_ARTIFACTS = new Map([
  ["chariox-kernel", "/usr/local/bin/chariox-kernel"],
  ["chariox-managed-bootstrap", "/usr/local/bin/chariox-managed-bootstrap"],
  [
    "chariox-managed-bootstrap.service",
    "/etc/systemd/system/chariox-managed-bootstrap.service",
  ],
])

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

async function sha256File(path) {
  const hash = createHash("sha256")
  for await (const chunk of createReadStream(path)) hash.update(chunk)
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

async function verifyImageRelease(rootfs, expectedDigest, trustedKeyPath) {
  if (!/^sha256:[a-f0-9]{64}$/.test(expectedDigest)) fail("expected release digest is invalid")
  const rootfsMetadata = await lstat(rootfs).catch((error) => fail(`image root cannot be read: ${error.message}`))
  if (rootfsMetadata.isSymbolicLink() || !rootfsMetadata.isDirectory()) {
    fail("image root must be a directory, not a symlink")
  }
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
  validateObjectKeys(manifest, ["artifacts", "schemaVersion"], "release manifest")
  if (manifest.schemaVersion !== 1 || !Array.isArray(manifest.artifacts)) {
    fail("release manifest schema is unsupported")
  }
  if (manifest.artifacts.length !== EXPECTED_ARTIFACTS.size) {
    fail("release manifest does not contain the exact image artifacts")
  }
  const seen = new Set()
  for (const artifact of manifest.artifacts) {
    validateObjectKeys(artifact, ["name", "path", "sha256"], "release artifact")
    const expectedPath = EXPECTED_ARTIFACTS.get(artifact.name)
    if (!expectedPath || artifact.path !== expectedPath || seen.has(artifact.name)) {
      fail("release manifest contains an unexpected or duplicate artifact")
    }
    if (!/^sha256:[a-f0-9]{64}$/.test(artifact.sha256)) fail("release artifact digest is invalid")
    const path = artifactPath(rootfs, artifact.path)
    await readRegularFile(path, `release artifact ${artifact.name}`, Number.MAX_SAFE_INTEGER)
    if ((await sha256File(path)) !== artifact.sha256) fail(`release artifact ${artifact.name} is corrupted`)
    seen.add(artifact.name)
  }
}

try {
  if (process.argv.length !== 5) {
    fail("usage: verify-image-release <rootfs> <expected-release-digest> <trusted-public-key>")
  }
  await verifyImageRelease(resolve(process.argv[2]), process.argv[3], resolve(process.argv[4]))
} catch (error) {
  const message = error instanceof Error ? error.message : String(error)
  process.stderr.write(`${basename(process.argv[1])}: ${message}\n`)
  process.exitCode = 1
}
