import { createHash } from "node:crypto"

const receiptKeys = ["schemaVersion", "status", "allocationId", "machineId", "kernelId", "relayPublicKey", "runtimeReleaseDigest", "homeCaller", "confirmedAt"]
const homeKeys = ["accountId", "kernelId", "machineId", "realmId", "relayPublicKey", "userId"]
const exactKeys = (value, keys) => value && typeof value === "object" && !Array.isArray(value)
  && Object.keys(value).length === keys.length && keys.every((key) => Object.hasOwn(value, key))
const identifier = (value) => typeof value === "string" && /^[a-z0-9][a-z0-9._:-]{0,127}$/.test(value)
const digest = (value) => typeof value === "string" && /^sha256:[a-f0-9]{64}$/.test(value)
const nonempty = (value) => typeof value === "string" && value.trim().length > 0

export function allocationWorkerBindingDigest(receipt) {
  if (!exactKeys(receipt, receiptKeys) || receipt.schemaVersion !== 1
    || receipt.status !== "confirmed" || typeof receipt.confirmedAt !== "string"
    || !Number.isFinite(Date.parse(receipt.confirmedAt))
    || ![receipt.allocationId, receipt.machineId, receipt.kernelId].every(identifier)
    || !nonempty(receipt.relayPublicKey) || !digest(receipt.runtimeReleaseDigest)
    || !exactKeys(receipt.homeCaller, homeKeys)
    || !homeKeys.filter((key) => key !== "relayPublicKey").every((key) => identifier(receipt.homeCaller[key]))
    || !nonempty(receipt.homeCaller.relayPublicKey)) {
    throw new Error("allocation worker bootstrap receipt is invalid")
  }
  // Alphabetical keys match serde_json's ordered map in worker.rs. Exclude only
  // confirmation bookkeeping; pin the original release and every authority ID.
  const binding = {
    allocationId: receipt.allocationId,
    homeCaller: Object.fromEntries(homeKeys.map((key) => [key, receipt.homeCaller[key]])),
    kernelId: receipt.kernelId,
    machineId: receipt.machineId,
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
