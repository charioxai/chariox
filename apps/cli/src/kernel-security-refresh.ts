/** MP-08 / MP-11: reconnect discards any cached authority and asks the kernel. */
export function createKernelSecurityRefresh(deps: {
  readVault(): Promise<Record<string, unknown>>
  readGrants(): Promise<{ grants: readonly unknown[] }>
  notify(message: string): void
}) {
  let epoch = 0
  let connected = true
  // A reconnect asks from the event and from connectivity; show the check once.
  let checking = false
  const refresh = async () => {
    const requestEpoch = ++epoch
    if (!checking) {
      checking = true
      deps.notify("Checking Vault and access status…")
    }
    try {
      const [vault, access] = await Promise.all([deps.readVault(), deps.readGrants()])
      if (requestEpoch !== epoch) return
      checking = false
      if (typeof vault.unlocked !== "boolean" || !Array.isArray(access.grants)) throw new Error("Invalid security status")
      deps.notify(`Vault ${vault.unlocked ? "unlocked" : "locked"} · ${access.grants.length} active access grant${access.grants.length === 1 ? "" : "s"}`)
    } catch {
      if (requestEpoch !== epoch) return
      checking = false
      deps.notify("Vault and access status unavailable; reconnect to check")
    }
  }
  return {
    invalidate() { epoch++ },
    /** Returns whether the connection really changed. */
    connectionChanged(next: boolean): boolean {
      if (connected === next) return false
      connected = next
      if (next) void refresh()
      else {
        epoch++
        checking = false
      }
      return true
    },
    refresh,
  }
}
