/** MP-08 / MP-11: reconnect discards any cached authority and asks the kernel. */
export function createKernelSecurityRefresh(deps: {
  readVault(): Promise<Record<string, unknown>>
  readGrants(): Promise<{ grants: readonly unknown[] }>
  notify(message: string): void
}) {
  let epoch = 0
  return {
    invalidate() { epoch++ },
    async refresh() {
      const requestEpoch = ++epoch
      deps.notify("Checking Vault and access status…")
      try {
        const [vault, access] = await Promise.all([deps.readVault(), deps.readGrants()])
        if (requestEpoch !== epoch) return
        if (typeof vault.unlocked !== "boolean" || !Array.isArray(access.grants)) throw new Error("Invalid security status")
        deps.notify(`Vault ${vault.unlocked ? "unlocked" : "locked"} · ${access.grants.length} active access grant${access.grants.length === 1 ? "" : "s"}`)
      } catch {
        if (requestEpoch === epoch) deps.notify("Vault and access status unavailable; reconnect to check")
      }
    },
  }
}
