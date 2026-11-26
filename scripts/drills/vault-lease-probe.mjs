// Row 554 harness check: which lease does a drill Vault get? (A) create, then unlock until the kernel stops;
// (B) lock, then unlock until the kernel stops. The status is read after each step.
import { randomBytes } from "node:crypto"
import { drillFlags, startDrillKernel } from "./own-kernel.mjs"

const { need } = drillFlags(process.argv)
const k = await startDrillKernel({ label: "nw554lease", binary: need("--kernel-binary"), stateRoot: need("--state-root"), evidence: need("--evidence-dir") })
const status = async step => {
  const s = (await k.rpc({ GetCredentialVaultStatus: null })).CredentialVaultStatus.status
  k.note(step, { unlocked: s.unlocked, expires_at_ms: s.expires_at_ms ?? null, in_minutes: s.expires_at_ms ? Math.round((s.expires_at_ms - Date.now()) / 60000) : null })
}
try {
  const passphrase = randomBytes(32).toString("hex")
  const created = await k.rpc({ CreateCredentialVault: { origin: { kind: "passphrase", passphrase } } })
  k.note("A0-created", { reply: JSON.stringify(created).slice(0, 200) })
  await status("A1-after-create")
  const unlocked = await k.rpc({ UnlockCredentialVault: { passphrase, duration: { kind: "until_kernel_stops" }, surface: "terminal" } })
  k.note("A2-unlock-reply", { reply: JSON.stringify(unlocked).slice(0, 300) })
  await status("A3-after-unlock-until-kernel-stops")
  await k.rpc({ LockCredentialVault: null })
  await status("B0-after-lock")
  const again = await k.rpc({ UnlockCredentialVault: { passphrase, duration: { kind: "until_kernel_stops" }, surface: "terminal" } })
  k.note("B1-unlock-reply", { reply: JSON.stringify(again).slice(0, 300) })
  await status("B2-after-unlock-until-kernel-stops")
} finally {
  await k.stop()
}
