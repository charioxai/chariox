// Row 554: when does a drill kernel's Vault lock, with and without a hot reload? The kernel's Vault is unlocked "until the
// kernel stops" and its status is sampled every minute. Run: node scripts/drills/vault-lock-probe.mjs --kernel-binary <path>
//   --state-root <dir> --evidence-dir <dir> [--reload-after-s 180] [--minutes 45]
import { randomBytes } from "node:crypto"
import { drillFlags, startDrillKernel } from "./own-kernel.mjs"
import { setTimeout as delay } from "node:timers/promises"

const { flags, need } = drillFlags(process.argv)
const reloadAfter = Number(flags.get("--reload-after-s") ?? 0)
const minutes = Number(flags.get("--minutes") ?? 45)
const k = await startDrillKernel({ label: "nw554vault", binary: need("--kernel-binary"), stateRoot: need("--state-root"), evidence: need("--evidence-dir") })
try {
  const passphrase = randomBytes(32).toString("hex")
  await k.rpc({ CreateCredentialVault: { origin: { kind: "passphrase", passphrase } } })
  await k.rpc({ UnlockCredentialVault: { passphrase, duration: { kind: "until_kernel_stops" }, surface: "terminal" } })
  const t0 = Date.now()
  let reloaded = false
  for (let i = 0; i < minutes; i++) {
    if (reloadAfter && !reloaded && Date.now() - t0 >= reloadAfter * 1000) {
      process.kill(k.pid, "SIGUSR2")
      reloaded = true
      k.note("hot-reload-requested", { pid: k.pid })
      await delay(20000)
    }
    let status
    try { status = JSON.stringify((await k.rpc({ GetCredentialVaultStatus: null }))).slice(0, 300) } catch (error) { status = `error ${String(error.message ?? error).slice(0, 150)}` }
    k.note("vault-status", { minute: Math.round((Date.now() - t0) / 60000), status })
    await delay(60000)
  }
} finally {
  await k.stop()
}
