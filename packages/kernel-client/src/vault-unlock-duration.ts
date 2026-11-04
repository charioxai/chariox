// How long an unlocked Vault stays open: the owner chooses on every unlock, the kernel preselects one.
export type VaultUnlockDuration =
  | { readonly kind: "once" }
  | { readonly kind: "minutes"; readonly minutes: number }
  | { readonly kind: "until_kernel_stops" }

export const vaultUnlockDurationUsage = "Use once, kernel, or a length such as 30m or 2h."

/** `once`, `kernel`, `45` or `45m` (minutes), `2h` (hours); null for anything else. */
export function parseVaultUnlockDurationArgument(token: string): VaultUnlockDuration | null {
  const value = token.trim().toLowerCase()
  if (value === "once") return { kind: "once" }
  if (value === "kernel") return { kind: "until_kernel_stops" }
  const match = /^(\d{1,5})([mh]?)$/.exec(value)
  if (!match) return null
  const minutes = Number(match[1]) * (match[2] === "h" ? 60 : 1)
  return minutes > 0 ? { kind: "minutes", minutes } : null
}

/** The duration the kernel preselects, from its `CredentialVaultStatus` answer. */
export function vaultUnlockDefaultFromStatus(response: unknown): VaultUnlockDuration {
  const status = (response as { CredentialVaultStatus?: { unlock?: { default?: unknown } } } | null)?.CredentialVaultStatus
  const preselected = status?.unlock?.default
  const parsed = typeof preselected === "object" && preselected !== null ? parseWire(preselected as Record<string, unknown>) : null
  if (!parsed) throw new Error("The kernel did not say how long the Vault stays unlocked.")
  return parsed
}

function parseWire(value: Record<string, unknown>): VaultUnlockDuration | null {
  if (value.kind === "once") return { kind: "once" }
  if (value.kind === "until_kernel_stops") return { kind: "until_kernel_stops" }
  return value.kind === "minutes" && typeof value.minutes === "number" && Number.isInteger(value.minutes) && value.minutes > 0 ? { kind: "minutes", minutes: value.minutes } : null
}
