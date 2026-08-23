#!/usr/bin/env node

import { createHash, createPrivateKey, createPublicKey, sign, verify } from "node:crypto"
import { spawnSync } from "node:child_process"
import { constants, createReadStream } from "node:fs"
import { chmod, copyFile, lstat, mkdir, readFile, readdir, utimes, writeFile } from "node:fs/promises"
import { basename, dirname, join, relative, resolve, sep } from "node:path"

const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex")
const REQUIRED_OPTIONS = [
  "kernel",
  "supervisor",
  "builder-attestation",
  "builder-attestation-signature",
  "trusted-builder-public-key",
  "signing-key",
  "source-repository",
  "source-commit",
  "output",
]
const BUILD_TARGET = "x86_64-unknown-linux-gnu"
const SLICE_BUILD_CONTEXT_PATH = "/usr/lib/chariox/slice-build-context"
const SLICE_BUILD_CONTEXT_SOURCES = [
  "Cargo.toml",
  "Cargo.lock",
  "adapters/rust",
  "apps/aegs-dummy",
  "apps/kernel",
  "apps/relay",
  "examples/workflow-code",
  "packages/aegs-sdk",
  "packages/event-protocol",
]

function usage() {
  return "usage: package-managed-kernel-release --kernel <path> --supervisor <path> --builder-attestation <path> --builder-attestation-signature <path> --trusted-builder-public-key <path> --signing-key <path> --source-repository <git-worktree> --source-commit <40-hex-commit> --output <directory>"
}

function parseOptions(argv) {
  const options = new Map()
  for (let index = 0; index < argv.length; index += 2) {
    const option = argv[index]
    const value = argv[index + 1]
    if (!option?.startsWith("--") || !value || value.startsWith("--")) {
      throw new Error(usage())
    }
    const name = option.slice(2)
    if (!REQUIRED_OPTIONS.includes(name) || options.has(name)) {
      throw new Error(usage())
    }
    options.set(name, name === "source-commit" ? value : resolve(value))
  }
  if (options.size !== REQUIRED_OPTIONS.length) {
    throw new Error(usage())
  }
  return Object.fromEntries(options)
}

async function requireRegularFile(path, label, maxBytes = Number.MAX_SAFE_INTEGER) {
  const metadata = await lstat(path).catch((error) => {
    throw new Error(`${label} cannot be read: ${error.message}`)
  })
  if (metadata.isSymbolicLink() || !metadata.isFile() || metadata.size > maxBytes) {
    throw new Error(`${label} must be a bounded regular file`)
  }
  return metadata
}

async function prepareOutput(path) {
  const metadata = await lstat(path).catch((error) => {
    if (error.code === "ENOENT") return null
    throw error
  })
  if (metadata) {
    if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
      throw new Error("output must be a directory, not a symlink")
    }
    if ((await readdir(path)).length !== 0) {
      throw new Error("output directory must be empty")
    }
  } else {
    await mkdir(path, { recursive: true, mode: 0o755 })
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
    if (!entry.isFile()) throw new Error("slice build context contains an unsupported file type")
    hash.update(`file:${Buffer.byteLength(pathFromRoot)}:${pathFromRoot}:${mode}:${metadata.size}:`)
    for await (const chunk of createReadStream(path)) hash.update(chunk)
  }
}

async function sha256Tree(root) {
  const metadata = await lstat(root)
  if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
    throw new Error("slice build context must be a directory, not a symlink")
  }
  const hash = createHash("sha256")
  hash.update(`directory:1:.:${metadata.mode & 0o7777}:`)
  await updateTreeHash(root, root, hash)
  return `sha256:${hash.digest("hex")}`
}

function gitText(repositoryRoot, args, label) {
  const result = spawnSync("git", args, {
    cwd: repositoryRoot,
    encoding: "utf8",
    maxBuffer: 1024 * 1024,
    env: gitEnvironment(),
  })
  if (result.status !== 0) throw new Error(`${label}: ${result.stderr.trim()}`)
  return result.stdout.trim()
}

function gitEnvironment() {
  return {
    PATH: process.env.PATH ?? "/usr/bin:/bin",
    HOME: "/nonexistent",
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_CONFIG_GLOBAL: "/dev/null",
    GIT_NO_REPLACE_OBJECTS: "1",
    LC_ALL: "C",
  }
}

function resolveSourceIdentity(repositoryRoot, sourceCommit) {
  if (!/^[a-f0-9]{40}$/.test(sourceCommit)) {
    throw new Error("source commit must be a full lowercase Git commit ID")
  }
  const commit = gitText(repositoryRoot, ["rev-parse", "--verify", `${sourceCommit}^{commit}`], "source commit cannot be resolved")
  if (commit !== sourceCommit) throw new Error("source commit did not resolve exactly")
  const tree = gitText(repositoryRoot, ["rev-parse", `${sourceCommit}^{tree}`], "source tree cannot be resolved")
  if (!/^[a-f0-9]{40}$/.test(tree)) throw new Error("source tree is invalid")
  return { commit, tree }
}

async function installSliceBuildContext(repositoryRoot, destination, sourceCommit) {
  await mkdir(destination, { recursive: true, mode: 0o755 })
  const listing = spawnSync(
    "git",
    ["ls-tree", "-r", "-z", "--full-tree", sourceCommit, "--", ...SLICE_BUILD_CONTEXT_SOURCES],
    { cwd: repositoryRoot, maxBuffer: 128 * 1024 * 1024, env: gitEnvironment() },
  )
  if (listing.status !== 0) {
    throw new Error(`slice build context cannot be listed: ${listing.stderr.toString().trim()}`)
  }
  for (const record of listing.stdout.toString("utf8").split("\0").filter(Boolean)) {
    const match = /^(100644|100755) blob ([a-f0-9]{40})\t([^\0]+)$/.exec(record)
    if (!match || match[3].startsWith("/") || match[3].split("/").includes("..")) {
      throw new Error("slice build context contains an unsupported Git object")
    }
    const blob = spawnSync("git", ["cat-file", "blob", match[2]], {
      cwd: repositoryRoot,
      maxBuffer: 128 * 1024 * 1024,
      env: gitEnvironment(),
    })
    if (blob.status !== 0) throw new Error("slice build context blob cannot be read")
    await installBytes(blob.stdout, join(destination, match[3]), match[1] === "100755" ? 0o755 : 0o644)
  }
}

function gitBlob(repositoryRoot, sourceCommit, path, label) {
  const result = spawnSync("git", ["show", `${sourceCommit}:${path}`], {
    cwd: repositoryRoot,
    maxBuffer: 1024 * 1024,
    env: gitEnvironment(),
  })
  if (result.status !== 0) throw new Error(`${label}: ${result.stderr.toString().trim()}`)
  return result.stdout
}

async function installBytes(bytes, destination, mode) {
  await mkdir(dirname(destination), { recursive: true, mode: 0o755 })
  await writeFile(destination, bytes, { flag: "wx", mode })
  await chmod(destination, mode)
}

async function installFile(source, destination, mode) {
  await mkdir(dirname(destination), { recursive: true, mode: 0o755 })
  await copyFile(source, destination, constants.COPYFILE_EXCL)
  await chmod(destination, mode)
}

function sourceDateEpoch() {
  const value = process.env.SOURCE_DATE_EPOCH ?? "0"
  if (!/^(?:0|[1-9][0-9]*)$/.test(value) || !Number.isSafeInteger(Number(value))) {
    throw new Error("SOURCE_DATE_EPOCH must be a non-negative integer")
  }
  return new Date(Number(value) * 1000)
}

async function normalizeTree(root, timestamp) {
  const entries = await readdir(root, { withFileTypes: true })
  for (const entry of entries) {
    const path = join(root, entry.name)
    if (entry.isDirectory()) {
      await normalizeTree(path, timestamp)
      await chmod(path, 0o755)
    } else if (entry.isFile()) {
      const executable =
        path.endsWith("/usr/local/bin/chariox-kernel") ||
        path.endsWith("/usr/local/bin/chariox-managed-bootstrap") ||
        path.endsWith("/enter-rootless-docker-namespace.sh") ||
        path.endsWith("/provision-linux-docker-slice.sh") ||
        path.endsWith("/managed-publication-access.sh")
      await chmod(path, executable ? 0o755 : 0o644)
    } else {
      throw new Error("packaged release contains an unsupported file type")
    }
    await utimes(path, timestamp, timestamp)
  }
  await chmod(root, 0o755)
  await utimes(root, timestamp, timestamp)
}

function rawEd25519PublicKey(privateKey) {
  const publicDer = createPublicKey(privateKey).export({ format: "der", type: "spki" })
  if (
    publicDer.length !== ED25519_SPKI_PREFIX.length + 32 ||
    !publicDer.subarray(0, ED25519_SPKI_PREFIX.length).equals(ED25519_SPKI_PREFIX)
  ) {
    throw new Error("signing key did not produce the expected Ed25519 public key")
  }
  return publicDer.subarray(ED25519_SPKI_PREFIX.length)
}

function validateObjectKeys(value, expected, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} is invalid`)
  }
  const actual = Object.keys(value).sort()
  const sortedExpected = [...expected].sort()
  if (actual.length !== sortedExpected.length || actual.some((key, index) => key !== sortedExpected[index])) {
    throw new Error(`${label} contains unsupported fields`)
  }
}

function decodeCanonicalBase64(bytes, expectedLength, label) {
  const text = bytes.toString("utf8").trim()
  const decoded = Buffer.from(text, "base64")
  if (decoded.length !== expectedLength || decoded.toString("base64") !== text) {
    throw new Error(`${label} is not canonical base64`)
  }
  return decoded
}

async function verifyBuilderAttestation(options, sourceIdentity, artifactDigests) {
  const attestationBytes = await readFile(options["builder-attestation"])
  if (attestationBytes.length > 64 * 1024) throw new Error("builder attestation is too large")
  let attestation
  try {
    attestation = JSON.parse(attestationBytes)
  } catch {
    throw new Error("builder attestation is invalid JSON")
  }
  if (!Buffer.from(JSON.stringify(attestation)).equals(attestationBytes)) {
    throw new Error("builder attestation is not canonical JSON")
  }
  validateObjectKeys(
    attestation,
    ["schemaVersion", "sourceCommit", "sourceTree", "target", "artifacts"],
    "builder attestation",
  )
  if (
    attestation.schemaVersion !== 1 ||
    attestation.sourceCommit !== sourceIdentity.commit ||
    attestation.sourceTree !== sourceIdentity.tree ||
    attestation.target !== BUILD_TARGET ||
    !Array.isArray(attestation.artifacts) ||
    attestation.artifacts.length !== 2
  ) {
    throw new Error("builder attestation identity or target does not match the release")
  }
  const expectedArtifacts = [
    ["chariox-kernel", artifactDigests.kernel],
    ["chariox-managed-bootstrap", artifactDigests.supervisor],
  ]
  for (const [index, artifact] of attestation.artifacts.entries()) {
    validateObjectKeys(artifact, ["name", "sha256"], "builder attestation artifact")
    const [expectedName, expectedDigest] = expectedArtifacts[index]
    if (artifact.name !== expectedName || artifact.sha256 !== expectedDigest) {
      throw new Error("builder attestation artifacts do not match the staged binaries")
    }
  }
  const trustedBuilderRawKey = decodeCanonicalBase64(
    await readFile(options["trusted-builder-public-key"]),
    32,
    "trusted builder public key",
  )
  const signature = decodeCanonicalBase64(
    await readFile(options["builder-attestation-signature"]),
    64,
    "builder attestation signature",
  )
  const builderPublicKey = createPublicKey({
    key: Buffer.concat([ED25519_SPKI_PREFIX, trustedBuilderRawKey]),
    format: "der",
    type: "spki",
  })
  if (!verify(null, attestationBytes, builderPublicKey, signature)) {
    throw new Error("builder attestation signature is invalid")
  }
  return { attestationBytes, signature, trustedBuilderRawKey }
}

async function packageRelease(options) {
  await requireRegularFile(options.kernel, "kernel binary")
  await requireRegularFile(options.supervisor, "bootstrap supervisor")
  await requireRegularFile(options["builder-attestation"], "builder attestation", 64 * 1024)
  await requireRegularFile(options["builder-attestation-signature"], "builder attestation signature", 1024)
  await requireRegularFile(options["trusted-builder-public-key"], "trusted builder public key", 1024)
  const repositoryRoot = options["source-repository"]
  const repositoryMetadata = await lstat(repositoryRoot).catch((error) => {
    throw new Error(`source repository cannot be read: ${error.message}`)
  })
  if (repositoryMetadata.isSymbolicLink() || !repositoryMetadata.isDirectory()) {
    throw new Error("source repository must be a directory, not a symlink")
  }
  const sourceIdentity = resolveSourceIdentity(repositoryRoot, options["source-commit"])
  await prepareOutput(options.output)
  const timestamp = sourceDateEpoch()

  const rootfs = join(options.output, "rootfs")
  const kernelDestination = join(rootfs, "usr/local/bin/chariox-kernel")
  const supervisorDestination = join(rootfs, "usr/local/bin/chariox-managed-bootstrap")
  const releaseDirectory = join(rootfs, "usr/lib/chariox")
  const sliceBuildContext = join(rootfs, SLICE_BUILD_CONTEXT_PATH.slice(1))
  const serviceDestination = join(
    rootfs,
    "etc/systemd/system/chariox-managed-bootstrap.service",
  )
  const rootlessDockerServiceDestination = join(
    rootfs,
    "etc/systemd/system/chariox-rootless-docker.service",
  )
  const sliceBrokerServiceDestination = join(
    rootfs,
    "etc/systemd/system/chariox-slice-broker.service",
  )
  const serviceBytes = gitBlob(
    repositoryRoot,
    sourceIdentity.commit,
    "deploy/managed-kernel/chariox-managed-bootstrap.service",
    "managed bootstrap service cannot be read from source commit",
  )
  const rootlessDockerServiceBytes = gitBlob(
    repositoryRoot,
    sourceIdentity.commit,
    "deploy/managed-kernel/chariox-rootless-docker.service",
    "rootless Docker service cannot be read from source commit",
  )
  const sliceBrokerServiceBytes = gitBlob(
    repositoryRoot,
    sourceIdentity.commit,
    "deploy/managed-kernel/chariox-slice-broker.service",
    "slice Docker broker service cannot be read from source commit",
  )
  if (
    serviceBytes.length > 64 * 1024 ||
    rootlessDockerServiceBytes.length > 64 * 1024 ||
    sliceBrokerServiceBytes.length > 64 * 1024
  ) {
    throw new Error("managed service unit is too large")
  }

  await installFile(options.kernel, kernelDestination, 0o755)
  await installFile(options.supervisor, supervisorDestination, 0o755)
  await installBytes(serviceBytes, serviceDestination, 0o644)
  await installBytes(rootlessDockerServiceBytes, rootlessDockerServiceDestination, 0o644)
  await installBytes(sliceBrokerServiceBytes, sliceBrokerServiceDestination, 0o644)
  await installSliceBuildContext(repositoryRoot, sliceBuildContext, sourceIdentity.commit)
  await normalizeTree(sliceBuildContext, timestamp)
  const sourceKernelDigest = await sha256File(options.kernel)
  const packagedKernelDigest = await sha256File(kernelDestination)
  if (sourceKernelDigest !== packagedKernelDigest) {
    throw new Error("kernel binary changed while the release was packaged")
  }
  const sourceSupervisorDigest = await sha256File(options.supervisor)
  const packagedSupervisorDigest = await sha256File(supervisorDestination)
  if (sourceSupervisorDigest !== packagedSupervisorDigest) {
    throw new Error("bootstrap supervisor changed while the release was packaged")
  }
  const packagedServiceDigest = await sha256File(serviceDestination)
  if (`sha256:${createHash("sha256").update(serviceBytes).digest("hex")}` !== packagedServiceDigest) {
    throw new Error("managed bootstrap service changed while the release was packaged")
  }
  const packagedRootlessDockerServiceDigest = await sha256File(rootlessDockerServiceDestination)
  if (`sha256:${createHash("sha256").update(rootlessDockerServiceBytes).digest("hex")}` !== packagedRootlessDockerServiceDigest) {
    throw new Error("rootless Docker service changed while the release was packaged")
  }
  const packagedSliceBrokerServiceDigest = await sha256File(sliceBrokerServiceDestination)
  if (`sha256:${createHash("sha256").update(sliceBrokerServiceBytes).digest("hex")}` !== packagedSliceBrokerServiceDigest) {
    throw new Error("slice Docker broker service changed while the release was packaged")
  }
  const packagedSliceBuildContextDigest = await sha256Tree(sliceBuildContext)
  const builderAttestation = await verifyBuilderAttestation(options, sourceIdentity, {
    kernel: packagedKernelDigest,
    supervisor: packagedSupervisorDigest,
  })
  const signingKeyMetadata = await requireRegularFile(
    options["signing-key"],
    "release signing key",
    64 * 1024,
  )
  if (process.platform !== "win32" && (signingKeyMetadata.mode & 0o077) !== 0) {
    throw new Error("release signing key must not be readable by group or other users")
  }

  await installBytes(
    builderAttestation.attestationBytes,
    join(releaseDirectory, "build-attestation.json"),
    0o644,
  )
  await installBytes(
    Buffer.from(builderAttestation.signature.toString("base64")),
    join(releaseDirectory, "build-attestation.sig"),
    0o644,
  )
  await installBytes(
    Buffer.from(builderAttestation.trustedBuilderRawKey.toString("base64")),
    join(releaseDirectory, "builder-public-key"),
    0o644,
  )
  const packagedBuildAttestationDigest = await sha256File(join(releaseDirectory, "build-attestation.json"))
  const packagedBuildAttestationSignatureDigest = await sha256File(join(releaseDirectory, "build-attestation.sig"))
  const packagedBuilderPublicKeyDigest = await sha256File(join(releaseDirectory, "builder-public-key"))

  const manifestBytes = Buffer.from(
    JSON.stringify({
      schemaVersion: 2,
      sourceCommit: sourceIdentity.commit,
      sourceTree: sourceIdentity.tree,
      artifacts: [
        {
          name: "chariox-kernel",
          path: "/usr/local/bin/chariox-kernel",
          sha256: packagedKernelDigest,
        },
        {
          name: "chariox-managed-bootstrap",
          path: "/usr/local/bin/chariox-managed-bootstrap",
          sha256: packagedSupervisorDigest,
        },
        {
          name: "chariox-managed-bootstrap.service",
          path: "/etc/systemd/system/chariox-managed-bootstrap.service",
          sha256: packagedServiceDigest,
        },
        {
          name: "chariox-rootless-docker.service",
          path: "/etc/systemd/system/chariox-rootless-docker.service",
          sha256: packagedRootlessDockerServiceDigest,
        },
        {
          name: "chariox-slice-broker.service",
          path: "/etc/systemd/system/chariox-slice-broker.service",
          sha256: packagedSliceBrokerServiceDigest,
        },
        {
          name: "chariox-slice-build-context",
          path: SLICE_BUILD_CONTEXT_PATH,
          sha256: packagedSliceBuildContextDigest,
        },
        {
          name: "chariox-build-attestation",
          path: "/usr/lib/chariox/build-attestation.json",
          sha256: packagedBuildAttestationDigest,
        },
        {
          name: "chariox-build-attestation-signature",
          path: "/usr/lib/chariox/build-attestation.sig",
          sha256: packagedBuildAttestationSignatureDigest,
        },
        {
          name: "chariox-builder-public-key",
          path: "/usr/lib/chariox/builder-public-key",
          sha256: packagedBuilderPublicKeyDigest,
        },
      ],
    }),
  )
  const privateKey = createPrivateKey({
    key: await readFile(options["signing-key"]),
    format: "pem",
    type: "pkcs8",
  })
  if (privateKey.asymmetricKeyType !== "ed25519") {
    throw new Error("release signing key must be an Ed25519 PKCS8 private key")
  }
  const signature = sign(null, manifestBytes, privateKey)
  if (signature.length !== 64) {
    throw new Error("release signature has the wrong length")
  }

  await mkdir(releaseDirectory, { recursive: true, mode: 0o755 })
  await writeFile(join(releaseDirectory, "release-manifest.json"), manifestBytes, {
    flag: "wx",
    mode: 0o644,
  })
  await writeFile(join(releaseDirectory, "release-manifest.sig"), signature.toString("base64"), {
    flag: "wx",
    mode: 0o644,
  })
  await writeFile(
    join(releaseDirectory, "release-public-key"),
    rawEd25519PublicKey(privateKey).toString("base64"),
    { flag: "wx", mode: 0o644 },
  )
  await normalizeTree(rootfs, timestamp)
  const releaseDigest = `sha256:${createHash("sha256").update(manifestBytes).digest("hex")}`
  process.stdout.write(`${releaseDigest}\n`)
}

try {
  await packageRelease(parseOptions(process.argv.slice(2)))
} catch (error) {
  const message = error instanceof Error ? error.message : String(error)
  process.stderr.write(`${basename(process.argv[1])}: ${message}\n`)
  process.exitCode = 1
}
