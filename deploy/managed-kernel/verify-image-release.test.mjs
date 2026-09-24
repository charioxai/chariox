import assert from "node:assert/strict"
import { createHash, generateKeyPairSync, sign } from "node:crypto"
import { chmod, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import { dirname, join } from "node:path"
import { tmpdir } from "node:os"
import { spawnSync } from "node:child_process"
import test from "node:test"

const verifier = new URL("./verify-image-release.mjs", import.meta.url)
const ARTIFACTS = [
  ["chariox-kernel", "/usr/local/bin/chariox-kernel", "file"],
  ["chariox-managed-bootstrap", "/usr/local/bin/chariox-managed-bootstrap", "file"],
  ["chariox-managed-bootstrap.service", "/etc/systemd/system/chariox-managed-bootstrap.service", "file"],
  ["chariox-path1-managed-bootstrap.service", "/etc/systemd/system/chariox-path1-managed-bootstrap.service", "file"],
  ["chariox-disposable-worker-bootstrap.service", "/etc/systemd/system/chariox-disposable-worker-bootstrap.service", "file"],
  ["chariox-rootless-docker.service", "/etc/systemd/system/chariox-rootless-docker.service", "file"],
  ["chariox-slice-broker.service", "/etc/systemd/system/chariox-slice-broker.service", "file"],
  ["chariox-slice-build-context", "/usr/lib/chariox/slice-build-context", "tree"],
  ["chariox-build-attestation", "/usr/lib/chariox/build-attestation.json", "file"],
  ["chariox-build-attestation-signature", "/usr/lib/chariox/build-attestation.sig", "file"],
  ["chariox-builder-public-key", "/usr/lib/chariox/builder-public-key", "file"],
]

function sha256(bytes) {
  return `sha256:${createHash("sha256").update(bytes).digest("hex")}`
}

function rawPublicKey(key) {
  return key.export({ format: "der", type: "spki" }).subarray(-32).toString("base64")
}

async function put(root, absolutePath, bytes, mode = 0o644) {
  const path = join(root, absolutePath.slice(1))
  await mkdir(dirname(path), { recursive: true, mode: 0o755 })
  await writeFile(path, bytes, { mode })
  await chmod(path, mode)
  return path
}

async function emptyTreeDigest(path) {
  await mkdir(path, { recursive: true, mode: 0o755 })
  await chmod(path, 0o755)
  return sha256(Buffer.from(`directory:1:.:${0o755}:`))
}

test("image verification rejects an invalid embedded builder attestation", async (context) => {
  const scratch = await mkdtemp(join(tmpdir(), "chariox-release-provenance-test-"))
  context.after(() => rm(scratch, { recursive: true, force: true }))
  const rootfs = join(scratch, "rootfs")
  await mkdir(rootfs, { mode: 0o755 })

  const releaseKeys = generateKeyPairSync("ed25519")
  const trustedReleaseKey = join(scratch, "trusted-release-public-key")
  await writeFile(trustedReleaseKey, rawPublicKey(releaseKeys.publicKey))
  const embeddedBuilderKeys = generateKeyPairSync("ed25519")
  const builderPublicKey = Buffer.from(rawPublicKey(embeddedBuilderKeys.publicKey))
  const contentByName = new Map([
    ["chariox-kernel", Buffer.from("kernel artifact")],
    ["chariox-managed-bootstrap", Buffer.from("bootstrap artifact")],
    ["chariox-managed-bootstrap.service", Buffer.from("[Service]\nExecStart=/usr/local/bin/chariox-managed-bootstrap\n")],
    ["chariox-path1-managed-bootstrap.service", Buffer.from("[Service]\nExecStart=/usr/local/bin/chariox-managed-bootstrap\n")],
    ["chariox-disposable-worker-bootstrap.service", Buffer.from("[Service]\nExecStart=/usr/local/bin/chariox-managed-bootstrap\n")],
    ["chariox-rootless-docker.service", Buffer.from("[Service]\n")],
    ["chariox-slice-broker.service", Buffer.from("[Service]\n")],
    ["chariox-build-attestation", Buffer.from("not an attestation")],
    ["chariox-build-attestation-signature", Buffer.from("not a signature")],
    ["chariox-builder-public-key", builderPublicKey],
  ])
  const artifacts = []
  for (const [name, path, type] of ARTIFACTS) {
    let digest
    if (type === "tree") {
      digest = await emptyTreeDigest(join(rootfs, path.slice(1)))
    } else {
      const contents = contentByName.get(name)
      await put(rootfs, path, contents)
      digest = sha256(contents)
    }
    artifacts.push({ name, path, sha256: digest })
  }

  const manifestBytes = Buffer.from(JSON.stringify({
    schemaVersion: 2,
    sourceCommit: "a".repeat(40),
    sourceTree: "b".repeat(40),
    artifacts,
  }))
  await put(rootfs, "/usr/lib/chariox/release-public-key", Buffer.from(rawPublicKey(releaseKeys.publicKey)))
  await put(rootfs, "/usr/lib/chariox/release-manifest.json", manifestBytes)
  await put(
    rootfs,
    "/usr/lib/chariox/release-manifest.sig",
    Buffer.from(sign(null, manifestBytes, releaseKeys.privateKey).toString("base64")),
  )

  const result = spawnSync(
    process.execPath,
    [verifier.pathname, rootfs, sha256(manifestBytes), trustedReleaseKey],
    { encoding: "utf8", timeout: 10_000 },
  )
  assert.notEqual(
    result.status,
    0,
    `verifier accepted a valid release signature over an invalid builder attestation: ${result.stderr}`,
  )
})
