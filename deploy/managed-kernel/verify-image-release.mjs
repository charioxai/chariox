#!/usr/bin/env node

import { createHash, createPublicKey, verify } from "node:crypto"
import { createReadStream } from "node:fs"
import { lstat, readFile, readdir, realpath } from "node:fs/promises"
import { basename, join, relative, resolve, sep } from "node:path"

const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex")
const MANAGED_BUILD_TARGET = "x86_64-unknown-linux-gnu"
const SLICE_RELAY_PATH = "/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/chariox-relay"
const EXPECTED_ARTIFACTS = new Map([
  ["chariox-kernel", { path: "/usr/local/bin/chariox-kernel", type: "file" }],
  ["chariox-managed-bootstrap", { path: "/usr/local/bin/chariox-managed-bootstrap", type: "file" }],
  ["chariox-app-package", { path: "/usr/local/bin/chariox-app-package", type: "file" }],
  ["chariox-app-storage", { path: "/usr/libexec/chariox-app-storage", type: "file" }],
  ["chariox-app-storage.service", { path: "/etc/systemd/system/chariox-app-storage.service", type: "file" }],
  [
    "chariox-managed-bootstrap.service",
    { path: "/etc/systemd/system/chariox-managed-bootstrap.service", type: "file" },
  ],
  [
    "chariox-path1-managed-bootstrap.service",
    { path: "/etc/systemd/system/chariox-path1-managed-bootstrap.service", type: "file" },
  ],
  [
    "chariox-disposable-worker-bootstrap.service",
    { path: "/etc/systemd/system/chariox-disposable-worker-bootstrap.service", type: "file" },
  ],
  [
    "chariox-rootless-docker.service",
    { path: "/etc/systemd/system/chariox-rootless-docker.service", type: "file" },
  ],
  [
    "chariox-slice-broker.service",
    { path: "/etc/systemd/system/chariox-slice-broker.service", type: "file" },
  ],
  [
    "chariox-slice-build-context",
    { path: "/usr/lib/chariox/slice-build-context", type: "tree" },
  ],
  ["chariox-build-attestation", { path: "/usr/lib/chariox/build-attestation.json", type: "file" }],
  ["chariox-build-attestation-signature", { path: "/usr/lib/chariox/build-attestation.sig", type: "file" }],
  ["chariox-builder-public-key", { path: "/usr/lib/chariox/builder-public-key", type: "file" }],
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

async function verifyPath1BuilderAttestation(
  rootfs,
  canonicalRootfs,
  manifest,
  verifiedArtifactDigests,
  trustedBuilderPublicKeyPath,
) {
  if (!trustedBuilderPublicKeyPath) {
    fail("Path-1 release verification requires an independently supplied trusted builder public key")
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
  if (
    attestation.schemaVersion !== 1 ||
    attestation.sourceCommit !== manifest.sourceCommit ||
    attestation.sourceTree !== manifest.sourceTree ||
    attestation.target !== MANAGED_BUILD_TARGET ||
    !Array.isArray(attestation.artifacts) ||
    attestation.artifacts.length !== 5
  ) {
    fail("builder attestation source identity or target does not match the signed release")
  }

  const relayPath = artifactPath(rootfs, SLICE_RELAY_PATH)
  await readRegularFile(relayPath, "builder-attested slice relay", Number.MAX_SAFE_INTEGER)
  const expectedArtifacts = [
    ["chariox-kernel", verifiedArtifactDigests.get("chariox-kernel")],
    ["chariox-managed-bootstrap", verifiedArtifactDigests.get("chariox-managed-bootstrap")],
    ["chariox-relay", await sha256File(relayPath)],
    ["chariox-app-package", verifiedArtifactDigests.get("chariox-app-package")],
    ["chariox-app-storage", verifiedArtifactDigests.get("chariox-app-storage")],
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

async function verifyImageRelease(
  rootfs,
  expectedDigest,
  trustedKeyPath,
  selectedTopology,
  trustedBuilderPublicKeyPath,
) {
  if (!/^sha256:[a-f0-9]{64}$/.test(expectedDigest)) fail("expected release digest is invalid")
  if (selectedTopology !== undefined && !["path1", "shared_host"].includes(selectedTopology)) {
    fail("selected managed provider topology must be path1 or shared_host")
  }
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
  validateObjectKeys(
    manifest,
    ["artifacts", "schemaVersion", "sourceCommit", "sourceTree"],
    "release manifest",
  )
  if (
    manifest.schemaVersion !== 2 ||
    !/^[a-f0-9]{40}$/.test(manifest.sourceCommit) ||
    !/^[a-f0-9]{40}$/.test(manifest.sourceTree) ||
    !Array.isArray(manifest.artifacts)
  ) {
    fail("release manifest schema is unsupported")
  }
  // Older schema 2 releases predate the worker and Path-1 home units. Keep them
  // verifiable for shared-host rollback, but never select an undeclared unit.
  const hasWorkerService = manifest.artifacts.some(
    (artifact) => artifact?.name === "chariox-disposable-worker-bootstrap.service",
  )
  const hasPath1Service = manifest.artifacts.some(
    (artifact) => artifact?.name === "chariox-path1-managed-bootstrap.service",
  )
  const expectedArtifactCount = EXPECTED_ARTIFACTS.size
    - (hasWorkerService ? 0 : 1)
    - (hasPath1Service ? 0 : 1)
  if (!hasWorkerService) {
    const workerPath = artifactPath(rootfs, EXPECTED_ARTIFACTS.get("chariox-disposable-worker-bootstrap.service").path)
    const workerExists = await lstat(workerPath).then(() => true, (error) => {
      if (error.code !== "ENOENT") throw error
      return false
    })
    if (workerExists) fail("release contains an undeclared worker service")
  }
  if (!hasPath1Service) {
    const path1Path = artifactPath(rootfs, EXPECTED_ARTIFACTS.get("chariox-path1-managed-bootstrap.service").path)
    const path1Exists = await lstat(path1Path).then(() => true, (error) => {
      if (error.code !== "ENOENT") throw error
      return false
    })
    if (path1Exists) fail("release contains an undeclared Path-1 managed-home service")
  }
  if (manifest.artifacts.length !== expectedArtifactCount) {
    fail("release manifest does not contain the exact image artifacts")
  }
  const seen = new Set()
  const verifiedArtifactDigests = new Map()
  for (const artifact of manifest.artifacts) {
    validateObjectKeys(artifact, ["name", "path", "sha256"], "release artifact")
    const expected = EXPECTED_ARTIFACTS.get(artifact.name)
    if (!expected || artifact.path !== expected.path || seen.has(artifact.name)) {
      fail("release manifest contains an unexpected or duplicate artifact")
    }
    if (!/^sha256:[a-f0-9]{64}$/.test(artifact.sha256)) fail("release artifact digest is invalid")
    const path = artifactPath(rootfs, artifact.path)
    const actualDigest = expected.type === "tree"
      ? await sha256Tree(path)
      : await readRegularFile(path, `release artifact ${artifact.name}`, Number.MAX_SAFE_INTEGER)
        .then(() => sha256File(path))
    if (actualDigest !== artifact.sha256) fail(`release artifact ${artifact.name} is corrupted`)
    seen.add(artifact.name)
    verifiedArtifactDigests.set(artifact.name, actualDigest)
  }

  if (selectedTopology === "path1") {
    await verifyPath1BuilderAttestation(
      rootfs,
      canonicalRootfs,
      manifest,
      verifiedArtifactDigests,
      trustedBuilderPublicKeyPath,
    )
  } else if (trustedBuilderPublicKeyPath) {
    fail("trusted builder public key is only accepted for Path-1 verification")
  } else if (selectedTopology === "shared_host") {
    // Keep release-signature verification for rollback to signed schema 2 shared-host releases
    // created before the Path-1 builder trust-root requirement existed.
  }

  if (selectedTopology !== undefined) {
    const selectedService = selectedTopology === "path1"
      ? "chariox-path1-managed-bootstrap.service"
      : "chariox-managed-bootstrap.service"
    if (!seen.has(selectedService)) {
      fail(`release does not declare the selected ${selectedTopology} managed bootstrap service`)
    }
    const servicePath = artifactPath(rootfs, EXPECTED_ARTIFACTS.get(selectedService).path)
    const service = (await readRegularFile(servicePath, `${selectedTopology} managed bootstrap service`, 64 * 1024))
      .toString("utf8")
    const lines = service.split(/\r?\n/)
    const execStarts = lines.filter((line) => line.startsWith("ExecStart="))
    if (execStarts.length !== 1 || execStarts[0] !== "ExecStart=/usr/local/bin/chariox-managed-bootstrap") {
      fail(`selected ${selectedTopology} managed bootstrap service has an incompatible ExecStart`)
    }
    if (selectedTopology === "path1") {
      for (const [name, required] of [
        ["CHARIOX_MANAGED_PROVIDER_TOPOLOGY", "Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1"],
        ["CHARIOX_MANAGED_BOOTSTRAP_PATH", "Environment=CHARIOX_MANAGED_BOOTSTRAP_PATH=/var/lib/chariox/managed-bootstrap.json"],
        ["CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY", "Environment=CHARIOX_TRUSTED_BUILDER_PUBLIC_KEY=/etc/chariox/trusted-builder-public-key"],
        ["HOME", "Environment=HOME=/home/chariox"],
        ["CHARIOX_HOME", "Environment=CHARIOX_HOME=/home/chariox/.chariox"],
        ["CHARIOX_SLICE_DOCKER_BROKER_SOCKET", "Environment=CHARIOX_SLICE_DOCKER_BROKER_SOCKET=/var/lib/chariox-slice-share/.broker-private/control/control.sock"],
      ]) {
        const assignments = lines.filter((line) => line.startsWith(`Environment=${name}=`))
        if (assignments.length !== 1 || assignments[0] !== required) {
          fail(`selected Path-1 managed bootstrap service is missing or overrides ${required}`)
        }
      }
      const brokerPrestarts = lines.filter((line) => line.startsWith("ExecStartPre="))
      if (brokerPrestarts.length !== 1
        || brokerPrestarts[0] !== "ExecStartPre=-+/usr/bin/systemctl restart chariox-slice-broker.service") {
        fail("selected Path-1 managed bootstrap service must restart the one-shot broker before launch")
      }
      for (const dependency of ["After=", "Wants="]) {
        const declarations = lines.filter((line) => line.startsWith(dependency))
        if (declarations.length !== 1 || !declarations[0].split(/\s+/).includes("chariox-rootless-docker.service")) {
          fail(`selected Path-1 managed bootstrap service must declare ${dependency}chariox-rootless-docker.service`)
        }
      }
      for (const forbidden of [
        "CHARIOX_MANAGED_PROVIDER_ISOLATION",
        "CHARIOX_CAPABILITY_ISOLATION_ROOT",
        "CHARIOX_MANAGED_PROVIDER_BWRAP",
        "CHARIOX_MANAGED_PROVIDER_HOME",
        "CHARIOX_MANAGED_SLICE_SERVICE_ROOT",
        "CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT",
        "CHARIOX_SLICE_ROOT",
        "bwrap",
        "--disposable-worker",
        "NoNewPrivileges=",
        "PrivateTmp=",
        "PrivateUsers=",
        "PrivateDevices=",
        "PrivateNetwork=",
        "ProtectSystem=",
        "ProtectHome=",
        "ProtectKernel",
        "ProtectControlGroups=",
        "RestrictNamespaces=",
        "RestrictAddressFamilies=",
        "RestrictSUIDSGID=",
        "ReadWritePaths=",
        "ReadOnlyPaths=",
        "InaccessiblePaths=",
        "BindPaths=",
        "BindReadOnlyPaths=",
        "RootDirectory=",
        "RootImage=",
        "SystemCallFilter=",
        "CapabilityBoundingSet=",
        "UMask=",
        "StateDirectory=",
        "SupplementaryGroups=",
      ]) {
        if (service.includes(forbidden)) fail(`selected Path-1 managed bootstrap service contains ${forbidden}`)
      }
    } else if (service.includes("Environment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY=path1")
      || service.includes("--disposable-worker")) {
      fail("selected shared-host managed bootstrap service has a mismatched topology")
    }
  }
}

try {
  if (process.argv.length < 5 || process.argv.length > 7) {
    fail("usage: verify-image-release <rootfs> <expected-release-digest> <trusted-release-public-key> [path1|shared_host [trusted-builder-public-key]]")
  }
  if (process.argv.length === 7 && process.argv[5] !== "path1") {
    fail("trusted builder public key is only accepted for Path-1 verification")
  }
  await verifyImageRelease(
    resolve(process.argv[2]),
    process.argv[3],
    resolve(process.argv[4]),
    process.argv[5],
    process.argv[6] ? resolve(process.argv[6]) : undefined,
  )
} catch (error) {
  const message = error instanceof Error ? error.message : String(error)
  process.stderr.write(`${basename(process.argv[1])}: ${message}\n`)
  process.exitCode = 1
}
