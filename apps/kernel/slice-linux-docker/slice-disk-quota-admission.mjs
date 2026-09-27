import { createHash, randomUUID } from "node:crypto"
import {
  closeSync,
  constants,
  fstatSync,
  fsyncSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  renameSync,
  unlinkSync,
  writeFileSync,
} from "node:fs"
import { dirname, join } from "node:path"
import { requestSliceDiskQuota } from "./slice-disk-quota-client.mjs"
import {
  SLICE_DISK_QUOTA_PROTOCOL_VERSION,
  sliceDiskQuotaIdentityKey,
  validateSliceDiskQuotaIdentity,
  validateSliceDiskQuotaRequest,
} from "./slice-disk-quota-contract.mjs"

const UNBOUNDED_PROOF_SCHEMA_VERSION = 1
const MAX_UNBOUNDED_PROOF_BYTES = 32 * 1024

function exactKeys(value, expected, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} is invalid`)
  }
  const actual = Object.keys(value).sort()
  const wanted = [...expected].sort()
  if (actual.length !== wanted.length || actual.some((key, index) => key !== wanted[index])) {
    throw new Error(`${label} contains unsupported fields`)
  }
}

function validateQuotaEvidenceClass(value, persistent, label) {
  exactKeys(value, ["backendSupportsHardQuota", "isPersistent", "effectiveLimitBytes", "usedBytes"], label)
  if (
    value.backendSupportsHardQuota !== true ||
    value.isPersistent !== persistent ||
    !Number.isSafeInteger(value.effectiveLimitBytes) ||
    value.effectiveLimitBytes <= 0 ||
    !Number.isSafeInteger(value.usedBytes) ||
    value.usedBytes < 0 ||
    value.usedBytes > value.effectiveLimitBytes
  ) {
    throw new Error(`${label} is not verified`)
  }
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
}

function dockerObjectNotFound(stderr, resourceKind, resourceName) {
  const lines = String(stderr ?? "").split(/\r?\n/).map((line) => line.trim()).filter(Boolean)
  if (lines.length !== 1) return false
  const name = escapeRegExp(resourceName)
  if (resourceKind === "container") {
    return new RegExp(`^(?:Error: )?No such (?:object|container): ${name}$`, "i").test(lines[0])
      || new RegExp(`^Error response from daemon: No such container: ${name}$`, "i").test(lines[0])
  }
  return new RegExp(`^(?:Error response from daemon: )?get ${name}: no such volume$`, "i").test(lines[0])
    || new RegExp(`^(?:Error: )?No such volume: ${name}$`, "i").test(lines[0])
    || new RegExp(`^volume ${name} (?:not found|does not exist)$`, "i").test(lines[0])
}

export function readSliceDiskQuotaMarkerInspection(result, resourceKind, resourceName) {
  if (!result || typeof result !== "object" || Array.isArray(result)
    || !["container", "volume"].includes(resourceKind)
    || typeof resourceName !== "string" || resourceName.length === 0) {
    throw new Error("managed disk quota marker inspection is invalid")
  }
  if (result.error || result.signal || result.status === null) {
    throw new Error("managed disk quota marker inspection could not be completed")
  }
  if (result.status !== 0) {
    if (result.status === 1 && dockerObjectNotFound(result.stderr, resourceKind, resourceName)) return false
    throw new Error("managed disk quota marker inspection failed")
  }

  let labels
  try {
    labels = JSON.parse(result.stdout)
  } catch {
    throw new Error("managed disk quota labels are malformed")
  }
  if (labels === null) return false
  if (typeof labels !== "object" || Array.isArray(labels)) {
    throw new Error("managed disk quota labels are unreadable")
  }
  const marker = labels["io.chariox.slice.disk-quota"]
  if (marker === undefined) return false
  if (marker !== "xfs-project-v1") throw new Error("managed disk quota marker is unsupported")
  return true
}

function validateEnsureBeforeStartResult(result) {
  if (!result || typeof result !== "object" || Array.isArray(result)) {
    throw new Error("managed disk quota allocator start result is invalid")
  }
  if (result.bounded === false) {
    exactKeys(result, ["bounded"], "managed disk quota allocator start result")
    return false
  }
  if (result.bounded !== true) {
    throw new Error("managed disk quota allocator start result is invalid")
  }
  exactKeys(result, ["bounded", "evidence"], "managed disk quota allocator start result")
  exactKeys(result.evidence, ["writableLayer", "persistentHome"], "managed disk quota evidence")
  validateQuotaEvidenceClass(result.evidence.writableLayer, false, "writable-layer quota evidence")
  validateQuotaEvidenceClass(result.evidence.persistentHome, true, "persistent-home quota evidence")
  return true
}

function validateUnboundedBinding(binding) {
  exactKeys(binding, [
    "containerId",
    "homeVolumeCreatedAt",
    "homeVolumeDevice",
    "homeVolumeDriver",
    "homeVolumeInode",
    "homeVolumeMountpoint",
    "homeVolumeName",
    "homeVolumeScope",
  ], "managed unbounded quota binding")
  if (
    typeof binding.containerId !== "string" || !/^[a-f0-9]{64}$/.test(binding.containerId) ||
    typeof binding.homeVolumeCreatedAt !== "string" || binding.homeVolumeCreatedAt.length > 128 || binding.homeVolumeCreatedAt.includes("\0") ||
    typeof binding.homeVolumeDevice !== "string" || !/^[0-9]+$/.test(binding.homeVolumeDevice) ||
    typeof binding.homeVolumeDriver !== "string" || binding.homeVolumeDriver.length === 0 || binding.homeVolumeDriver.length > 256 || binding.homeVolumeDriver.includes("\0") ||
    typeof binding.homeVolumeInode !== "string" || !/^[0-9]+$/.test(binding.homeVolumeInode) ||
    typeof binding.homeVolumeMountpoint !== "string" || !binding.homeVolumeMountpoint.startsWith("/") ||
    binding.homeVolumeMountpoint.includes("\0") || binding.homeVolumeMountpoint.length > 4096 ||
    typeof binding.homeVolumeName !== "string" ||
    typeof binding.homeVolumeScope !== "string" || binding.homeVolumeScope.length === 0 || binding.homeVolumeScope.length > 128 || binding.homeVolumeScope.includes("\0")
  ) {
    throw new Error("managed unbounded quota binding is invalid")
  }
  return {
    containerId: binding.containerId,
    homeVolumeCreatedAt: binding.homeVolumeCreatedAt,
    homeVolumeDevice: binding.homeVolumeDevice,
    homeVolumeDriver: binding.homeVolumeDriver,
    homeVolumeInode: binding.homeVolumeInode,
    homeVolumeMountpoint: binding.homeVolumeMountpoint,
    homeVolumeName: binding.homeVolumeName,
    homeVolumeScope: binding.homeVolumeScope,
  }
}

function unboundedProofPath(root, identity) {
  if (typeof root !== "string" || root.length === 0 || !root.startsWith("/")) {
    throw new TypeError("managed unbounded quota proof root is invalid")
  }
  validateSliceDiskQuotaIdentity(identity)
  const key = createHash("sha256").update(identity.containerName).digest("hex")
  return join(root, `${key}.json`)
}

function unboundedProofRootMetadata(root) {
  let metadata
  try {
    metadata = lstatSync(root)
  } catch (error) {
    if (error?.code === "ENOENT") return undefined
    throw error
  }
  if (
    metadata.isSymbolicLink() || !metadata.isDirectory() ||
    (typeof process.getuid === "function" && metadata.uid !== process.getuid()) ||
    (metadata.mode & 0o077) !== 0
  ) {
    throw new Error("managed unbounded quota proof root is unreadable")
  }
  return metadata
}

function ensureUnboundedProofRoot(root) {
  mkdirSync(root, { recursive: true, mode: 0o700 })
  if (!unboundedProofRootMetadata(root)) throw new Error("managed unbounded quota proof root is invalid")
}

function readUnboundedProofFile(path, identity) {
  let fd
  try {
    fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW)
  } catch (error) {
    if (error?.code === "ENOENT") return undefined
    throw error
  }
  try {
    const metadata = fstatSync(fd)
    if (
      !metadata.isFile() ||
      metadata.nlink !== 1 ||
      (typeof process.getuid === "function" && metadata.uid !== process.getuid()) ||
      (metadata.mode & 0o077) !== 0 ||
      metadata.size <= 0 || metadata.size > MAX_UNBOUNDED_PROOF_BYTES
    ) {
      throw new Error("managed unbounded quota proof file is unreadable")
    }
    let proof
    try {
      proof = JSON.parse(readFileSync(fd, "utf8"))
    } catch {
      throw new Error("managed unbounded quota proof is malformed")
    }
    exactKeys(proof, ["schemaVersion", "identity", "binding"], "managed unbounded quota proof")
    if (proof.schemaVersion !== UNBOUNDED_PROOF_SCHEMA_VERSION) {
      throw new Error("managed unbounded quota proof version is unsupported")
    }
    if (sliceDiskQuotaIdentityKey(proof.identity) !== sliceDiskQuotaIdentityKey(identity)) {
      throw new Error("managed unbounded quota proof identity is stale")
    }
    return {
      schemaVersion: proof.schemaVersion,
      identity: proof.identity,
      binding: validateUnboundedBinding(proof.binding),
    }
  } finally {
    closeSync(fd)
  }
}

function syncUnboundedProofDirectory(path) {
  const fd = openSync(dirname(path), constants.O_RDONLY | constants.O_DIRECTORY)
  try {
    fsyncSync(fd)
  } finally {
    closeSync(fd)
  }
}

export function writeSliceDiskQuotaUnboundedProof(root, identity, binding) {
  const normalizedBinding = validateUnboundedBinding(binding)
  if (normalizedBinding.homeVolumeName !== identity.homeVolumeName) {
    throw new Error("managed unbounded quota proof does not match its home volume")
  }
  const path = unboundedProofPath(root, identity)
  ensureUnboundedProofRoot(root)
  // Do not overwrite an unreadable or malformed prior proof as if it were absent.
  readUnboundedProofFile(path, identity)
  const payload = Buffer.from(JSON.stringify({
    schemaVersion: UNBOUNDED_PROOF_SCHEMA_VERSION,
    identity,
    binding: normalizedBinding,
  }))
  if (payload.length > MAX_UNBOUNDED_PROOF_BYTES) {
    throw new Error("managed unbounded quota proof is too large")
  }
  const temporary = join(root, `.new-${process.pid}-${randomUUID()}`)
  const fd = openSync(temporary, constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY | constants.O_NOFOLLOW, 0o600)
  try {
    try {
      writeFileSync(fd, payload)
      fsyncSync(fd)
    } finally {
      closeSync(fd)
    }
    renameSync(temporary, path)
    syncUnboundedProofDirectory(path)
  } catch (error) {
    try {
      unlinkSync(temporary)
    } catch (cleanupError) {
      if (cleanupError?.code !== "ENOENT") throw cleanupError
    }
    throw error
  }
}

export function hasMatchingSliceDiskQuotaUnboundedProof(root, identity, binding) {
  const rootMetadata = unboundedProofRootMetadata(root)
  if (!rootMetadata) return false
  const normalizedBinding = validateUnboundedBinding(binding)
  if (normalizedBinding.homeVolumeName !== identity.homeVolumeName) return false
  const path = unboundedProofPath(root, identity)
  const proof = readUnboundedProofFile(path, identity)
  if (!proof) return false
  return JSON.stringify(proof.binding) === JSON.stringify(normalizedBinding)
}

export function removeSliceDiskQuotaUnboundedProof(root, identity) {
  const path = unboundedProofPath(root, identity)
  if (!unboundedProofRootMetadata(root)) return
  try {
    const metadata = lstatSync(path)
    if (metadata.isDirectory()) throw new Error("managed unbounded quota proof path is obstructed")
    unlinkSync(path)
    syncUnboundedProofDirectory(path)
  } catch (error) {
    if (error?.code !== "ENOENT") throw error
  }
}

export async function runWithSliceDiskQuotaAdmission({
  containerName,
  quotaMarkerPresent,
  resolveUnboundedProof,
  requestQuota = requestSliceDiskQuota,
  run,
}) {
  if (typeof quotaMarkerPresent !== "boolean") {
    throw new TypeError("managed disk quota marker status must be boolean")
  }
  if (typeof requestQuota !== "function" || typeof run !== "function") {
    throw new TypeError("managed disk quota admission requires request and run functions")
  }
  const request = {
    protocolVersion: SLICE_DISK_QUOTA_PROTOCOL_VERSION,
    operation: "ensure_before_start",
    containerName,
  }
  validateSliceDiskQuotaRequest(request)
  let result
  let proof
  try {
    result = await requestQuota(request)
  } catch (error) {
    if (
      quotaMarkerPresent ||
      !new Set(["ENOENT", "ECONNREFUSED"]).has(error?.code) ||
      typeof resolveUnboundedProof !== "function"
    ) {
      throw error
    }
    proof = await resolveUnboundedProof()
    if (proof === undefined || proof === null) throw error
    exactKeys(proof, ["containerId"], "broker-owned unbounded quota proof")
    if (typeof proof.containerId !== "string" || !/^[a-f0-9]{64}$/.test(proof.containerId)) {
      throw new Error("broker-owned unbounded quota proof is invalid")
    }
    result = { bounded: false }
  }
  if (!validateEnsureBeforeStartResult(result) && quotaMarkerPresent) {
    throw new Error("container has a disk quota marker but no matching durable reservation")
  }
  return run(result, { source: proof ? "broker-proof" : "allocator", containerId: proof?.containerId })
}
