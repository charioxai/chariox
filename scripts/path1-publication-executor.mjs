#!/usr/bin/env node

import { randomUUID } from "node:crypto"
import { spawnSync } from "node:child_process"
import {
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs"
import { tmpdir } from "node:os"
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path"
import { fileURLToPath } from "node:url"

import {
  BUILDKIT_CONTRACT,
  IMAGE_DIGEST_PATTERN,
  SOURCE_REVISION_LABEL,
  buildCosignCommands,
  buildPublicationPlan,
  createSlsaV02Statement,
  descriptorDigestFromBuildxMetadata,
  publicationContractManifest,
  quarantineDedicatedRunRoot,
  removeDedicatedRunRoot,
  renderBuildKitConfig,
  validateBuildxPublication,
  validateCosignSignature,
  validatePublicationDockerfile,
  validateSlsaV02Attestation,
} from "./publication-release-manifest.mjs"

export const APPLY_GUARD = "PATH1_IMMUTABLE_APPLY"
export const HOST_PREFLIGHT_SCHEMA = "chariox.path1.publication-host-preflight.v1"
export const PUBLICATION_RECEIPT_SCHEMA = "chariox.path1.oss-publication-receipt.v1"
export const JOURNAL_SCHEMA = "chariox.path1.publication-executor-journal.v1"
export const MINIMUM_DEDICATED_VOLUME_FREE_BYTES = 35n * 1024n ** 3n
export const CLOUD_PIN_WARNING =
  "Cloud release pins must match the final integrated source head before any live run."

const SOURCE_REVISION_PATTERN = /^[0-9a-f]{40}$/
const CHECKSUM_PATTERN = /^[0-9a-f]{64}$/
const REDACTED_VALUE_PATTERN = /^\[redacted(?:-[A-Za-z0-9._-]+)?\]$/i
const REDACTED_PATH_PATTERN = /^\[redacted\](?:\/[A-Za-z0-9._-]+)*$/
const REVIEWED_PUBLICATION_RECEIPT_SCHEMA = "chariox.path1.oss-publication-receipt.v1"
const PROTECTED_ENVIRONMENT_KEYS = Object.freeze([
  "CHARIOX_SLICE_IMAGE_SIGNATURE_KEY",
  "COSIGN_PUBLIC_KEY",
])
export const REQUIRED_PREFLIGHT_CHECKS = Object.freeze([
  "dockerDaemonResponsive",
  "buildxMetadataOutputSupported",
  "cosignInstalled",
  "rootPostBuildSafety",
  "publicationVolumePostBuildSafety",
  "builderContract",
  "exactRepositorySupplied",
  "sourceRepositoryPresent",
  "registryAuthPresent",
  "registryPackageWriteCapability",
  "keyBasedSigning",
  "signingKeyInputPresent",
  "signingPasswordInputPresent",
  "hostInventoryPresent",
  "trackedHost",
  "dedicatedPublicationRole",
  "sharedWorkerRejected",
  "activeProfilePresent",
  "immutableRuntimeReleaseDigest",
  "cloudHeadExact",
  "ossHeadExact",
  "localDaemonProtocolExact",
  "relayPeerProtocolExact",
  "quarantinePlanExplicit",
  "ownedCleanupPlanExplicit",
  "broadPruneRejected",
])
const TOOL_NAMES = Object.freeze(["docker", "buildx", "cosign"])

const defaultFileSystem = Object.freeze({
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
})

export class Path1PublicationExecutorError extends Error {
  constructor(message, { code = "publication_blocked", phase = "preflight", cause } = {}) {
    super(message, cause ? { cause } : undefined)
    this.name = "Path1PublicationExecutorError"
    this.code = code
    this.phase = phase
  }
}

function fail(message, options) {
  throw new Path1PublicationExecutorError(message, options)
}

function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value)
}

function valueAt(value, keys) {
  let current = value
  for (const key of keys) {
    if (!isRecord(current)) return undefined
    current = current[key]
  }
  return current
}

function firstDefined(...values) {
  return values.find((value) => value !== undefined && value !== null)
}

function firstString(...values) {
  return values.find((value) => typeof value === "string" && value.trim())?.trim()
}

function firstProtocolValue(...values) {
  const value = firstDefined(...values)
  if (value === undefined) return undefined
  return Number.isSafeInteger(value) && value >= 0 ? value : null
}

function isRedactedValue(value) {
  return typeof value === "string" && REDACTED_VALUE_PATTERN.test(value)
}

function isRedactedPath(value) {
  return typeof value === "string" && REDACTED_PATH_PATTERN.test(value)
}

function requireAbsolutePath(value, field, { allowRedacted = false } = {}) {
  if (allowRedacted && isRedactedPath(value)) return value
  if (typeof value !== "string" || !isAbsolute(value)) {
    fail(`${field} must be an absolute path`)
  }
  return value
}

function requireSourceRevision(value, field = "source revision") {
  if (!SOURCE_REVISION_PATTERN.test(String(value ?? ""))) {
    fail(`${field} must be an exact 40-character lowercase source SHA`)
  }
  return value
}

function requireDigest(value, field = "digest") {
  if (!IMAGE_DIGEST_PATTERN.test(String(value ?? ""))) {
    fail(`${field} must be sha256:<64 lowercase hex>`)
  }
  return value
}

function parseImmutableReference(value, field) {
  if (typeof value !== "string") fail(`${field} must be repository@sha256:<digest>`)
  const at = value.lastIndexOf("@")
  if (at <= 0 || at !== value.indexOf("@")) {
    fail(`${field} must use an immutable repository@sha256 digest reference`)
  }
  const repository = value.slice(0, at)
  const digest = value.slice(at + 1)
  requireDigest(digest, `${field} digest`)
  return { repository, digest, reference: value }
}

function normalizeChecksum(value) {
  if (typeof value !== "string") return null
  const checksum = value.trim().replace(/^sha256:/, "")
  return CHECKSUM_PATTERN.test(checksum) ? checksum : null
}

function looksLikeSecretKey(key) {
  return /(?:secret|token|password|private.?key|credential|auth(?:value|token)|key.?(?:contents?|material|value))/i.test(key)
}

function containsInlineSecretMaterial(value, key = "") {
  if (typeof value === "string") {
    if (isRedactedValue(value) || isRedactedPath(value)) return false
    if (/-----BEGIN [A-Z ]+-----|\b(?:Bearer\s+|gh[pousr]_|github_pat_|xox[baprs]-|AKIA[0-9A-Z]{16})/i.test(value)) {
      return true
    }
    return looksLikeSecretKey(key) && !/path$|environment$/i.test(key) && value.trim().length > 0
  }
  if (Array.isArray(value)) return value.some((child) => containsInlineSecretMaterial(child, key))
  if (!isRecord(value)) return false
  return Object.entries(value).some(([childKey, child]) => containsInlineSecretMaterial(child, childKey))
}

function rejectInlineSecrets(value, label) {
  if (containsInlineSecretMaterial(value)) {
    fail(`${label} must be redacted and must not contain inline secret material`)
  }
}

function containsBroadCleanup(value, key = "") {
  if (typeof value === "string") {
    return /docker\s+(?:system|container|volume|builder)\s+prune|\b(?:system|container|volume|builder)\s+prune\b/i.test(value)
  }
  if (Array.isArray(value)) return value.some((child) => containsBroadCleanup(child, key))
  if (!isRecord(value)) return false
  return Object.entries(value).some(([childKey, child]) => {
    if (childKey === "broadPruneRejected" && child === true) return false
    return containsBroadCleanup(child, childKey)
  })
}

function requireReceiptStatus(receipt, label) {
  const status = String(receipt.status ?? receipt.state ?? "").toLowerCase()
  if (receipt.ready === false || receipt.ok === false || ["blocked", "failed", "invalid", "unready"].includes(status)) {
    fail(`${label} is not ready`)
  }
  if (receipt.ready === true || receipt.ok === true || ["ready", "confirmed", "pass", "passed", "complete", "completed"].includes(status)) {
    return true
  }
  // The Cloud bootstrap receipt has existed in both a boolean-ready and a
  // confirmed-status form. A schema plus explicit checks is sufficient when
  // neither top-level status field was serialized by the redactor.
  if (isRecord(receipt.checks) || isRecord(receipt.gates)) return true
  fail(`${label} does not carry a ready/confirmed status`)
}

function extractSourceRevision(receipt) {
  const candidates = [
    ["sourceRevision", receipt?.sourceRevision],
    ["source_revision", receipt?.source_revision],
    ["sourceSha", receipt?.sourceSha],
    ["source_sha", receipt?.source_sha],
    ["source.revision", valueAt(receipt, ["source", "revision"])],
    ["source.sha", valueAt(receipt, ["source", "sha"])],
    ["source.ossRevision", valueAt(receipt, ["source", "ossRevision"])],
    ["source.revisionLabel.value", valueAt(receipt, ["source", "revisionLabel", "value"])],
    ["contract.sourceRevision", valueAt(receipt, ["contract", "sourceRevision"])],
    ["contract.ossRevision", valueAt(receipt, ["contract", "ossRevision"])],
    ["release.sourceRevision", valueAt(receipt, ["release", "sourceRevision"])],
    ["release.ossRevision", valueAt(receipt, ["release", "ossRevision"])],
    ["ossRevision", receipt?.ossRevision],
  ].filter(([, value]) => value !== undefined && value !== null)
  if (candidates.length === 0) fail("reviewed publication receipt does not carry an authoritative source SHA")
  const values = new Set(candidates.map(([, value]) => String(value)))
  if (values.size !== 1) fail("reviewed publication receipt has conflicting source SHA fields")
  const [field, value] = candidates[0]
  return { sourceRevision: requireSourceRevision(value, `publication receipt ${field}`), field }
}

function extractCloudRevision(receipt) {
  const candidates = [
    ["cloudRevision", receipt?.cloudRevision],
    ["source.cloudRevision", valueAt(receipt, ["source", "cloudRevision"])],
    ["contract.cloudRevision", valueAt(receipt, ["contract", "cloudRevision"])],
    ["release.cloudRevision", valueAt(receipt, ["release", "cloudRevision"])],
    ["cloudReleasePins.cloudRevision", valueAt(receipt, ["cloudReleasePins", "cloudRevision"])],
  ].filter(([, value]) => value !== undefined && value !== null)
  if (candidates.length === 0) fail("reviewed publication receipt does not carry the Cloud release SHA")
  const values = new Set(candidates.map(([, value]) => String(value)))
  if (values.size !== 1) fail("reviewed publication receipt has conflicting Cloud release SHA fields")
  const [field, value] = candidates[0]
  return { cloudRevision: requireSourceRevision(value, `publication receipt ${field}`), field }
}

function extractRepository(receipt, suppliedRepository) {
  const imageReference = firstString(
    receipt?.image?.reference,
    receipt?.image?.repository,
    receipt?.publication?.reference,
  )
  const imageRepository = imageReference
    ? parseImmutableReference(imageReference, "reviewed publication image reference").repository
    : undefined
  const receiptRepositories = [
    receipt?.repository,
    receipt?.registryRepository,
    imageRepository,
    valueAt(receipt, ["registry", "repository"]),
    valueAt(receipt, ["contract", "repository"]),
    valueAt(receipt, ["publication", "repository"]),
  ].filter((value) => typeof value === "string" && value.trim()).map((value) => value.trim())
  for (const candidate of [suppliedRepository, ...receiptRepositories]) {
    if (!candidate) continue
    const lastSlash = candidate.lastIndexOf("/")
    const lastColon = candidate.lastIndexOf(":")
    if (candidate.includes("@") || (lastColon > lastSlash && lastColon >= 0)) {
      fail("publication refuses mutable tagged or digest-qualified repositories")
    }
  }
  const repositoryValues = [...new Set(receiptRepositories)]
  if (suppliedRepository) repositoryValues.push(suppliedRepository)
  if (new Set(repositoryValues).size > 1) {
    fail("publication repository fields do not agree")
  }
  const receiptRepository = receiptRepositories[0]
  if (suppliedRepository && receiptRepository && suppliedRepository !== receiptRepository) {
    fail("supplied repository does not match the reviewed publication receipt")
  }
  const repository = firstString(suppliedRepository, receiptRepository)
  if (!repository) fail("an exact registry repository is required")
  return repository
}

function extractRepoRoot(options, hostPreflight, publicationReceipt) {
  const candidate = firstString(
    options.repoRoot,
    options.sourceRoot,
    publicationReceipt?.repoRoot,
    valueAt(publicationReceipt, ["source", "path"]),
    valueAt(hostPreflight, ["paths", "repository"]),
  )
  return requireAbsolutePath(candidate, "source repository path")
}

function protocolValues(document) {
  const local = firstProtocolValue(
    document?.localDaemonProtocolVersion,
    document?.localDaemonProtocol,
    valueAt(document, ["protocols", "localDaemon"]),
    valueAt(document, ["protocols", "localDaemonProtocolVersion"]),
    valueAt(document, ["protocol", "localDaemonProtocolVersion"]),
    valueAt(document, ["release", "localDaemonProtocolVersion"]),
    valueAt(document, ["contract", "localDaemonProtocolVersion"]),
  )
  const relay = firstProtocolValue(
    document?.relayPeerProtocolVersion,
    document?.relayPeerProtocol,
    valueAt(document, ["protocols", "relayPeer"]),
    valueAt(document, ["protocols", "relayPeerProtocolVersion"]),
    valueAt(document, ["protocol", "relayPeerProtocolVersion"]),
    valueAt(document, ["release", "relayPeerProtocolVersion"]),
    valueAt(document, ["contract", "relayPeerProtocolVersion"]),
  )
  return { local, relay }
}

function resolveProtocols(options, publicationReceipt, hostPreflight, bootstrapReceipt) {
  const documents = [options.protocols, publicationReceipt, hostPreflight, bootstrapReceipt]
  const localValues = documents.map((document) => protocolValues(document).local).filter((value) => value !== undefined)
  const relayValues = documents.map((document) => protocolValues(document).relay).filter((value) => value !== undefined)
  if (localValues.length === 0 || relayValues.length === 0) {
    fail("publication receipts must carry both protocol versions")
  }
  if (localValues.includes(null) || relayValues.includes(null)
    || new Set(localValues).size !== 1 || new Set(relayValues).size !== 1) {
    fail("publication receipts carry stale or mismatched protocol versions")
  }
  return { localDaemonProtocolVersion: localValues[0], relayPeerProtocolVersion: relayValues[0] }
}

function resolveCloudPinStatus(publicationReceipt, sourceRevision) {
  const explicitFlags = [
    publicationReceipt?.cloudReleasePinsMatchFinalIntegratedSourceHead,
    publicationReceipt?.cloudReleasePinsMatchSourceRevision,
    valueAt(publicationReceipt, ["release", "cloudReleasePinsMatchFinalIntegratedSourceHead"]),
    valueAt(publicationReceipt, ["release", "pinsMatchFinalIntegratedSourceHead"]),
    valueAt(publicationReceipt, ["cloudReleasePins", "matched"]),
    valueAt(publicationReceipt, ["cloudReleasePins", "verified"]),
  ].filter((value) => typeof value === "boolean")
  let mismatch = explicitFlags.some((value) => value === false)

  const finalIntegrated = firstString(
    publicationReceipt?.finalIntegratedSourceHead,
    publicationReceipt?.finalIntegratedSourceRevision,
    valueAt(publicationReceipt, ["release", "finalIntegratedSourceHead"]),
    valueAt(publicationReceipt, ["release", "finalIntegratedSourceRevision"]),
    valueAt(publicationReceipt, ["cloudReleasePins", "finalIntegratedSourceHead"]),
  )
  const ossPin = firstString(
    publicationReceipt?.ossRevision,
    valueAt(publicationReceipt, ["source", "ossRevision"]),
    valueAt(publicationReceipt, ["source", "revisionLabel", "value"]),
    valueAt(publicationReceipt, ["release", "ossRevision"]),
    valueAt(publicationReceipt, ["cloudReleasePins", "ossRevision"]),
  )
  if (finalIntegrated && finalIntegrated !== sourceRevision) mismatch = true
  if (ossPin && ossPin !== sourceRevision) mismatch = true
  const matched = !mismatch && (explicitFlags.some((value) => value === true)
    || Boolean(finalIntegrated && ossPin && finalIntegrated === sourceRevision && ossPin === sourceRevision)
    || Boolean(ossPin && ossPin === sourceRevision))
  return {
    matched,
    finalIntegratedSourceHead: finalIntegrated ?? null,
    ossRevision: ossPin ?? null,
  }
}

function checkMap(receipt) {
  return isRecord(receipt?.checks) ? receipt.checks : isRecord(receipt?.gates) ? receipt.gates : null
}

function validateHostPreflight(receipt) {
  if (!isRecord(receipt) || receipt.schema !== HOST_PREFLIGHT_SCHEMA) {
    fail(`host preflight receipt must use ${HOST_PREFLIGHT_SCHEMA}`)
  }
  if (receipt.ready !== true) fail("host preflight receipt is not ready")
  const checks = checkMap(receipt)
  if (!checks) fail("host preflight receipt has no check map")
  for (const check of REQUIRED_PREFLIGHT_CHECKS) {
    if (checks[check] !== true) fail(`host preflight check failed: ${check}`)
  }
  if (containsBroadCleanup(receipt)) fail("host preflight contains a broad Docker cleanup")

  const paths = isRecord(receipt.paths) ? receipt.paths : {}
  for (const field of [
    "registryAuth",
    "signingKey",
    "signingPasswordFile",
    "profile",
    "hostInventory",
    "builderRoot",
    "publicationVolume",
    "quarantine",
    "ownedCleanup",
  ]) {
    requireAbsolutePath(paths[field], `host preflight paths.${field}`, { allowRedacted: true })
  }
  const cleanup = receipt.cleanup
  if (!isRecord(cleanup) || cleanup.broadPruneRejected !== true) {
    fail("host preflight must explicitly reject broad cleanup")
  }
  if (!isRecord(cleanup.quarantine) || cleanup.quarantine.ownedOnly !== true) {
    fail("host preflight quarantine ownership is missing")
  }
  if (!isRecord(cleanup.owned) || cleanup.owned.ownedOnly !== true) {
    fail("host preflight owned cleanup scope is missing")
  }
  if (receipt.host?.sharedWorker === true || receipt.host?.dedicatedPublicationRole === false) {
    fail("shared-worker hosts are not allowed for immutable publication")
  }
  return receipt
}

function toolChecksumSources(receipt) {
  const sources = [
    receipt?.toolChecksums,
    valueAt(receipt, ["tools", "checksums"]),
    valueAt(receipt, ["tools", "toolChecksums"]),
    valueAt(receipt, ["checks", "toolChecksums"]),
    receipt?.tools,
  ].filter(isRecord)
  return sources
}

function checksumEntry(source, toolName) {
  if (Array.isArray(source)) {
    return source.find((entry) => entry?.tool === toolName || entry?.name === toolName) ?? null
  }
  if (!isRecord(source)) return null
  if (isRecord(source[toolName])) return source[toolName]
  return null
}

function bootstrapArtifactChecksumEntry(receipt, toolName) {
  const artifactName = toolName === "docker" ? "dockerEngine" : toolName
  const artifact = receipt?.artifacts?.[artifactName]
  return isRecord(artifact) ? artifact : null
}

function hasChecksumEvidence(entry) {
  return isRecord(entry) && [
    entry.sha256,
    entry.checksum,
    entry.digest,
    entry.actualSha256,
    entry.expectedSha256,
    entry.expected,
    entry.expectedDigest,
    entry.checksumVerified,
    entry.verified,
    entry.status,
    entry.result,
  ].some((value) => value !== undefined)
}

function validateToolChecksums(hostPreflight, bootstrapReceipt) {
  const sources = [
    ...toolChecksumSources(hostPreflight),
    ...toolChecksumSources(bootstrapReceipt),
  ]
  const parentVerified = [hostPreflight, bootstrapReceipt].some((receipt) => (
    receipt?.toolChecksumsVerified === true
    || receipt?.checksumsVerified === true
    || valueAt(receipt, ["tools", "checksumsVerified"]) === true
  ))
  if (sources.length === 0) fail("verified Docker, Buildx, and Cosign tool checksums are required")

  const normalized = {}
  for (const toolName of TOOL_NAMES) {
    const entry = sources.map((source) => checksumEntry(source, toolName)).find(hasChecksumEvidence)
      ?? bootstrapArtifactChecksumEntry(bootstrapReceipt, toolName)
    if (!isRecord(entry)) fail(`tool checksum is missing: ${toolName}`)
    const checksum = normalizeChecksum(firstString(entry.sha256, entry.checksum, entry.digest, entry.actualSha256))
    const expected = normalizeChecksum(firstString(entry.expectedSha256, entry.expected, entry.expectedDigest))
    const actual = normalizeChecksum(firstString(entry.actualSha256, entry.actual, entry.sha256, entry.checksum, entry.digest))
    const redactedVerified = entry.checksumVerified === true
      && bootstrapReceipt?.checks?.artifactChecksumsVerified === true
    if ((!checksum && !redactedVerified) || (expected && actual && expected !== actual)) {
      fail(`tool checksum is invalid or mismatched: ${toolName}`)
    }
    const verified = entry.verified === true
      || entry.status === "verified"
      || entry.result === "verified"
      || redactedVerified
      || (parentVerified && entry.verified !== false)
    if (!verified) fail(`tool checksum is not verified: ${toolName}`)
    normalized[toolName] = checksum ?? "[redacted-verified]"
  }
  return normalized
}

function validateBootstrapReceipt(receipt, expectedProtocols) {
  if (!isRecord(receipt)) fail("a redacted Cloud bootstrap receipt is required")
  const schema = String(receipt.schema ?? receipt.receiptSchema ?? receipt.kind ?? "")
  if (!/bootstrap/i.test(schema)) fail("Cloud bootstrap receipt schema is invalid")
  requireReceiptStatus(receipt, "Cloud bootstrap receipt")
  if (receipt.redacted !== undefined && receipt.redacted !== true) {
    fail("Cloud bootstrap receipt must be redacted")
  }
  if (receipt.mode !== undefined && receipt.mode !== "apply") {
    fail("Cloud bootstrap receipt must come from an applied bootstrap")
  }
  if (receipt.outcome !== undefined && !["applied", "already-correct"].includes(receipt.outcome)) {
    fail("Cloud bootstrap receipt does not report a completed bootstrap")
  }
  rejectInlineSecrets(receipt, "Cloud bootstrap receipt")
  if (containsBroadCleanup(receipt)) fail("Cloud bootstrap receipt contains a broad cleanup")

  const checks = checkMap(receipt)
  if (checks && Object.entries(checks).some(([, value]) => value !== true)) {
    fail("Cloud bootstrap receipt contains a failed check")
  }
  for (const check of [
    "publicationVolumePostBuildSafety",
    "artifactChecksumsVerified",
    "trackedDedicatedPublicationRole",
    "sharedWorkerRejected",
    "exactReleaseHeads",
    "exactProtocols",
    "broadPruneRejected",
  ]) {
    if (checks && Object.hasOwn(checks, check) && checks[check] !== true) {
      fail(`Cloud bootstrap receipt check failed: ${check}`)
    }
  }
  const host = receipt.host ?? receipt.inventory ?? receipt
  if (host?.sharedWorker === true || host?.dedicated === false || host?.role === "shared-worker") {
    fail("shared-worker bootstrap hosts are not allowed")
  }
  if (host?.role && host.role !== "dedicated-publication") {
    fail("bootstrap host role is not dedicated-publication")
  }

  const protocols = protocolValues(receipt)
  if (protocols.local !== undefined && protocols.local !== expectedProtocols.localDaemonProtocolVersion) {
    fail("Cloud bootstrap receipt carries a stale local-daemon protocol")
  }
  if (protocols.relay !== undefined && protocols.relay !== expectedProtocols.relayPeerProtocolVersion) {
    fail("Cloud bootstrap receipt carries a stale relay peer protocol")
  }

  const freeBytes = firstDefined(
    receipt.publicationVolumeFreeBytes,
    receipt.storage?.publicationVolumeFreeBytes,
    receipt.storage?.dedicatedVolumeFreeBytes,
    receipt.volume?.freeBytes,
  )
  if (freeBytes !== undefined) {
    let bytes = null
    if (typeof freeBytes === "bigint") bytes = freeBytes
    else if (typeof freeBytes === "number" && Number.isSafeInteger(freeBytes) && freeBytes >= 0) bytes = BigInt(freeBytes)
    else if (typeof freeBytes === "string" && /^\d+$/.test(freeBytes)) bytes = BigInt(freeBytes)
    if (bytes === null || bytes < MINIMUM_DEDICATED_VOLUME_FREE_BYTES) {
      fail("dedicated publication volume has insufficient post-build headroom")
    }
  }
  return receipt
}

function validateReceiptAuthority(receipt) {
  if (!isRecord(receipt)) fail("a reviewed publication receipt is required")
  rejectInlineSecrets(receipt, "reviewed publication receipt")
  if (receipt.reviewed === false || receipt.authoritative === false || receipt.verified === false) {
    fail("reviewed publication receipt is not authoritative")
  }
  const status = String(receipt.status ?? receipt.state ?? "").toLowerCase()
  if (["unreviewed", "draft", "rejected", "failed", "blocked"].includes(status)) {
    fail("reviewed publication receipt is not approved")
  }
  if (receipt.schema === REVIEWED_PUBLICATION_RECEIPT_SCHEMA) {
    if (status !== "published") fail("reviewed publication receipt is not published")
    const source = receipt.source
    if (!isRecord(source) || source.treeClean !== true) {
      fail("reviewed publication receipt source tree is not clean")
    }
    if (source.cloudRevision !== undefined) {
      requireSourceRevision(source.cloudRevision, "publication receipt source.cloudRevision")
    }
    const sourceRevision = requireSourceRevision(source.ossRevision, "publication receipt source.ossRevision")
    if (source.revisionLabel?.name !== SOURCE_REVISION_LABEL
      || source.revisionLabel?.value !== sourceRevision) {
      fail("reviewed publication receipt source label is not bound to source.ossRevision")
    }
    const image = receipt.image
    const imageReference = parseImmutableReference(image?.reference, "reviewed publication image reference")
    if (image?.digest !== imageReference.digest
      || image?.signatureVerification !== "verified"
      || image?.attestationVerification !== "verified") {
      fail("reviewed publication receipt image verification is incomplete")
    }
    if (!Array.isArray(image?.manifests) || image.manifests.length !== 1) {
      fail("reviewed publication receipt must contain exactly one manifest")
    }
    const manifest = image.manifests[0]
    if (manifest?.platform !== "linux/amd64" || manifest?.digest !== imageReference.digest) {
      fail("reviewed publication receipt manifest is not the single linux/amd64 digest")
    }
    if (receipt.publication?.status !== "succeeded" || receipt.publication?.quarantine !== "none") {
      fail("reviewed publication receipt is not quarantine-free")
    }
  }
  return receipt
}

function protectedPaths(hostPreflight) {
  const paths = hostPreflight.paths ?? {}
  return {
    registryAuthPath: paths.registryAuth ?? "[redacted-registry-auth-path]",
    signingKeyPath: paths.signingKey ?? "[redacted-signing-key-path]",
    signingPasswordFilePath: paths.signingPasswordFile ?? "[redacted-signing-password-file-path]",
  }
}

function normalizeInputs(options, mode) {
  const publicationReceipt = options.reviewedPublicationReceipt ?? options.publicationReceipt
  const hostPreflight = options.hostPreflightReceipt ?? options.hostPreflight
  const bootstrapReceipt = options.bootstrapReceipt ?? options.hostBootstrapReceipt
  validateReceiptAuthority(publicationReceipt)
  validateHostPreflight(hostPreflight)
  const source = extractSourceRevision(publicationReceipt)
  const cloud = extractCloudRevision(publicationReceipt)
  const repository = extractRepository(publicationReceipt, options.repository)
  const repoRoot = extractRepoRoot(options, hostPreflight, publicationReceipt)
  const protocols = resolveProtocols(options, publicationReceipt, hostPreflight, bootstrapReceipt)
  const bootstrap = validateBootstrapReceipt(bootstrapReceipt, protocols)
  const toolChecksums = validateToolChecksums(hostPreflight, bootstrap)
  const cloudPins = resolveCloudPinStatus(publicationReceipt, source.sourceRevision)
  if (mode === "apply" && !cloudPins.matched) fail(CLOUD_PIN_WARNING)

  const hostRepository = hostPreflight.paths?.repository
  if (hostRepository && !isRedactedPath(hostRepository) && resolve(hostRepository) !== resolve(repoRoot)) {
    fail("source repository path does not match the Cloud host-preflight receipt")
  }
  return Object.freeze({
    publicationReceipt,
    hostPreflight,
    bootstrapReceipt: bootstrap,
    sourceRevision: source.sourceRevision,
    sourceRevisionField: source.field,
    cloudRevision: cloud.cloudRevision,
    cloudRevisionField: cloud.field,
    repository,
    repoRoot,
    protocols,
    toolChecksums,
    cloudPins,
    protectedPaths: protectedPaths(hostPreflight),
  })
}

function redactedProtectedPaths(paths) {
  return Object.fromEntries(Object.keys(paths).map((key) => [key, "[redacted]"]))
}

function defaultRunCommand(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd,
    env: options.env,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  })
  if (result.error || result.status !== 0) {
    const suffix = result.signal ? ` (${result.signal})` : ""
    throw new Error(`${command} command failed${suffix}`)
  }
  return { stdout: result.stdout ?? "", stderr: result.stderr ?? "" }
}

function commandResult(value) {
  if (value && typeof value.then === "function") {
    fail("publication command runners must be synchronous")
  }
  if (value === undefined || value === null) return { stdout: "", stderr: "" }
  if (typeof value === "string") return { stdout: value, stderr: "" }
  return { ...value, stdout: String(value.stdout ?? ""), stderr: String(value.stderr ?? "") }
}

function isInterruption(error) {
  const code = String(error?.code ?? "")
  const signal = String(error?.signal ?? "")
  const message = String(error?.message ?? error)
  return ["EINTR", "SIGINT", "SIGTERM", "INTERRUPTED"].includes(code.toUpperCase())
    || ["SIGINT", "SIGTERM"].includes(signal.toUpperCase())
    || /(?:interrupted|SIGINT|SIGTERM)/i.test(message)
}

function safeErrorMessage(error, secretValues = []) {
  let message = String(error?.message ?? error ?? "publication failed")
  for (const value of secretValues) {
    if (value) message = message.replaceAll(String(value), "[redacted]")
  }
  return message
    .replace(/(?:Bearer\s+)[^\s]+/gi, "Bearer [redacted]")
    .replace(/(?:gh[pousr]_)[A-Za-z0-9_-]+/g, "[redacted-token]")
    .replace(/(?:xox[baprs]-)[A-Za-z0-9-]+/g, "[redacted-token]")
    .slice(0, 500)
}

function runStep(run, command, args, { cwd, env, phase, secretValues }) {
  try {
    const result = commandResult(run(command, args, { cwd, env }))
    if (result.status !== undefined && result.status !== 0) {
      throw new Error(`${command} command returned a non-zero status`)
    }
    if (result.error) throw result.error
    return result
  } catch (error) {
    if (error instanceof Path1PublicationExecutorError) throw error
    throw new Path1PublicationExecutorError(
      `${phase} command failed: ${safeErrorMessage(error, secretValues)}`,
      { code: isInterruption(error) ? "interrupted" : "command_failed", phase, cause: error },
    )
  }
}

function parseJson(text, description) {
  try {
    return JSON.parse(String(text ?? ""))
  } catch {
    fail(`${description} must be strict JSON`)
  }
}

function jsonForWrite(value) {
  return `${JSON.stringify(value, null, 2)}\n`
}

function writeJson(fs, target, value, { mode = 0o600 } = {}) {
  fs.writeFileSync(target, jsonForWrite(value), { encoding: "utf8", mode })
}

function writeJsonAtomic(fs, target, value) {
  const parent = dirname(target)
  fs.mkdirSync(parent, { recursive: true, mode: 0o700 })
  const temporary = `${target}.tmp-${process.pid}-${randomUUID().slice(0, 8)}`
  try {
    fs.writeFileSync(temporary, jsonForWrite(value), { encoding: "utf8", mode: 0o600 })
    fs.renameSync(temporary, target)
  } catch (error) {
    try {
      fs.rmSync(temporary, { force: true })
    } catch {
      // Preserve the journal-write failure.
    }
    fail("publication journal could not be written", { code: "journal_write_failed", phase: "journal", cause: error })
  }
}

function readJson(fs, target, description) {
  try {
    return parseJson(fs.readFileSync(target, "utf8"), description)
  } catch (error) {
    if (error instanceof Path1PublicationExecutorError) throw error
    fail(`${description} could not be read`, { code: "journal_read_failed", phase: "journal", cause: error })
  }
}

function journalPathInsideRunRoot(journalPath, runRoot) {
  const child = relative(runRoot, journalPath)
  return child === "" || (!child.startsWith("..") && !child.startsWith(`..${sep}`) && !isAbsolute(child))
}

function identityFromOptions(options, normalized, fs) {
  const explicitRunRoot = options.runRoot ? resolve(options.runRoot) : null
  const provisionalJournalPath = options.journalPath
    ? requireAbsolutePath(resolve(options.journalPath), "journal path")
    : explicitRunRoot
      ? `${explicitRunRoot}.journal.json`
      : null
  let existing = null
  let journalPath = provisionalJournalPath
  if (journalPath && fs.existsSync(journalPath)) existing = readJson(fs, journalPath, "publication journal")
  const existingOwned = existing?.owned ?? {}
  const stagingNonce = firstString(existingOwned.stagingNonce, options.stagingNonce)
    ?? randomUUID().replace(/[^A-Za-z0-9._-]/g, "-").slice(0, 48)
  const runRoot = resolve(firstString(existingOwned.runRoot, explicitRunRoot) ?? join(tmpdir(), `chariox-publication-run-${stagingNonce}`))
  journalPath = journalPath ?? `${runRoot}.journal.json`
  journalPath = requireAbsolutePath(journalPath, "journal path")
  if (journalPathInsideRunRoot(journalPath, runRoot)) {
    fail("publication journal must be outside the owned run root")
  }
  const builderName = firstString(existingOwned.builderName, options.builderName)
    ?? `chariox-publication-builder-${stagingNonce}`
  if (existing && existing.schema !== JOURNAL_SCHEMA) fail("publication journal schema is unsupported")
  if (existing && existing.sourceRevision !== normalized.sourceRevision) {
    fail("publication journal source SHA does not match the reviewed publication receipt")
  }
  if (existing && existing.repository !== normalized.repository) {
    fail("publication journal repository does not match the reviewed publication receipt")
  }
  return { existing, stagingNonce, runRoot, journalPath, builderName }
}

function planKey(plan, identity, normalized) {
  return JSON.stringify({
    repoRoot: plan.repoRoot,
    repository: plan.repository,
    sourceRevision: plan.sourceRevision,
    runRoot: plan.runRoot,
    builderName: plan.builderName,
    stagingNonce: identity.stagingNonce,
    stagingTag: plan.stagingTag,
    protocols: normalized?.protocols ?? null,
  })
}

function newJournal(plan, identity, normalized) {
  return {
    schema: JOURNAL_SCHEMA,
    version: 1,
    status: "planned",
    planKey: planKey(plan, identity, normalized),
    repository: normalized.repository,
    sourceRevision: normalized.sourceRevision,
    protocols: normalized.protocols,
    owned: {
      builderName: plan.builderName,
      runRoot: plan.runRoot,
      stagingNonce: identity.stagingNonce,
      stagingTag: plan.stagingTag,
    },
    stages: {
      runRootCreated: false,
      configWritten: false,
      builderCreateStarted: false,
      builderCreated: false,
      buildStarted: false,
      buildCompleted: false,
      inspectionCompleted: false,
      predicateWritten: false,
      signed: false,
      attested: false,
      signatureVerified: false,
      attestationVerified: false,
      builderRemoved: false,
      runRootRemoved: false,
    },
    digest: null,
    reference: null,
    evidence: {},
    quarantine: null,
    receipt: null,
  }
}

function assertJournalMatches(journal, plan, identity, normalized) {
  if (!isRecord(journal) || journal.schema !== JOURNAL_SCHEMA || journal.version !== 1) {
    fail("publication journal schema is unsupported")
  }
  if (journal.planKey !== planKey(plan, identity, normalized)) {
    fail("publication journal does not match the immutable execution plan")
  }
  if (journal.sourceRevision !== normalized.sourceRevision || journal.repository !== normalized.repository) {
    fail("publication journal source or repository binding changed")
  }
  if (journal.owned?.stagingTag !== plan.stagingTag) fail("publication journal staging tag is not owned by this run")
  if (["quarantined", "quarantine-failed", "completed"].includes(journal.status) && journal.status !== "completed") {
    fail("quarantined publication evidence cannot be resumed")
  }
}

function updateJournal(fs, journalPath, journal, patch = {}) {
  Object.assign(journal, patch)
  journal.sequence = Number.isSafeInteger(journal.sequence) ? journal.sequence + 1 : 1
  writeJsonAtomic(fs, journalPath, journal)
}

function stageJournal(fs, journalPath, journal, stage, value = true) {
  journal.stages[stage] = value
  updateJournal(fs, journalPath, journal)
}

function readOptionalJson(fs, target, description) {
  if (!fs.existsSync(target)) return null
  return readJson(fs, target, description)
}

function sourceLabelFromInspection(inspection) {
  const maps = [
    inspection?.Labels,
    inspection?.labels,
    inspection?.Config?.Labels,
    inspection?.Config?.labels,
    inspection?.config?.Labels,
    inspection?.config?.labels,
    inspection?.Image?.Config?.Labels,
    inspection?.image?.config?.Labels,
  ]
  for (const map of maps) {
    if (isRecord(map) && typeof map[SOURCE_REVISION_LABEL] === "string") return map[SOURCE_REVISION_LABEL]
  }
  return null
}

function validateSourceLabel(inspection, sourceRevision) {
  const label = sourceLabelFromInspection(inspection)
  if (label !== sourceRevision) {
    fail("published image source label does not match the reviewed source SHA", {
      code: "source_label_mismatch",
      phase: "metadata",
    })
  }
  return label
}

function assertContractCommands(plan, cosignCommands) {
  if (!plan.builderCommand.includes(`image=${BUILDKIT_CONTRACT.reference}`)) {
    fail("publication builder is not pinned to the reviewed BuildKit digest")
  }
  if (!plan.buildCommand.includes("--pull") || !plan.buildCommand.includes("--push")) {
    fail("publication build must pull and push through the manifest contract")
  }
  const tagIndexes = plan.buildCommand.reduce((indexes, value, index) => (
    value === "--tag" ? [...indexes, index] : indexes
  ), [])
  if (tagIndexes.length !== 1 || plan.buildCommand[tagIndexes[0] + 1] !== plan.stagingTag) {
    fail("publication build must push exactly one owned staging tag")
  }
  if (plan.buildCommand.filter((value) => value === "--push").length !== 1) {
    fail("publication build must push exactly once")
  }
  if (plan.buildCommand.includes("latest") || plan.buildCommand.some((value) => /prune/i.test(value))) {
    fail("publication command plan contains a mutable tag or broad cleanup")
  }
  if (plan.cleanupBuilderCommand.some((value) => /prune|rm\s+-rf/i.test(value))) {
    fail("publication cleanup is broader than the dedicated builder")
  }
  for (const command of [cosignCommands.sign, cosignCommands.attest, cosignCommands.verify, cosignCommands.verifyAttestation]) {
    if (command.includes("fulcio") || command.includes("rekor") || command.includes("identity-token")) {
      fail("publication Cosign policy must remain key-based")
    }
  }
}

function buildNormalizedPlan(normalized, identity) {
  const plan = buildPublicationPlan({
    repoRoot: normalized.repoRoot,
    repository: normalized.repository,
    sourceRevision: normalized.sourceRevision,
    runRoot: identity.runRoot,
    builderName: identity.builderName,
    stagingNonce: identity.stagingNonce,
  })
  const placeholderReference = `${normalized.repository}@<descriptor-digest>`
  const cosignCommands = buildCosignCommands({
    reference: placeholderReference,
    predicateFile: plan.predicateFile,
  })
  assertContractCommands(plan, cosignCommands)
  return Object.freeze({ ...plan, cosignCommands, placeholderReference })
}

function publicPlan(plan, normalized, identity) {
  return {
    mode: "plan",
    ready: normalized.cloudPins.matched,
    blockedReason: normalized.cloudPins.matched ? null : CLOUD_PIN_WARNING,
    warning: CLOUD_PIN_WARNING,
    contract: publicationContractManifest(),
    sourceRevision: normalized.sourceRevision,
    sourceRevisionField: normalized.sourceRevisionField,
    repository: normalized.repository,
    protocols: normalized.protocols,
    cloudReleasePins: {
      matchedFinalIntegratedSourceHead: normalized.cloudPins.matched,
      finalIntegratedSourceHead: normalized.cloudPins.finalIntegratedSourceHead,
      ossRevision: normalized.cloudPins.ossRevision,
    },
    resources: {
      builderName: identity.builderName,
      runRoot: identity.runRoot,
      stagingTag: plan.stagingTag,
      journalPath: identity.journalPath,
    },
    commands: {
      builder: plan.builderCommand,
      build: plan.buildCommand,
      inspect: plan.inspectCommand,
      sign: plan.cosignCommands.sign,
      attest: plan.cosignCommands.attest,
      verify: plan.cosignCommands.verify,
      verifyAttestation: plan.cosignCommands.verifyAttestation,
      cleanupBuilder: plan.cleanupBuilderCommand,
    },
    protectedInputs: redactedProtectedPaths(normalized.protectedPaths),
    toolChecksums: normalized.toolChecksums,
    cleanup: {
      allowed: ["dedicated builder", "dedicated publication run root"],
      stagingTagDeletion: false,
      broadDockerGarbageCollection: false,
    },
  }
}

function exactCleanSource(normalized, run, env, secretValues) {
  const head = runStep(run, "git", ["rev-parse", "--verify", "HEAD"], {
    cwd: normalized.repoRoot,
    env,
    phase: "source head",
    secretValues,
  }).stdout.trim()
  if (head !== normalized.sourceRevision) {
    fail("clean source HEAD does not equal the authoritative reviewed publication SHA", {
      code: "source_head_mismatch",
      phase: "source",
    })
  }
  const status = runStep(run, "git", ["status", "--porcelain", "--untracked-files=all"], {
    cwd: normalized.repoRoot,
    env,
    phase: "source cleanliness",
    secretValues,
  }).stdout.trim()
  if (status) {
    fail("publication requires a clean source tree", { code: "dirty_source", phase: "source" })
  }
}

function readPublicationDockerfile(normalized) {
  let dockerfile
  try {
    dockerfile = readFileSync(join(normalized.repoRoot, "docker/publication/Dockerfile"), "utf8")
  } catch (error) {
    fail("publication Dockerfile could not be read", { code: "dockerfile_missing", phase: "source", cause: error })
  }
  try {
    validatePublicationDockerfile(dockerfile)
  } catch (error) {
    if (error instanceof Path1PublicationExecutorError) throw error
    fail(safeErrorMessage(error), { code: "dockerfile_contract_mismatch", phase: "source", cause: error })
  }
  return dockerfile
}

function protectedEnvironment(options) {
  const env = { ...process.env, ...(options.env ?? {}) }
  const secretValues = []
  for (const key of PROTECTED_ENVIRONMENT_KEYS) {
    if (!String(env[key] ?? "").trim()) fail(`required protected environment key is absent: ${key}`)
    secretValues.push(String(env[key]))
  }
  // The registry and signing paths remain paths in the redacted receipts. No
  // secret file is read or copied here; Docker/Cosign inherit the protected
  // environment and the host's preconfigured file-descriptor/path bindings.
  return { env, secretValues }
}

function writeFailureEvidence(fs, runRoot, journal, error, secretValues = []) {
  if (!fs.existsSync(runRoot)) return
  try {
    writeJson(fs, join(runRoot, "publication-failure.json"), {
      schema: "chariox.path1.publication-failure-evidence.v1",
      phase: error?.phase ?? "publication",
      code: error?.code ?? "publication_failed",
      reason: safeErrorMessage(error, secretValues),
      digest: journal.digest,
      reference: journal.reference,
      stagingTag: "[redacted-owned-staging-tag]",
    })
  } catch {
    // The original publication failure is more useful than a secondary
    // evidence-write failure; the journal still records the failure.
  }
}

function quarantineAfterFailure(fs, journalPath, journal, error, secretValues = []) {
  const reason = safeErrorMessage(error, secretValues)
  const digest = journal.digest && IMAGE_DIGEST_PATTERN.test(journal.digest) ? journal.digest : null
  const quarantinePath = `${journal.owned.runRoot}.quarantine`
  journal.status = "quarantining"
  journal.quarantine = {
    immutableDigest: digest,
    immutableReference: digest ? `${journal.repository}@${digest}` : null,
    reason,
    evidencePath: quarantinePath,
    stagingTagRetained: true,
  }
  updateJournal(fs, journalPath, journal)
  if (fs.existsSync(journal.owned.runRoot)) {
    try {
      quarantineDedicatedRunRoot(journal.owned.runRoot, reason, { rename: fs.renameSync })
      journal.status = "quarantined"
      updateJournal(fs, journalPath, journal)
    } catch (quarantineError) {
      journal.status = "quarantine-failed"
      journal.quarantine.error = safeErrorMessage(quarantineError)
      updateJournal(fs, journalPath, journal)
    }
  } else {
    journal.status = "quarantined"
    updateJournal(fs, journalPath, journal)
  }
}

function createReceipt(normalized, digest, reference, sourceLabel, now = Date.now()) {
  const issuedAt = new Date(now)
  if (!Number.isFinite(issuedAt.valueOf())) fail("publication receipt clock is invalid")
  const validUntil = new Date(issuedAt.valueOf() + 15 * 60 * 1000)
  return {
    schema: PUBLICATION_RECEIPT_SCHEMA,
    issuedAt: issuedAt.toISOString(),
    validUntil: validUntil.toISOString(),
    status: "published",
    source: {
      cloudRevision: normalized.cloudRevision,
      ossRevision: normalized.sourceRevision,
      treeClean: true,
      revisionLabel: {
        name: SOURCE_REVISION_LABEL,
        value: sourceLabel,
      },
    },
    protocols: {
      localDaemon: normalized.protocols.localDaemonProtocolVersion,
      relayPeer: normalized.protocols.relayPeerProtocolVersion,
    },
    image: {
      reference,
      digest,
      manifests: [{ platform: "linux/amd64", digest }],
      signatureVerification: "verified",
      attestationVerification: "verified",
    },
    publication: {
      status: "succeeded",
      quarantine: "none",
    },
  }
}

function removeUndefined(value) {
  if (Array.isArray(value)) return value.map(removeUndefined)
  if (!isRecord(value)) return value
  return Object.fromEntries(Object.entries(value)
    .filter(([, child]) => child !== undefined)
    .map(([key, child]) => [key, removeUndefined(child)]))
}

function redactSensitive(value, key = "") {
  if (typeof value === "string") {
    if (looksLikeSecretKey(key) && !/path$|environment$/i.test(key)) return "[redacted]"
    if (/-----BEGIN [A-Z ]+-----|\b(?:Bearer\s+|gh[pousr]_\w+|github_pat_\w+|xox[baprs]-\w+)/i.test(value)) {
      return "[redacted]"
    }
    return value
  }
  if (Array.isArray(value)) return value.map((child) => redactSensitive(child, key))
  if (!isRecord(value)) return value
  return Object.fromEntries(Object.entries(value).map(([childKey, child]) => [
    childKey,
    redactSensitive(child, childKey),
  ]))
}

export function redactPublicationReceipt(receipt) {
  return removeUndefined(redactSensitive(receipt))
}

function completeResult(receipt, resumed = false) {
  const digest = receipt.digest ?? receipt.image?.digest
  const reference = receipt.reference ?? receipt.image?.reference
  return {
    mode: "apply",
    resumed,
    digest,
    reference,
    receipt: redactPublicationReceipt(receipt),
  }
}

function commandForInspect(plan, digest) {
  return plan.inspectCommandForDigest(digest)
}

function loadOrCreateJournal(fs, journalPath, plan, identity, normalized) {
  if (fs.existsSync(journalPath)) {
    const journal = readJson(fs, journalPath, "publication journal")
    assertJournalMatches(journal, plan, identity, normalized)
    return { journal, existing: true }
  }
  const journal = newJournal(plan, identity, normalized)
  // This is deliberately the first mutation in apply mode. It records every
  // resource before the run root, builder, or staging tag can be changed.
  writeJsonAtomic(fs, journalPath, journal)
  return { journal, existing: false }
}

function applyGuard(options) {
  if (options.mode !== "apply") return
  const supplied = firstString(options.confirm, options.guard, options.applyGuard)
  if (supplied !== APPLY_GUARD) {
    fail(`apply mode requires --confirm=${APPLY_GUARD}`)
  }
}

export function readPublicationJournal(journalPath, fsOverride = {}) {
  const fs = { ...defaultFileSystem, ...fsOverride }
  const target = requireAbsolutePath(resolve(journalPath), "journal path")
  if (!fs.existsSync(target)) fail("publication journal does not exist", { code: "journal_missing", phase: "journal" })
  return readJson(fs, target, "publication journal")
}

export function recoverInterruptedPublication(journalPath, fsOverride = {}) {
  const journal = readPublicationJournal(journalPath, fsOverride)
  if (!isRecord(journal) || !["interrupted", "running", "planned"].includes(journal.status)) {
    fail("publication journal is not resumable", { code: "journal_not_resumable", phase: "journal" })
  }
  return {
    schema: JOURNAL_SCHEMA,
    status: journal.status,
    nextAction: "rerun the guarded apply with the same reviewed receipt, run root, builder, and staging nonce",
    owned: journal.owned,
    digest: journal.digest,
    reference: journal.reference,
  }
}

export function buildPath1PublicationPlan(options = {}) {
  const mode = options.mode ?? "plan"
  if (mode !== "plan" && mode !== "apply") fail("publication mode must be plan or apply")
  const normalized = normalizeInputs(options, "plan")
  const fs = { ...defaultFileSystem, ...(options.fs ?? {}) }
  const identity = identityFromOptions(options, normalized, fs)
  const plan = buildNormalizedPlan(normalized, identity)
  readPublicationDockerfile(normalized)
  return publicPlan(plan, normalized, identity)
}

export function runPath1Publication(options = {}) {
  const mode = options.mode ?? (options.apply === true ? "apply" : "plan")
  if (mode !== "plan" && mode !== "apply") fail("publication mode must be plan or apply")
  const normalized = normalizeInputs(options, mode)
  const fs = { ...defaultFileSystem, ...(options.fs ?? {}) }
  const identity = identityFromOptions(options, normalized, fs)
  const plan = buildNormalizedPlan(normalized, identity)
  const publicPlanResult = publicPlan(plan, normalized, identity)
  if (mode === "plan") return publicPlanResult
  applyGuard({ ...options, mode })

  const { env, secretValues } = protectedEnvironment(options)
  const run = options.run ?? defaultRunCommand
  const existingJournal = fs.existsSync(identity.journalPath)
    ? readJson(fs, identity.journalPath, "publication journal")
    : null
  if (existingJournal?.status === "completed" && existingJournal.receipt) {
    assertJournalMatches(existingJournal, plan, identity, normalized)
    const receipt = redactPublicationReceipt(existingJournal.receipt)
    const completedDigest = receipt.digest ?? receipt.image?.digest
    const completedReference = receipt.reference ?? receipt.image?.reference
    requireDigest(completedDigest, "completed publication digest")
    if (completedReference !== `${normalized.repository}@${completedDigest}`) {
      fail("completed publication receipt is not bound to its immutable repository digest")
    }
    return completeResult(receipt, true)
  }

  exactCleanSource(normalized, run, env, secretValues)
  readPublicationDockerfile(normalized)
  if (existingJournal && existingJournal.status === "quarantined") {
    fail("quarantined publication evidence cannot be resumed")
  }
  if (existingJournal && existingJournal.status === "quarantine-failed") {
    fail("publication quarantine did not complete; manual evidence recovery is required")
  }
  if (!existingJournal && fs.existsSync(identity.runRoot)) {
    fail("dedicated publication run root already exists without an owned journal")
  }

  const loaded = loadOrCreateJournal(fs, identity.journalPath, plan, identity, normalized)
  const journal = loaded.journal
  assertJournalMatches(journal, plan, identity, normalized)
  journal.status = "running"
  updateJournal(fs, identity.journalPath, journal)

  let builderCreated = journal.stages.builderCreated === true
  let digest = journal.digest
  let reference = journal.reference
  let inspection = null
  let sourceLabel = normalized.sourceRevision
  let interrupted = false
  try {
    if (!journal.stages.runRootCreated) {
      fs.mkdirSync(plan.runRoot, { recursive: true, mode: 0o700 })
      stageJournal(fs, identity.journalPath, journal, "runRootCreated")
    }
    if (!journal.stages.configWritten) {
      fs.writeFileSync(plan.buildKitConfigPath, renderBuildKitConfig(), "utf8")
      stageJournal(fs, identity.journalPath, journal, "configWritten")
    }

    if (!builderCreated) {
      if (journal.stages.builderCreateStarted) {
        try {
          runStep(run, "docker", ["buildx", "inspect", "--name", plan.builderName], {
            cwd: normalized.repoRoot,
            env,
            phase: "builder recovery",
            secretValues,
          })
          builderCreated = true
          stageJournal(fs, identity.journalPath, journal, "builderCreated")
        } catch (recoveryError) {
          if (isInterruption(recoveryError)) throw recoveryError
        }
      }
      if (!builderCreated) {
        stageJournal(fs, identity.journalPath, journal, "builderCreateStarted")
        runStep(run, plan.builderCommand[0], plan.builderCommand.slice(1), {
          cwd: normalized.repoRoot,
          env,
          phase: "pinned BuildKit builder creation",
          secretValues,
        })
        builderCreated = true
        stageJournal(fs, identity.journalPath, journal, "builderCreated")
      }
    }

    let metadata = null
    if (!journal.stages.buildCompleted) {
      const existingMetadata = readOptionalJson(fs, plan.metadataFile, "Buildx metadata")
      if (existingMetadata) {
        metadata = existingMetadata
        digest = descriptorDigestFromBuildxMetadata(metadata)
        journal.digest = digest
        stageJournal(fs, identity.journalPath, journal, "buildCompleted")
      } else {
        stageJournal(fs, identity.journalPath, journal, "buildStarted")
        runStep(run, plan.buildCommand[0], plan.buildCommand.slice(1), {
          cwd: normalized.repoRoot,
          env,
          phase: "source-pinned publication build",
          secretValues,
        })
        metadata = readJson(fs, plan.metadataFile, "Buildx metadata")
        digest = descriptorDigestFromBuildxMetadata(metadata)
        journal.digest = digest
        stageJournal(fs, identity.journalPath, journal, "buildCompleted")
      }
    } else {
      metadata = readJson(fs, plan.metadataFile, "Buildx metadata")
      digest = descriptorDigestFromBuildxMetadata(metadata)
      if (journal.digest && journal.digest !== digest) fail("Buildx descriptor digest changed during resume")
      journal.digest = digest
    }
    requireDigest(digest, "Buildx descriptor digest")
    reference = `${normalized.repository}@${digest}`
    journal.reference = reference
    updateJournal(fs, identity.journalPath, journal)

    if (!journal.stages.inspectionCompleted) {
      const inspectionResult = runStep(run, plan.inspectCommandForDigest(digest)[0], plan.inspectCommandForDigest(digest).slice(1), {
        cwd: normalized.repoRoot,
        env,
        phase: "registry descriptor inspection",
        secretValues,
      })
      inspection = parseJson(inspectionResult.stdout, "Buildx inspection")
      writeJson(fs, join(plan.runRoot, "inspection.json"), inspection)
      validateBuildxPublication({ repository: normalized.repository, digest, inspection })
      sourceLabel = validateSourceLabel(inspection, normalized.sourceRevision)
      journal.evidence.inspection = "inspection.json"
      stageJournal(fs, identity.journalPath, journal, "inspectionCompleted")
    } else {
      inspection = readJson(fs, join(plan.runRoot, "inspection.json"), "Buildx inspection evidence")
      validateBuildxPublication({ repository: normalized.repository, digest, inspection })
      sourceLabel = validateSourceLabel(inspection, normalized.sourceRevision)
    }

    const predicate = createSlsaV02Statement({
      reference,
      digest,
      sourceRevision: normalized.sourceRevision,
    })
    const predicateExisting = readOptionalJson(fs, plan.predicateFile, "SLSA predicate")
    if (predicateExisting && JSON.stringify(predicateExisting) !== JSON.stringify(predicate)) {
      fail("SLSA predicate evidence changed during resume")
    }
    if (!predicateExisting) {
      writeJson(fs, plan.predicateFile, predicate)
      stageJournal(fs, identity.journalPath, journal, "predicateWritten")
    } else if (!journal.stages.predicateWritten) {
      stageJournal(fs, identity.journalPath, journal, "predicateWritten")
    }

    const cosignCommands = buildCosignCommands({ reference, predicateFile: plan.predicateFile })
    if (!journal.stages.signed) {
      runStep(run, cosignCommands.sign[0], cosignCommands.sign.slice(1), {
        cwd: normalized.repoRoot,
        env,
        phase: "key-based Cosign signature",
        secretValues,
      })
      stageJournal(fs, identity.journalPath, journal, "signed")
    }
    if (!journal.stages.attested) {
      runStep(run, cosignCommands.attest[0], cosignCommands.attest.slice(1), {
        cwd: normalized.repoRoot,
        env,
        phase: "key-based SLSA attestation",
        secretValues,
      })
      stageJournal(fs, identity.journalPath, journal, "attested")
    }

    let signatureEvidence = readOptionalJson(fs, join(plan.runRoot, "signature.json"), "Cosign signature evidence")
    if (!signatureEvidence || !journal.stages.signatureVerified) {
      const verified = runStep(run, cosignCommands.verify[0], cosignCommands.verify.slice(1), {
        cwd: normalized.repoRoot,
        env,
        phase: "Cosign signature verification",
        secretValues,
      })
      signatureEvidence = parseJson(verified.stdout, "Cosign signature")
      writeJson(fs, join(plan.runRoot, "signature.json"), signatureEvidence)
      validateCosignSignature({ evidence: signatureEvidence, repository: normalized.repository, digest })
      stageJournal(fs, identity.journalPath, journal, "signatureVerified")
    } else {
      validateCosignSignature({ evidence: signatureEvidence, repository: normalized.repository, digest })
    }

    let attestationEvidence = readOptionalJson(fs, join(plan.runRoot, "attestation.json"), "Cosign attestation evidence")
    if (!attestationEvidence || !journal.stages.attestationVerified) {
      const verified = runStep(run, cosignCommands.verifyAttestation[0], cosignCommands.verifyAttestation.slice(1), {
        cwd: normalized.repoRoot,
        env,
        phase: "SLSA v0.2 attestation verification",
        secretValues,
      })
      attestationEvidence = parseJson(verified.stdout, "Cosign attestation")
      writeJson(fs, join(plan.runRoot, "attestation.json"), attestationEvidence)
      validateSlsaV02Attestation({
        evidence: attestationEvidence,
        repository: normalized.repository,
        digest,
        sourceRevision: normalized.sourceRevision,
      })
      stageJournal(fs, identity.journalPath, journal, "attestationVerified")
    } else {
      validateSlsaV02Attestation({
        evidence: attestationEvidence,
        repository: normalized.repository,
        digest,
        sourceRevision: normalized.sourceRevision,
      })
    }

    const receipt = createReceipt(normalized, digest, reference, sourceLabel, options.now ?? Date.now())
    journal.receipt = receipt
    journal.status = "verified"
    updateJournal(fs, identity.journalPath, journal)

    runStep(run, plan.cleanupBuilderCommand[0], plan.cleanupBuilderCommand.slice(1), {
      cwd: normalized.repoRoot,
      env,
      phase: "dedicated builder cleanup",
      secretValues,
    })
    builderCreated = false
    stageJournal(fs, identity.journalPath, journal, "builderRemoved")
    removeDedicatedRunRoot(plan.runRoot, { remove: fs.rmSync })
    stageJournal(fs, identity.journalPath, journal, "runRootRemoved")
    journal.status = "completed"
    updateJournal(fs, identity.journalPath, journal)
    return completeResult(receipt)
  } catch (error) {
    interrupted = isInterruption(error) || error?.code === "interrupted"
    const executorError = error instanceof Path1PublicationExecutorError
      ? error
      : new Path1PublicationExecutorError(safeErrorMessage(error, secretValues), {
        code: interrupted ? "interrupted" : "publication_failed",
        phase: "publication",
        cause: error,
      })
    if (interrupted) {
      journal.status = "interrupted"
      writeFailureEvidence(fs, plan.runRoot, journal, executorError, secretValues)
      journal.interruption = { reason: safeErrorMessage(executorError), resumeWithSameOwnedResources: true }
      updateJournal(fs, identity.journalPath, journal)
      throw executorError
    }

    writeFailureEvidence(fs, plan.runRoot, journal, executorError, secretValues)
    quarantineAfterFailure(fs, identity.journalPath, journal, executorError, secretValues)
    if (builderCreated) {
      try {
        runStep(run, plan.cleanupBuilderCommand[0], plan.cleanupBuilderCommand.slice(1), {
          cwd: normalized.repoRoot,
          env,
          phase: "dedicated builder failure cleanup",
          secretValues,
        })
        journal.stages.builderRemoved = true
        updateJournal(fs, identity.journalPath, journal)
      } catch (cleanupError) {
        journal.cleanupError = safeErrorMessage(cleanupError, secretValues)
        updateJournal(fs, identity.journalPath, journal)
      }
    }
    throw executorError
  }
}

export const runPublication = runPath1Publication

function parseArguments(argv) {
  const values = {}
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index]
    if (!argument.startsWith("--")) fail(`unexpected argument: ${argument}`)
    const equals = argument.indexOf("=")
    const key = (equals >= 0 ? argument.slice(2, equals) : argument.slice(2)).replaceAll("-", "_")
    const value = equals >= 0 ? argument.slice(equals + 1) : argv[++index]
    if (!value || value.startsWith("--")) fail(`--${key.replaceAll("_", "-")} requires a value`)
    if (values[key] !== undefined) fail(`duplicate option: --${key.replaceAll("_", "-")}`)
    values[key] = value
  }
  return values
}

function readInputFile(target) {
  try {
    return parseJson(readFileSync(resolve(target), "utf8"), "executor input")
  } catch (error) {
    if (error instanceof Path1PublicationExecutorError) throw error
    fail("executor input could not be read", { code: "input_read_failed", phase: "preflight", cause: error })
  }
}

function loadCliOptions(argv) {
  const args = parseArguments(argv)
  const input = args.input ? readInputFile(args.input) : {}
  if (!isRecord(input)) fail("executor input must be a JSON object")
  const options = { ...input }
  if (args.mode) options.mode = args.mode
  if (args.confirm) options.confirm = args.confirm
  if (args.repository) options.repository = args.repository
  if (args.repo_root) options.repoRoot = args.repo_root
  if (args.run_root) options.runRoot = args.run_root
  if (args.builder_name) options.builderName = args.builder_name
  if (args.staging_nonce) options.stagingNonce = args.staging_nonce
  if (args.journal_path) options.journalPath = args.journal_path
  for (const [argument, field] of [
    ["publication_receipt", "reviewedPublicationReceipt"],
    ["host_preflight", "hostPreflightReceipt"],
    ["bootstrap_receipt", "bootstrapReceipt"],
  ]) {
    if (args[argument]) options[field] = readInputFile(args[argument])
  }
  if (args.apply === "true") options.mode = "apply"
  if (args.plan === "true") options.mode = "plan"
  return options
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const options = loadCliOptions(process.argv.slice(2))
    const result = runPath1Publication(options)
    if (result.mode === "apply") {
      // The Cloud promotion tool receives only the immutable reference and
      // the redacted receipt. No command output, paths, or protected values
      // are forwarded.
      process.stdout.write(`${JSON.stringify({ reference: result.reference, receipt: result.receipt })}\n`)
    } else {
      process.stdout.write(`${JSON.stringify(result, null, 2)}\n`)
    }
  } catch (error) {
    process.stderr.write(`publication executor blocked: ${safeErrorMessage(error)}\n`)
    process.exitCode = 1
  }
}
