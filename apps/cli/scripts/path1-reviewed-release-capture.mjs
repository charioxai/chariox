#!/usr/bin/env node

import { createHash, createPublicKey, verify } from "node:crypto"
import { constants } from "node:fs"
import { lstat, open, opendir, realpath, unlink } from "node:fs/promises"
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path"
import { pathToFileURL } from "node:url"
import { validateCaptureOutput } from "./path1-provider-rebuild-capture.mjs"

const RELEASE_KEY_PREFIX = Buffer.from("302a300506032b6570032100", "hex")
const MAX_MANIFEST_BYTES = 64 * 1024
const MAX_ATTESTATION_BYTES = 16 * 1024
const MAX_KEY_BYTES = 1024
const MAX_SIGNATURE_BYTES = 1024
const MAX_SERVICE_UNIT_BYTES = 64 * 1024
const MAX_ARTIFACT_BYTES = 512 * 1024 * 1024
const MAX_CONTEXT_ENTRIES = 25000
const MAX_CONTEXT_BYTES = 512 * 1024 * 1024
const MAX_EVIDENCE_BYTES = 128 * 1024
const TARGET = "x86_64-unknown-linux-gnu"
const CONTEXT_ROOT = "usr/lib/chariox/slice-build-context"
const RELAY_CONTEXT_PATH = "apps/kernel/slice-linux-docker/prebuilt/chariox-relay"
const SLICE_KERNEL_CONTEXT_PATH = "apps/kernel/slice-linux-docker/prebuilt/chariox-kernel"
const EXECUTABLE_SUFFIXES = [
  "/slice-linux-docker/prebuilt/chariox-kernel",
  "/slice-linux-docker/prebuilt/chariox-relay",
  "/enter-rootless-docker-namespace.sh",
  "/managed-rootless-service.sh",
  "/provision-linux-docker-slice.sh",
  "/managed-publication-access.sh",
]
const ARTIFACTS = [
  ["chariox-kernel", "/usr/local/bin/chariox-kernel", "file", 0o755],
  ["chariox-managed-bootstrap", "/usr/local/bin/chariox-managed-bootstrap", "file", 0o755],
  ["chariox-managed-bootstrap.service", "/etc/systemd/system/chariox-managed-bootstrap.service", "file", 0o644],
  ["chariox-path1-managed-bootstrap.service", "/etc/systemd/system/chariox-path1-managed-bootstrap.service", "file", 0o644],
  ["chariox-disposable-worker-bootstrap.service", "/etc/systemd/system/chariox-disposable-worker-bootstrap.service", "file", 0o644],
  ["chariox-rootless-docker.service", "/etc/systemd/system/chariox-rootless-docker.service", "file", 0o644],
  ["chariox-slice-broker.service", "/etc/systemd/system/chariox-slice-broker.service", "file", 0o644],
  ["chariox-slice-build-context", "/usr/lib/chariox/slice-build-context", "tree", 0o755],
  ["chariox-build-attestation", "/usr/lib/chariox/build-attestation.json", "file", 0o644],
  ["chariox-build-attestation-signature", "/usr/lib/chariox/build-attestation.sig", "file", 0o644],
  ["chariox-builder-public-key", "/usr/lib/chariox/builder-public-key", "file", 0o644],
]
const EXECUTABLES = {
  "chariox-kernel": {
    path: "/usr/local/bin/chariox-kernel",
    binding: "chariox-kernel",
  },
  "chariox-managed-bootstrap": {
    path: "/usr/local/bin/chariox-managed-bootstrap",
    binding: "chariox-managed-bootstrap",
  },
  "chariox-relay": {
    path: "/usr/lib/chariox/slice-build-context/apps/kernel/slice-linux-docker/prebuilt/chariox-relay",
    binding: "chariox-slice-build-context",
  },
}
const ATTESTED_ARTIFACTS = [
  ["chariox-kernel", "chariox-kernel"],
  ["chariox-managed-bootstrap", "chariox-managed-bootstrap"],
  ["chariox-relay", "chariox-relay"],
]
const USAGE =
  "usage: path1-reviewed-release-capture --release-root <versioned-release-dir> --trusted-release-public-key <file> --trusted-builder-public-key <file> --expected-source-commit <40-hex> --expected-source-tree <40-hex> --executable <chariox-kernel|chariox-managed-bootstrap|chariox-relay> --evidence-output <new-external-file>"

function fail(message) {
  throw new Error(message)
}

function assertObjectKeys(value, expected, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    fail(label + " must be an object")
  }
  const keys = Object.keys(value).sort()
  const wanted = [...expected].sort()
  if (keys.length !== wanted.length || keys.some((key, index) => key !== wanted[index])) {
    fail(label + " has missing or unsupported fields")
  }
}

function assertNoDuplicateJsonKeys(text, label) {
  let index = 0
  let count = 0
  const numberPattern = /-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?/y
  const whitespace = () => {
    while (text[index] === " " || text[index] === "\n" || text[index] === "\r" || text[index] === "\t") index++
  }
  const parseString = () => {
    const start = index
    if (text[index] !== '"') fail(label + " is malformed JSON")
    index++
    while (index < text.length) {
      const character = text[index++]
      if (character === '"') {
        try {
          return JSON.parse(text.slice(start, index))
        } catch {
          fail(label + " is malformed JSON")
        }
      }
      if (character === "\\") index++
    }
    fail(label + " is malformed JSON")
  }
  const parseValue = (depth) => {
    if (depth > 32 || ++count > 20000) fail(label + " exceeds the JSON complexity limit")
    whitespace()
    const character = text[index]
    if (character === "{") {
      index++
      whitespace()
      const keys = new Set()
      if (text[index] === "}") {
        index++
        return
      }
      while (index < text.length) {
        whitespace()
        const key = parseString()
        if (keys.has(key)) fail(label + " contains a duplicate field")
        keys.add(key)
        whitespace()
        if (text[index++] !== ":") fail(label + " is malformed JSON")
        parseValue(depth + 1)
        whitespace()
        if (text[index] === "}") {
          index++
          return
        }
        if (text[index++] !== ",") fail(label + " is malformed JSON")
      }
      fail(label + " is malformed JSON")
    }
    if (character === "[") {
      index++
      whitespace()
      if (text[index] === "]") {
        index++
        return
      }
      while (index < text.length) {
        parseValue(depth + 1)
        whitespace()
        if (text[index] === "]") {
          index++
          return
        }
        if (text[index++] !== ",") fail(label + " is malformed JSON")
      }
      fail(label + " is malformed JSON")
    }
    if (character === '"') {
      parseString()
      return
    }
    for (const literal of ["true", "false", "null"]) {
      if (text.startsWith(literal, index)) {
        index += literal.length
        return
      }
    }
    numberPattern.lastIndex = index
    const number = numberPattern.exec(text)
    if (!number) fail(label + " is malformed JSON")
    index += number[0].length
  }

  parseValue(0)
  whitespace()
  if (index !== text.length) fail(label + " has trailing data")
}

function parseStrictJson(bytes, label) {
  let text
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(bytes)
  } catch {
    fail(label + " is not valid UTF-8")
  }
  assertNoDuplicateJsonKeys(text, label)
  try {
    return JSON.parse(text)
  } catch {
    fail(label + " is malformed JSON")
  }
}

function decodeTextBytes(bytes, length, label) {
  let text
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(bytes).trim()
  } catch {
    fail(label + " is not text")
  }
  if (text.length === length * 2 && /^[0-9a-fA-F]+$/.test(text)) {
    return Buffer.from(text, "hex")
  }
  const standard = Buffer.from(text, "base64")
  if (standard.length === length && standard.toString("base64") === text) return standard
  const url = Buffer.from(text, "base64url")
  if (url.length === length && url.toString("base64url") === text) return url
  fail(label + " is not a supported Ed25519 text encoding")
}

function assertDigest(value, label) {
  if (typeof value !== "string" || !/^sha256:[0-9a-f]{64}$/.test(value)) {
    fail(label + " is not a lowercase SHA-256 digest")
  }
}

function assertGitId(value, label) {
  if (typeof value !== "string" || !/^[0-9a-f]{40}$/.test(value)) {
    fail(label + " must be a full lowercase Git object ID")
  }
}

function isWithin(root, candidate) {
  const relativePath = relative(root, candidate)
  return relativePath === "" || (relativePath !== ".." && !relativePath.startsWith(".." + sep) && !isAbsolute(relativePath))
}

async function validateReleaseRoot(input) {
  if (typeof input !== "string" || input.length === 0) fail("release root is required")
  const requested = resolve(input)
  const metadata = await lstat(requested).catch(() => null)
  if (!metadata || metadata.isSymbolicLink() || !metadata.isDirectory()) {
    fail("release root must be a non-symlink directory")
  }
  const root = await realpath(requested)
  const parent = dirname(root)
  const parentMetadata = await lstat(parent).catch(() => null)
  if (
    basename(parent) !== "releases" ||
    !parentMetadata ||
    parentMetadata.isSymbolicLink() ||
    !parentMetadata.isDirectory() ||
    (metadata.mode & 0o7777) !== 0o755 ||
    !/^[0-9a-f]{64}$/.test(basename(root))
  ) {
    fail("release root must use the installed releases/<manifest-digest> layout")
  }
  return root
}

async function resolveRegularPath(root, relativePath, expectedMode, maxBytes) {
  if (
    typeof relativePath !== "string" ||
    relativePath.startsWith("/") ||
    relativePath.split("/").some((part) => part === "" || part === "." || part === "..")
  ) {
    fail("release artifact path is invalid")
  }
  const parts = relativePath.split("/")
  let current = root
  for (let index = 0; index < parts.length; index++) {
    current = join(current, parts[index])
    const metadata = await lstat(current).catch(() => null)
    if (!metadata || metadata.isSymbolicLink()) fail("release artifact is missing or symlinked")
    if (index + 1 < parts.length) {
      if (!metadata.isDirectory()) fail("release artifact parent is not a directory")
    } else {
      if (!metadata.isFile() || metadata.size > maxBytes) fail("release artifact is not a bounded regular file")
      if (expectedMode !== undefined && (metadata.mode & 0o7777) !== expectedMode) {
        fail("release artifact mode does not match the installed format")
      }
    }
  }
  return current
}

async function readBoundedFile(path, label, maxBytes, expectedMode) {
  const before = await lstat(path).catch(() => null)
  if (!before || before.isSymbolicLink() || !before.isFile() || before.size > maxBytes) {
    fail(label + " must be a bounded regular file")
  }
  if (expectedMode !== undefined && (before.mode & 0o7777) !== expectedMode) {
    fail(label + " mode does not match the installed format")
  }
  let handle
  try {
    handle = await open(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0))
    const metadata = await handle.stat()
    if (
      !metadata.isFile() ||
      metadata.dev !== before.dev ||
      metadata.ino !== before.ino ||
      metadata.size > maxBytes
    ) {
      fail(label + " changed while being opened")
    }
    const bytes = await handle.readFile()
    if (bytes.length > maxBytes || bytes.length !== metadata.size) fail(label + " changed while being read")
    return { bytes, metadata }
  } catch (error) {
    if (error instanceof Error && error.message.startsWith(label)) throw error
    fail(label + " cannot be read")
  } finally {
    await handle?.close().catch(() => {})
  }
}

async function hashBoundedFile(path, label, maxBytes, expectedMode) {
  const before = await lstat(path).catch(() => null)
  if (!before || before.isSymbolicLink() || !before.isFile() || before.size > maxBytes) {
    fail(label + " must be a bounded regular file")
  }
  if (expectedMode !== undefined && (before.mode & 0o7777) !== expectedMode) {
    fail(label + " mode does not match the installed format")
  }
  let handle
  try {
    handle = await open(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0))
    const metadata = await handle.stat()
    if (
      !metadata.isFile() ||
      metadata.dev !== before.dev ||
      metadata.ino !== before.ino ||
      metadata.size !== before.size
    ) {
      fail(label + " changed while being opened")
    }
    const hash = createHash("sha256")
    let size = 0
    const stream = handle.createReadStream({ autoClose: false })
    for await (const chunk of stream) {
      size += chunk.length
      if (size > maxBytes) fail(label + " exceeds its size limit")
      hash.update(chunk)
    }
    if (size !== metadata.size) fail(label + " changed while being read")
    return "sha256:" + hash.digest("hex")
  } catch (error) {
    if (error instanceof Error && error.message.startsWith(label)) throw error
    if (error instanceof Error && error.message.startsWith("slice build context")) throw error
    fail(label + " cannot be read")
  } finally {
    await handle?.close().catch(() => {})
  }
}

async function readReleaseFile(root, absolutePath, label, maxBytes, mode) {
  if (!absolutePath.startsWith("/")) fail("release artifact path is invalid")
  const path = await resolveRegularPath(root, absolutePath.slice(1), mode, maxBytes)
  return readBoundedFile(path, label, maxBytes, mode)
}

function contextExecutable(pathFromRoot) {
  return EXECUTABLE_SUFFIXES.some((suffix) => pathFromRoot.endsWith(suffix))
}

async function hashContextDirectory(root) {
  const treePath = await resolveDirectoryPath(root, CONTEXT_ROOT)
  const rootMetadata = await lstat(treePath)
  if ((rootMetadata.mode & 0o7777) !== 0o755) fail("slice build context root mode is invalid")
  const hash = createHash("sha256")
  hash.update("directory:1:.:493:")
  const counter = { bytes: 0, entries: 0 }
  const selectedDigests = new Map()

  const walk = async (directory, prefix) => {
    const entries = []
    const directoryHandle = await opendir(directory)
    for await (const entry of directoryHandle) {
      if (++counter.entries > MAX_CONTEXT_ENTRIES) fail("slice build context exceeds its entry limit")
      entries.push(entry.name)
    }
    entries.sort((left, right) => (left < right ? -1 : left > right ? 1 : 0))
    for (const name of entries) {
      const path = join(directory, name)
      const pathFromRoot = prefix ? prefix + "/" + name : name
      const metadata = await lstat(path)
      if (metadata.isSymbolicLink()) fail("slice build context contains a symlink")
      const mode = metadata.mode & 0o7777
      const byteLength = Buffer.byteLength(pathFromRoot)
      if (metadata.isDirectory()) {
        if (mode !== 0o755) fail("slice build context directory mode is invalid")
        hash.update("directory:" + byteLength + ":" + pathFromRoot + ":" + mode + ":")
        await walk(path, pathFromRoot)
      } else if (metadata.isFile()) {
        const executable = contextExecutable(pathFromRoot)
        const expectedMode = executable ? 0o755 : 0o644
        if (mode !== expectedMode) fail("slice build context file mode is invalid")
        if (metadata.size > MAX_CONTEXT_BYTES) fail("slice build context file exceeds its size limit")
        hash.update("file:" + byteLength + ":" + pathFromRoot + ":" + mode + ":" + metadata.size + ":")
        const digest = createHash("sha256")
        const before = await lstat(path)
        let handle
        let size = 0
        try {
          handle = await open(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0))
          const opened = await handle.stat()
          if (
            !opened.isFile() ||
            opened.dev !== before.dev ||
            opened.ino !== before.ino ||
            opened.size !== before.size
          ) {
            fail("slice build context artifact changed while being opened")
          }
          const stream = handle.createReadStream({ autoClose: false })
          for await (const chunk of stream) {
            size += chunk.length
            counter.bytes += chunk.length
            if (counter.bytes > MAX_CONTEXT_BYTES) fail("slice build context exceeds its size limit")
            hash.update(chunk)
            if (pathFromRoot === RELAY_CONTEXT_PATH || pathFromRoot === SLICE_KERNEL_CONTEXT_PATH) {
              digest.update(chunk)
            }
          }
          if (size !== opened.size) fail("slice build context artifact changed while being read")
        } catch (error) {
          if (error instanceof Error && error.message.startsWith("slice build context")) throw error
          fail("slice build context artifact cannot be read")
        } finally {
          await handle?.close().catch(() => {})
        }
        if (pathFromRoot === RELAY_CONTEXT_PATH || pathFromRoot === SLICE_KERNEL_CONTEXT_PATH) {
          selectedDigests.set(pathFromRoot, "sha256:" + digest.digest("hex"))
        }
      } else {
        fail("slice build context contains an unsupported file type")
      }
    }
  }

  await walk(treePath, "")
  return { digest: "sha256:" + hash.digest("hex"), selectedDigests }
}

async function resolveDirectoryPath(root, relativePath) {
  const parts = relativePath.split("/")
  let current = root
  for (const part of parts) {
    current = join(current, part)
    const metadata = await lstat(current).catch(() => null)
    if (!metadata || metadata.isSymbolicLink() || !metadata.isDirectory()) {
      fail("release artifact directory is missing or symlinked")
    }
  }
  return current
}

async function readExternalPublicKey(pathInput, label, releaseRoot) {
  if (typeof pathInput !== "string" || pathInput.length === 0) fail(label + " is required")
  const path = resolve(pathInput)
  const result = await readBoundedFile(path, label, MAX_KEY_BYTES)
  const canonicalPath = await realpath(path)
  if (isWithin(releaseRoot, canonicalPath)) fail(label + " must be outside the release")
  return result
}

async function verifyAtRoot(options, root) {
  if (!options || typeof options !== "object" || Array.isArray(options)) fail("capture options are required")
  const allowed = [
    "releaseRoot",
    "trustedReleasePublicKeyPath",
    "trustedBuilderPublicKeyPath",
    "expectedSourceCommit",
    "expectedSourceTree",
    "executable",
    "evidenceOutputPath",
  ]
  if (Object.keys(options).some((key) => !allowed.includes(key))) fail("capture options contain unsupported fields")
  assertGitId(options.expectedSourceCommit, "expected source commit")
  assertGitId(options.expectedSourceTree, "expected source tree")
  if (!Object.hasOwn(EXECUTABLES, options.executable)) {
    fail("unknown executable selection")
  }

  const trustedRelease = await readExternalPublicKey(
    options.trustedReleasePublicKeyPath,
    "trusted release public key",
    root,
  )
  const trustedBuilder = await readExternalPublicKey(
    options.trustedBuilderPublicKeyPath,
    "trusted builder public key",
    root,
  )
  const releaseKey = decodeTextBytes(trustedRelease.bytes, 32, "trusted release public key")
  const builderKey = decodeTextBytes(trustedBuilder.bytes, 32, "trusted builder public key")
  if (releaseKey.equals(builderKey)) fail("release and builder trust keys must be distinct")

  const manifestFile = await readReleaseFile(
    root,
    "/usr/lib/chariox/release-manifest.json",
    "release manifest",
    MAX_MANIFEST_BYTES,
    0o644,
  )
  const manifestBytes = manifestFile.bytes
  const releaseManifestDigest = "sha256:" + createHash("sha256").update(manifestBytes).digest("hex")
  if (basename(root) !== releaseManifestDigest.slice("sha256:".length)) {
    fail("release manifest digest does not match the installed release directory")
  }

  const releaseSignatureFile = await readReleaseFile(
    root,
    "/usr/lib/chariox/release-manifest.sig",
    "release signature",
    MAX_SIGNATURE_BYTES,
    0o644,
  )
  const includedReleaseKeyFile = await readReleaseFile(
    root,
    "/usr/lib/chariox/release-public-key",
    "included release public key",
    MAX_KEY_BYTES,
    0o644,
  )
  const includedReleaseKey = decodeTextBytes(
    includedReleaseKeyFile.bytes,
    32,
    "included release public key",
  )
  if (
    includedReleaseKeyFile.metadata.dev === trustedRelease.metadata.dev &&
    includedReleaseKeyFile.metadata.ino === trustedRelease.metadata.ino
  ) {
    fail("included release key cannot be used as independent trust")
  }
  if (!includedReleaseKey.equals(releaseKey)) fail("included release key does not match independent trust")
  const releaseSignature = decodeTextBytes(
    releaseSignatureFile.bytes,
    64,
    "release signature",
  )
  const releasePublicKey = createPublicKey({
    key: Buffer.concat([RELEASE_KEY_PREFIX, releaseKey]),
    format: "der",
    type: "spki",
  })
  if (!verify(null, manifestBytes, releasePublicKey, releaseSignature)) {
    fail("release manifest signature is invalid")
  }

  const manifest = parseStrictJson(manifestBytes, "release manifest")
  assertObjectKeys(manifest, ["schemaVersion", "sourceCommit", "sourceTree", "artifacts"], "release manifest")
  if (manifest.schemaVersion !== 2 || !Array.isArray(manifest.artifacts) || manifest.artifacts.length !== ARTIFACTS.length) {
    fail("release manifest schema is not the installed format")
  }
  assertGitId(manifest.sourceCommit, "release manifest source commit")
  assertGitId(manifest.sourceTree, "release manifest source tree")
  if (
    manifest.sourceCommit !== options.expectedSourceCommit ||
    manifest.sourceTree !== options.expectedSourceTree
  ) {
    fail("release manifest source identity does not match the expected commit and tree")
  }

  const manifestArtifacts = new Map()
  for (let index = 0; index < ARTIFACTS.length; index++) {
    const [expectedName, expectedPath] = ARTIFACTS[index]
    const artifact = manifest.artifacts[index]
    assertObjectKeys(artifact, ["name", "path", "sha256"], "release artifact")
    if (artifact.name !== expectedName || artifact.path !== expectedPath) {
      fail("release manifest artifact order, name, or path is ambiguous")
    }
    assertDigest(artifact.sha256, "release artifact digest")
    if (manifestArtifacts.has(artifact.name) || [...manifestArtifacts.values()].some((item) => item.path === artifact.path)) {
      fail("release manifest contains an ambiguous artifact")
    }
    manifestArtifacts.set(artifact.name, artifact)
  }

  const actualDigests = new Map()
  for (const [name, path, kind, mode] of ARTIFACTS) {
    if (kind === "tree") continue
    const maxBytes = name.endsWith(".service")
      ? MAX_SERVICE_UNIT_BYTES
      : name === "chariox-build-attestation"
        ? MAX_ATTESTATION_BYTES
        : name === "chariox-build-attestation-signature" || name === "chariox-builder-public-key"
          ? MAX_SIGNATURE_BYTES
          : MAX_ARTIFACT_BYTES
    const file = await resolveRegularPath(root, path.slice(1), mode, maxBytes)
    const digest = await hashBoundedFile(file, name, maxBytes, mode)
    const expected = manifestArtifacts.get(name).sha256
    if (digest !== expected) fail(name + " artifact digest does not match the signed release")
    actualDigests.set(name, digest)
  }
  const context = await hashContextDirectory(root)
  if (context.digest !== manifestArtifacts.get("chariox-slice-build-context").sha256) {
    fail("slice build context digest does not match the signed release")
  }
  const directKernelDigest = actualDigests.get("chariox-kernel")
  if (context.selectedDigests.get(SLICE_KERNEL_CONTEXT_PATH) !== directKernelDigest) {
    fail("slice kernel artifact does not match the signed release kernel")
  }
  actualDigests.set("chariox-slice-build-context", context.digest)

  const attestationFile = await readReleaseFile(
    root,
    "/usr/lib/chariox/build-attestation.json",
    "builder attestation",
    MAX_ATTESTATION_BYTES,
    0o644,
  )
  const attestationSignatureFile = await readReleaseFile(
    root,
    "/usr/lib/chariox/build-attestation.sig",
    "builder attestation signature",
    MAX_SIGNATURE_BYTES,
    0o644,
  )
  const includedBuilderKeyFile = await readReleaseFile(
    root,
    "/usr/lib/chariox/builder-public-key",
    "included builder public key",
    MAX_KEY_BYTES,
    0o644,
  )
  const includedBuilderKey = decodeTextBytes(
    includedBuilderKeyFile.bytes,
    32,
    "included builder public key",
  )
  if (
    includedBuilderKeyFile.metadata.dev === trustedBuilder.metadata.dev &&
    includedBuilderKeyFile.metadata.ino === trustedBuilder.metadata.ino
  ) {
    fail("included builder key cannot be used as independent trust")
  }
  if (!includedBuilderKey.equals(builderKey)) fail("included builder key does not match independent trust")
  const attestationSignature = decodeTextBytes(
    attestationSignatureFile.bytes,
    64,
    "builder attestation signature",
  )
  const builderPublicKey = createPublicKey({
    key: Buffer.concat([RELEASE_KEY_PREFIX, builderKey]),
    format: "der",
    type: "spki",
  })
  if (!verify(null, attestationFile.bytes, builderPublicKey, attestationSignature)) {
    fail("builder attestation signature is invalid")
  }

  const attestation = parseStrictJson(attestationFile.bytes, "builder attestation")
  assertObjectKeys(
    attestation,
    ["schemaVersion", "sourceCommit", "sourceTree", "target", "artifacts"],
    "builder attestation",
  )
  if (
    attestation.schemaVersion !== 1 ||
    attestation.sourceCommit !== options.expectedSourceCommit ||
    attestation.sourceTree !== options.expectedSourceTree ||
    attestation.sourceCommit !== manifest.sourceCommit ||
    attestation.sourceTree !== manifest.sourceTree ||
    attestation.target !== TARGET ||
    !Array.isArray(attestation.artifacts) ||
    attestation.artifacts.length !== ATTESTED_ARTIFACTS.length
  ) {
    fail("builder attestation source identity or target does not match")
  }

  const relayDigest = context.selectedDigests.get(RELAY_CONTEXT_PATH)
  if (!relayDigest) fail("builder attestation relay artifact is missing")
  const expectedAttestedDigests = new Map([
    ["chariox-kernel", actualDigests.get("chariox-kernel")],
    ["chariox-managed-bootstrap", actualDigests.get("chariox-managed-bootstrap")],
    ["chariox-relay", relayDigest],
  ])
  for (let index = 0; index < ATTESTED_ARTIFACTS.length; index++) {
    const [expectedName] = ATTESTED_ARTIFACTS[index]
    const artifact = attestation.artifacts[index]
    assertObjectKeys(artifact, ["name", "sha256"], "builder attestation artifact")
    assertDigest(artifact.sha256, "builder attestation artifact digest")
    if (artifact.name !== expectedName || artifact.sha256 !== expectedAttestedDigests.get(expectedName)) {
      fail("builder attestation artifacts do not match the installed release")
    }
  }

  const selected = EXECUTABLES[options.executable]
  let selectedDigest
  if (options.executable === "chariox-relay") selectedDigest = relayDigest
  else selectedDigest = actualDigests.get(selected.binding)
  const selectedPath = await resolveRegularPath(root, selected.path.slice(1), 0o755, MAX_ARTIFACT_BYTES)
  const selectedActualDigest = await hashBoundedFile(
    selectedPath,
    "selected executable",
    MAX_ARTIFACT_BYTES,
    0o755,
  )
  if (selectedActualDigest !== selectedDigest) fail("selected executable is not bound to the signed release")

  const artifactBindings = []
  for (const [name, path] of ARTIFACTS) {
    const digest = name === "chariox-slice-build-context"
      ? context.digest
      : actualDigests.get(name)
    artifactBindings.push({ name, path, sha256: manifestArtifacts.get(name).sha256, actualSha256: digest })
  }

  return {
    evidenceSchema: "chariox-path1-reviewed-release-evidence-v1",
    releaseManifestDigest,
    releaseManifestBase64: manifestBytes.toString("base64"),
    releaseManifestSignatureBase64: releaseSignature.toString("base64"),
    releasePublicKeyBase64: releaseKey.toString("base64"),
    builderAttestationBase64: attestationFile.bytes.toString("base64"),
    builderAttestationSignatureBase64: attestationSignature.toString("base64"),
    builderPublicKeyBase64: builderKey.toString("base64"),
    sourceCommit: manifest.sourceCommit,
    sourceTree: manifest.sourceTree,
    target: attestation.target,
    selectedExecutable: {
      name: options.executable,
      path: selected.path,
      sha256: selectedDigest,
    },
    artifactBindings,
  }
}

async function writeExternalEvidence(pathInput, evidence, releaseRoot) {
  if (typeof pathInput !== "string" || pathInput.length === 0) fail("evidence output path is required")
  let path
  try {
    path = await validateCaptureOutput(pathInput)
  } catch (error) {
    fail("evidence output must be a new writable regular file: " + error.message)
  }
  const parent = await realpath(dirname(path)).catch(() => null)
  if (!parent || isWithin(releaseRoot, parent)) {
    fail("evidence output must be a new file outside the release")
  }
  const bytes = Buffer.from(JSON.stringify(evidence, null, 2) + "\n")
  if (bytes.length > MAX_EVIDENCE_BYTES) fail("verification evidence exceeds its size limit")
  let handle
  try {
    handle = await open(
      path,
      constants.O_WRONLY |
        constants.O_CREAT |
        constants.O_EXCL |
        (constants.O_NOFOLLOW ?? 0),
      0o600,
    )
    await handle.writeFile(bytes)
    await handle.chmod(0o600)
    const metadata = await handle.stat()
    if (!metadata.isFile() || (metadata.mode & 0o777) !== 0o600) {
      fail("evidence output mode is not 0600")
    }
  } catch (error) {
    if (handle) {
      await handle.close().catch(() => {})
      await unlink(path).catch(() => {})
    }
    if (error instanceof Error && error.message.startsWith("evidence")) throw error
    fail("evidence output must be a new writable regular file")
  } finally {
    await handle?.close().catch(() => {})
  }
}

export async function verifyPath1ReviewedRelease(options) {
  if (!options || typeof options !== "object" || Array.isArray(options)) fail("capture options are required")
  const root = await validateReleaseRoot(options.releaseRoot)
  return verifyAtRoot(options, root)
}

export async function capturePath1ReviewedRelease(options) {
  if (!options || typeof options !== "object" || Array.isArray(options)) fail("capture options are required")
  const root = await validateReleaseRoot(options?.releaseRoot)
  const evidence = await verifyAtRoot(options, root)
  await writeExternalEvidence(options.evidenceOutputPath, evidence, root)
  return evidence
}

function parseOptions(argv) {
  const names = [
    "release-root",
    "trusted-release-public-key",
    "trusted-builder-public-key",
    "expected-source-commit",
    "expected-source-tree",
    "executable",
    "evidence-output",
  ]
  const values = new Map()
  for (let index = 0; index < argv.length; index += 2) {
    const option = argv[index]
    const value = argv[index + 1]
    if (!option?.startsWith("--") || !value || value.startsWith("--")) fail(USAGE)
    const name = option.slice(2)
    if (!names.includes(name) || values.has(name)) fail(USAGE)
    values.set(name, value)
  }
  if (values.size !== names.length) fail(USAGE)
  return {
    releaseRoot: values.get("release-root"),
    trustedReleasePublicKeyPath: values.get("trusted-release-public-key"),
    trustedBuilderPublicKeyPath: values.get("trusted-builder-public-key"),
    expectedSourceCommit: values.get("expected-source-commit"),
    expectedSourceTree: values.get("expected-source-tree"),
    executable: values.get("executable"),
    evidenceOutputPath: values.get("evidence-output"),
  }
}

function isMainModule() {
  return process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href
}

if (isMainModule()) {
  try {
    const options = parseOptions(process.argv.slice(2))
    const evidence = await capturePath1ReviewedRelease(options)
    process.stdout.write("release manifest digest: " + evidence.releaseManifestDigest + "\n")
    process.stdout.write("raw verification evidence written: " + resolve(options.evidenceOutputPath) + "\n")
  } catch (error) {
    const message = error instanceof Error ? error.message : "capture failed"
    process.stderr.write("path1-reviewed-release-capture: " + message + "\n")
    process.exitCode = 1
  }
}
