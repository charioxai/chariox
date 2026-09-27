import { requestSliceDiskQuota } from "./slice-disk-quota-client.mjs"
import {
  SLICE_DISK_QUOTA_PROTOCOL_VERSION,
  validateSliceDiskQuotaRequest,
} from "./slice-disk-quota-contract.mjs"

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

export async function runWithSliceDiskQuotaAdmission({
  containerName,
  quotaMarkerPresent,
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
  const result = await requestQuota(request)
  if (!validateEnsureBeforeStartResult(result) && quotaMarkerPresent) {
    throw new Error("container has a disk quota marker but no matching durable reservation")
  }
  return run(result)
}
