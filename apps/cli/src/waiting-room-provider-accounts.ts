import {
  providerAccountCapacity,
  providerAccountCapacityLabel,
  type ProviderAccountProfile,
} from "@chariox/kernel-client"

export { providerAccountCapacity }

export function providerAccountFamily(provider: string): string {
  return provider === "claude-headless" || provider === "claude-p" ? "claude" : provider
}

export function providerAccountsForProvider(
  profiles: readonly ProviderAccountProfile[] | undefined,
  provider: string,
): readonly ProviderAccountProfile[] {
  const family = providerAccountFamily(provider)
  return (profiles ?? []).filter((profile) => profile.provider === family)
}

export function selectedProviderAccount(
  profiles: readonly ProviderAccountProfile[] | undefined,
  provider: string,
  profileId: string | undefined,
): ProviderAccountProfile | null {
  const accounts = providerAccountsForProvider(profiles, provider)
  if (!profileId || profileId === "default") {
    return accounts.find((profile) => profile.is_default) ?? null
  }
  return accounts.find((profile) => profile.profile_id === profileId) ?? null
}

export type ProviderAccountSelection =
  | { readonly kind: "resolved"; readonly profile: ProviderAccountProfile }
  | { readonly kind: "ambiguous"; readonly aliases: readonly string[] }
  | { readonly kind: "missing" }

export function resolveProviderAccountSelection(
  profiles: readonly ProviderAccountProfile[] | undefined,
  provider: string,
  reference: string,
): ProviderAccountSelection {
  const accounts = providerAccountsForProvider(profiles, provider)
  const normalized = reference.trim()
  const profile = accounts.find((candidate) => candidate.profile_id === normalized)
  if (profile) return { kind: "resolved", profile }
  const exactAlias = accounts.find((candidate) => candidate.label === normalized)
  if (exactAlias) return { kind: "resolved", profile: exactAlias }
  const foldedAliases = accounts.filter(
    (candidate) => candidate.label.localeCompare(normalized, "en", { sensitivity: "accent" }) === 0,
  )
  if (foldedAliases.length === 1) {
    return { kind: "resolved", profile: foldedAliases[0]! }
  }
  if (foldedAliases.length > 1) {
    return { kind: "ambiguous", aliases: foldedAliases.map((candidate) => candidate.label) }
  }
  return { kind: "missing" }
}

export function defaultProviderAccountProfileId(
  profiles: readonly ProviderAccountProfile[] | undefined,
  provider: string,
): string {
  const accounts = providerAccountsForProvider(profiles, provider)
  return accounts.find((profile) => profile.is_default)?.profile_id
    ?? "default"
}

export function providerAccountDisplayLabel(profile: ProviderAccountProfile, model?: string | null): string {
  return providerAccountCapacityLabel(profile, Date.now(), model)
}

export function providerAccountCopiesSummary(profile: ProviderAccountProfile): string {
  return (profile.materializations ?? []).map((status) => {
    const copy = status.copy
    if (!copy) return `Copy on ${status.target_kind} ${status.target_ref}: ${status.state} (machine identity not recorded)`
    const warning = copy.renewable_services.length > 0
      ? " This copy can log you out. Consider logging in on that machine."
      : ""
    const login = status.target_kind === "slice"
      ? `/slice auth login ${status.target_ref} ${profile.provider} ${copy.target_account_id}`
      : `/provider login ${profile.provider} ${copy.target_account_id}`
    return `Copy on ${copy.target_machine_id} (${copy.target_kernel_id}), account ${copy.target_account_id}; copied from ${copy.source_machine_id} (${copy.source_account_id}); ${copy.auth_state.replaceAll("_", " ")} observed ${new Date(status.observed_at_ms).toISOString()}.${warning} Official login on receiving machine: ${login}`
  }).join("\n")
}
