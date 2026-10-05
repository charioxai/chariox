import { createHash } from "node:crypto"

export const SYNTHETIC_LEAK_SURFACES = ["arguments", "logs", "evidence", "prompts", "fixtures", "observations"]
const digest = (text) => createHash("sha256").update(text).digest("hex")

// This only measures supplied synthetic evidence. Real Vault/input execution
// must provide separate product receipts before a harness can accept it.
export function scanSyntheticCredentialSurfaces({ marker, runId, surfaces }) {
  if (typeof marker !== "string" || !marker || typeof runId !== "string" || !runId) throw new Error("synthetic leak scan requires a marker and run identity")
  const receipts = {}
  for (const name of SYNTHETIC_LEAK_SURFACES) {
    const items = surfaces?.[name]
    if (!Array.isArray(items) || !items.every((item) => typeof item === "string")) throw new Error(`synthetic leak scan did not measure ${name}`)
    const text = items.join("\n")
    receipts[name] = { status: "measured", runId, itemCount: items.length, byteCount: Buffer.byteLength(text), matches: text.split(marker).length - 1, sha256: digest(text) }
  }
  return { status: "measured", runId, markerSha256: digest(marker), receipts }
}

export function syntheticVaultEvidenceFailure(result, runId) {
  if (result?.vaultValidation?.status !== "measured" || result.vaultValidation.runId !== runId
    || result.vaultValidation.storagePath !== "kernel-vault" || result.vaultValidation.inputPath !== "credential-backed-target-input") return "synthetic_vault_evidence_not_measured"
  const scan = result.leakValidation
  if (scan?.status !== "measured" || scan.runId !== runId || !/^[a-f0-9]{64}$/.test(scan.markerSha256 ?? "")) return "synthetic_vault_evidence_not_measured"
  for (const name of SYNTHETIC_LEAK_SURFACES) {
    const receipt = scan.receipts?.[name]
    if (receipt?.status !== "measured" || receipt.runId !== runId || !Number.isSafeInteger(receipt.itemCount) || receipt.itemCount < 0
      || !Number.isSafeInteger(receipt.byteCount) || receipt.byteCount < 0 || !Number.isSafeInteger(receipt.matches) || receipt.matches < 0 || !/^[a-f0-9]{64}$/.test(receipt.sha256 ?? "")) return "synthetic_vault_evidence_not_measured"
    if (receipt.matches !== 0) return "synthetic_vault_leak_detected"
  }
  return null
}
