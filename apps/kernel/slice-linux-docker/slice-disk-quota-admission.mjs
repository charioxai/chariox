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
