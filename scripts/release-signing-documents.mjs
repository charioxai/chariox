// The small canonical documents that carry release trust, and the signing of them.
// Pure on purpose (node:crypto only, no filesystem): the signing Mac needs this file and
// sign-digests-on-mac.mjs, never the multi-gigabyte release artifacts.
import { createHash, createPublicKey, sign, verify } from "node:crypto"

const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex")
const BUILD_TARGET = "x86_64-unknown-linux-gnu"
const SIGNATURE_DEPENDENT = ["chariox-build-attestation-signature", "chariox-builder-public-key"]
const ATTESTATION_NAMES = {
  boot: ["chariox-kernel", "chariox-managed-bootstrap", "chariox-relay"],
  managed: ["chariox-kernel", "chariox-managed-bootstrap", "chariox-relay", "chariox-app-package", "chariox-app-wasm-worker"],
}
// The boot facade pins the release authority as its builder; the managed rootfs has its own builder authority.
// The two on-disk formats differ only in a trailing newline (documents and signatures) and in the manifest header.
const KINDS = {
  boot: { attestationSigner: "release", eol: "\n", header: (identity) => ({ schemaVersion: 2, ...identity }) },
  managed: {
    attestationSigner: "builder",
    eol: "",
    header: (identity) => ({ schemaVersion: 3, managedUpdateEvidenceVersion: 1, ...identity }),
  },
}
export const SIGNED_FILES = [
  "attestation.sig",
  "builder-public-key",
  "release-manifest.json",
  "release-manifest.sig",
  "release-public-key",
]

export const sha = (bytes) => `sha256:${createHash("sha256").update(bytes).digest("hex")}`
const fail = (message) => {
  throw new Error(message)
}
const isDigest = (value) => typeof value === "string" && /^sha256:[0-9a-f]{64}$/.test(value)

export function rawPublicKey(privateKey) {
  const der = createPublicKey(privateKey).export({ format: "der", type: "spki" })
  if (der.length !== ED25519_SPKI_PREFIX.length + 32 || !der.subarray(0, ED25519_SPKI_PREFIX.length).equals(ED25519_SPKI_PREFIX)) {
    fail("signing key is not an Ed25519 key")
  }
  return der.subarray(ED25519_SPKI_PREFIX.length)
}
export const keyFingerprint = (raw) => sha(raw)

function publicKeyObject(base64, label) {
  const text = base64.toString("utf8").trim()
  const raw = Buffer.from(text, "base64")
  if (raw.length !== 32 || raw.toString("base64") !== text) fail(`${label} is not a canonical base64 Ed25519 public key`)
  return { raw, key: createPublicKey({ key: Buffer.concat([ED25519_SPKI_PREFIX, raw]), format: "der", type: "spki" }) }
}
function signatureBytes(base64, label) {
  const text = base64.toString("utf8").trim()
  const raw = Buffer.from(text, "base64")
  if (raw.length !== 64 || raw.toString("base64") !== text) fail(`${label} is not a canonical base64 Ed25519 signature`)
  return raw
}

export function manifestBytes(kind, identity, entries) {
  const { eol, header } = KINDS[kind]
  return Buffer.from(JSON.stringify({ ...header({ sourceCommit: identity.sourceCommit, sourceTree: identity.sourceTree }), artifacts: entries }) + eol)
}

// A signing request is everything the signer needs to sign, and nothing else: the builder
// attestation (exact bytes) and the manifest entries with the digests of the staged artifacts.
// The two entries that hold the attestation signature and builder key digests stay null until signed.
export function signingRequestBytes(kind, { sourceCommit, sourceTree, attestationBytes, entries, sliceBuildContext, sliceImage }) {
  if (!KINDS[kind]) fail(`unknown release kind ${kind}`)
  const request = {
    schemaVersion: 1,
    kind,
    sourceCommit,
    sourceTree,
    attestation: { sha256: sha(attestationBytes), base64: attestationBytes.toString("base64") },
    entries,
    ...(kind === "managed" ? { sliceBuildContext, sliceImage } : {}),
  }
  return Buffer.from(JSON.stringify(request) + "\n")
}

export function parseSigningRequest(bytes, expectedDigest) {
  if (!isDigest(expectedDigest)) fail("expected request digest must be sha256:<64 hex>")
  if (sha(bytes) !== expectedDigest) fail(`signing request digest mismatch: got ${sha(bytes)}, expected ${expectedDigest}`)
  if (bytes.length > 256 * 1024) fail("signing request is too large")
  const request = JSON.parse(bytes)
  if (!Buffer.from(JSON.stringify(request) + "\n").equals(bytes)) fail("signing request is not canonical JSON")
  const { kind } = request
  if (!KINDS[kind] || request.schemaVersion !== 1) fail("signing request has an unsupported kind or schema")
  const keys = ["schemaVersion", "kind", "sourceCommit", "sourceTree", "attestation", "entries", ...(kind === "managed" ? ["sliceBuildContext", "sliceImage"] : [])]
  if (Object.keys(request).join() !== keys.join()) fail("signing request contains unsupported fields")
  if (!/^[0-9a-f]{40}$/.test(request.sourceCommit) || !/^[0-9a-f]{40}$/.test(request.sourceTree)) fail("signing request source identity is invalid")
  const attestationBytes = Buffer.from(request.attestation.base64, "base64")
  if (attestationBytes.toString("base64") !== request.attestation.base64 || sha(attestationBytes) !== request.attestation.sha256) {
    fail("builder attestation digest does not match the signing request")
  }
  const attestation = JSON.parse(attestationBytes)
  if (!Buffer.from(JSON.stringify(attestation) + KINDS[kind].eol).equals(attestationBytes)) {
    fail("builder attestation is not canonical JSON")
  }
  const names = ATTESTATION_NAMES[kind]
  if (
    attestation.schemaVersion !== 1 ||
    attestation.sourceCommit !== request.sourceCommit ||
    attestation.sourceTree !== request.sourceTree ||
    attestation.target !== BUILD_TARGET ||
    attestation.artifacts?.length !== names.length ||
    attestation.artifacts.some((artifact, index) => artifact.name !== names[index] || !isDigest(artifact.sha256))
  ) {
    fail("builder attestation does not match the signing request identity")
  }
  const seen = new Set()
  for (const entry of request.entries) {
    if (Object.keys(entry).join() !== "name,path,sha256" || seen.has(entry.name)) fail("signing request entries are invalid")
    seen.add(entry.name)
    const dependent = SIGNATURE_DEPENDENT.includes(entry.name)
    if (dependent ? entry.sha256 !== null : !isDigest(entry.sha256)) fail(`signing request entry ${entry.name} has the wrong digest state`)
  }
  if (!SIGNATURE_DEPENDENT.every((name) => seen.has(name))) fail("signing request lacks the attestation signature entries")
  const entry = (name) => request.entries.find((candidate) => candidate.name === name)
  if (entry("chariox-build-attestation")?.sha256 !== request.attestation.sha256) fail("manifest attestation entry does not match the attestation document")
  // The manifest must pin the same binaries the builder attested; relay lives in the slice context for managed.
  for (const artifact of attestation.artifacts) {
    const pinned = entry(artifact.name)
    if (pinned && pinned.sha256 !== artifact.sha256) fail(`manifest entry ${artifact.name} differs from the builder attestation`)
  }
  if (kind === "managed") {
    if (!isDigest(request.sliceBuildContext) || !isDigest(request.sliceImage)) fail("managed signing request lacks the slice digests")
    if (entry("chariox-slice-build-context")?.sha256 !== request.sliceBuildContext || entry("chariox-slice-image")?.sha256 !== request.sliceImage) {
      fail("slice build context or image digest differs from the manifest entries")
    }
  }
  return { request, attestationBytes }
}

function filledEntries(request, attestationSignatureBytes, builderPublicKeyBytes) {
  return request.entries.map((entry) => {
    if (entry.name === "chariox-build-attestation-signature") return { ...entry, sha256: sha(attestationSignatureBytes) }
    if (entry.name === "chariox-builder-public-key") return { ...entry, sha256: sha(builderPublicKeyBytes) }
    return entry
  })
}

// Signs the documents of one request. keys.release and keys.builder are Ed25519 private keys.
export function signRequest({ request, attestationBytes }, keys) {
  const { eol, attestationSigner } = KINDS[request.kind]
  const builderKey = keys[attestationSigner]
  const releaseKey = keys.release
  if (!builderKey || !releaseKey) fail(`the ${request.kind} request needs the ${attestationSigner} and release keys`)
  const builderRaw = rawPublicKey(builderKey)
  const files = new Map()
  files.set("attestation.sig", Buffer.from(sign(null, attestationBytes, builderKey).toString("base64") + eol))
  files.set("builder-public-key", Buffer.from(builderRaw.toString("base64") + eol))
  const manifest = manifestBytes(request.kind, request, filledEntries(request, files.get("attestation.sig"), files.get("builder-public-key")))
  files.set("release-manifest.json", manifest)
  files.set("release-manifest.sig", Buffer.from(sign(null, manifest, releaseKey).toString("base64") + eol))
  files.set("release-public-key", Buffer.from(rawPublicKey(releaseKey).toString("base64") + eol))
  return files
}

// Checks one returned set of signed files against the request and the pinned public keys.
// pins.release is required; pins.builder only for managed (boot uses the release key as builder).
export function verifySignedFiles({ request, attestationBytes }, files, pins) {
  for (const name of SIGNED_FILES) if (!files.has(name)) fail(`signed output lacks ${name} for the ${request.kind} release`)
  const label = (name) => `${request.kind} ${name}`
  const { attestationSigner } = KINDS[request.kind]
  const pinned = { release: pins.release, builder: attestationSigner === "release" ? pins.release : pins.builder }
  if (!pinned.release || !pinned.builder) fail(`the ${request.kind} release needs ${attestationSigner === "release" ? "a release" : "release and builder"} public key pin`)
  const builder = publicKeyObject(files.get("builder-public-key"), label("builder-public-key"))
  const release = publicKeyObject(files.get("release-public-key"), label("release-public-key"))
  if (builder.raw.toString("base64") !== pinned.builder) fail(`${label("builder-public-key")} is not the pinned builder key`)
  if (release.raw.toString("base64") !== pinned.release) fail(`${label("release-public-key")} is not the pinned release key`)
  if (!verify(null, attestationBytes, builder.key, signatureBytes(files.get("attestation.sig"), label("attestation.sig")))) {
    fail(`${label("attestation.sig")} does not verify the builder attestation`)
  }
  const expected = manifestBytes(request.kind, request, filledEntries(request, files.get("attestation.sig"), files.get("builder-public-key")))
  if (!expected.equals(files.get("release-manifest.json"))) fail(`${label("release-manifest.json")} is not the manifest the request and signatures produce`)
  if (!verify(null, expected, release.key, signatureBytes(files.get("release-manifest.sig"), label("release-manifest.sig")))) {
    fail(`${label("release-manifest.sig")} does not verify the release manifest`)
  }
  return filledEntries(request, files.get("attestation.sig"), files.get("builder-public-key"))
}
