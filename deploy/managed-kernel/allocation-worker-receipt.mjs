import { createHash } from "node:crypto"
import { lstatSync, realpathSync } from "node:fs"
import { join, posix } from "node:path"

const receiptKeys = ["schemaVersion", "status", "allocationId", "machineId", "kernelId", "relayPublicKey", "runtimeReleaseDigest", "homeCaller", "confirmedAt"]
const schemaTwoReceiptKeys = [...receiptKeys, "managedRepositoryRoot"]
export const DEFAULT_MANAGED_REPOSITORY_ROOT = "/home/chariox"
const protectedRoots = ["/", "/var/lib/chariox", "/usr/lib/chariox/releases"]
const homeKeys = ["accountId", "kernelId", "machineId", "realmId", "relayPublicKey", "userId"]
const exactKeys = (value, keys) => value && typeof value === "object" && !Array.isArray(value)
  && Object.keys(value).length === keys.length && keys.every((key) => Object.hasOwn(value, key))
const identifier = (value) => typeof value === "string" && /^[a-z0-9][a-z0-9._:-]{0,127}$/.test(value)
const digest = (value) => typeof value === "string" && /^sha256:[a-f0-9]{64}$/.test(value)
const nonempty = (value) => typeof value === "string" && value.trim().length > 0

export function normalizeManagedRepositoryRoot(value) {
  if (value === undefined) return DEFAULT_MANAGED_REPOSITORY_ROOT
  if (typeof value !== "string" || value.length === 0 || value.length > 4096
    || value !== value.trim() || value.startsWith("//") || !value.startsWith("/")
    || /[\u0000-\u001f\u007f]/.test(value)) throw new Error("managed repository root is invalid")
  const segments = value.split("/")
  if (segments.some((segment) => segment === "." || segment === "..")) {
    throw new Error("managed repository root is invalid")
  }
  const normalized = posix.normalize(value).replace(/\/+$/, "") || "/"
  const overlaps = protectedRoots.some((root) => root === normalized
    || (root !== "/" && (normalized.startsWith(`${root}/`) || root.startsWith(`${normalized}/`))))
  if (overlaps || normalized === "/") throw new Error("managed repository root is protected")
  let current = "/"
  for (const segment of normalized.split("/").filter(Boolean)) {
    current = join(current, segment)
    let metadata
    try {
      metadata = lstatSync(current)
    } catch (error) {
      if (error?.code === "ENOENT") break
      throw new Error("managed repository root cannot be inspected")
    }
    if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
      throw new Error("managed repository root is not a real directory")
    }
  }
  try {
    if (realpathSync.native(normalized) !== normalized) {
      throw new Error("managed repository root resolves through a symlink")
    }
  } catch (error) {
    if (error?.code !== "ENOENT") throw error
  }
  return normalized
}

export function allocationWorkerBindingDigest(receipt) {
  const keys = receipt?.schemaVersion === 2 || Object.hasOwn(receipt ?? {}, "managedRepositoryRoot")
    ? schemaTwoReceiptKeys : receiptKeys
  if (!exactKeys(receipt, keys) || ![1, 2].includes(receipt.schemaVersion)
    || receipt.status !== "confirmed" || typeof receipt.confirmedAt !== "string"
    || !Number.isFinite(Date.parse(receipt.confirmedAt))
    || ![receipt.allocationId, receipt.machineId, receipt.kernelId].every(identifier)
    || !nonempty(receipt.relayPublicKey) || !digest(receipt.runtimeReleaseDigest)
    || !exactKeys(receipt.homeCaller, homeKeys)
    || !homeKeys.filter((key) => key !== "relayPublicKey").every((key) => identifier(receipt.homeCaller[key]))
    || !nonempty(receipt.homeCaller.relayPublicKey)) {
    throw new Error("allocation worker bootstrap receipt is invalid")
  }
  const normalizedRoot = normalizeManagedRepositoryRoot(receipt.managedRepositoryRoot)
  if (receipt.schemaVersion === 2 && receipt.managedRepositoryRoot !== normalizedRoot) {
    throw new Error("allocation worker bootstrap receipt is invalid")
  }
  if (receipt.schemaVersion === 1 && Object.hasOwn(receipt, "managedRepositoryRoot")
    && receipt.managedRepositoryRoot !== DEFAULT_MANAGED_REPOSITORY_ROOT) {
    throw new Error("allocation worker bootstrap receipt is invalid")
  }
  // Alphabetical keys match serde_json's ordered map in worker.rs. Exclude only
  // confirmation bookkeeping; pin the original release and every authority ID.
  const binding = {
    allocationId: receipt.allocationId,
    homeCaller: Object.fromEntries(homeKeys.map((key) => [key, receipt.homeCaller[key]])),
    kernelId: receipt.kernelId,
    machineId: receipt.machineId,
    ...(receipt.schemaVersion === 2 ? { managedRepositoryRoot: normalizedRoot } : {}),
    relayPublicKey: receipt.relayPublicKey,
    runtimeReleaseDigest: receipt.runtimeReleaseDigest,
    schemaVersion: receipt.schemaVersion,
  }
  return `sha256:${createHash("sha256").update(JSON.stringify(binding)).digest("hex")}`
}

export function allocationWorkerReleaseMatches(receipt, override, expectedDigest) {
  const bindingDigest = allocationWorkerBindingDigest(receipt)
  if (override && (!exactKeys(override, ["schemaVersion", "kind", "bindingDigest", "runtimeReleaseDigest"])
    || override.schemaVersion !== 1 || override.kind !== "disposable_worker_release"
    || override.bindingDigest !== bindingDigest || !digest(override.runtimeReleaseDigest))) {
    throw new Error("allocation worker release override is invalid")
  }
  return (override?.runtimeReleaseDigest ?? receipt.runtimeReleaseDigest) === expectedDigest
}
