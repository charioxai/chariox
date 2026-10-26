import {
  providerAccountAdmissionBlocked,
  providerAccountCapacityResetLabel,
  providerAccountName,
  providerAccountStatusLabel,
  type ProviderAccountProfile,
} from "@chariox/kernel-client"

export { providerAccountAdmissionBlocked }

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

export function normalizeProviderAccountProfileId(
  profiles: readonly ProviderAccountProfile[] | undefined,
  provider: string,
  profileId: string | null | undefined,
): string {
  const accounts = providerAccountsForProvider(profiles, provider)
  return accounts.some((profile) => profile.profile_id === profileId)
    ? profileId!
    : accounts.find((profile) => profile.is_default)?.profile_id
      ?? accounts[0]?.profile_id
      ?? "default"
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
  const status = providerAccountStatusLabel(profile, Date.now(), model)
  return status ? `${providerAccountName(profile, model)} · ${status}` : providerAccountName(profile, model)
}

export function selectedProviderAccountStartBlockReason(
  profiles: readonly ProviderAccountProfile[] | undefined,
  provider: string,
  profileId: string | undefined,
  remoteExecution?: Readonly<Record<string, unknown>> | null,
  model?: string | null,
): string | null {
  // Worker credentials are resolved by the worker kernel, outside this inventory.
  if (remoteExecution && remoteExecution.kind !== "local") return null
  if (profiles === undefined) return null
  const profile = selectedProviderAccount(profiles, provider, profileId)
  // MP-08 / MP-10 / MP-11: replicas and retained home aliases may be outside
  // this inventory. The kernel resolves their namespace and authentication.
  if (!profile && profileId && profileId !== "default") return null
  // MP-08 / MP-10 / MP-11: first-prompt status checking belongs to the kernel.
  if (profile?.auth_state === "unknown") return null
  // A Claude enrollment can be verified or recover through native login at
  // launch. Saved observations alone cannot decide whether it can run.
  if (providerAccountFamily(provider) === "claude"
    && profile?.auth_state === "expired") return null
  if (profile?.auth_state === "authenticated") {
    if (!providerAccountAdmissionBlocked(profile, Date.now(), model)) return null
    const reset = providerAccountCapacityResetLabel(profile, Date.now(), model)
    const alternative = providerAccountsForProvider(profiles, provider).find(candidate => candidate.auth_state === "authenticated"
      && !providerAccountAdmissionBlocked(candidate, Date.now(), model))
    return `${profile.label} has exhausted its allowance${reset ? `; ${reset}` : ""}. Select ${alternative ? `another signed-in account ${alternative.label}` : "another account with quota, or wait for its allowance to reset"}.`
  }
  const label = profile?.label ?? profileId ?? "default"
  const alternative = providerAccountsForProvider(profiles, provider)
    .find((candidate) => candidate.auth_state === "authenticated")
  const requirement = profile?.auth_state === "unknown"
    ? "sign-in not checked. Refresh its status through Provider Accounts"
    : "needs sign-in. Sign in through Provider Accounts"
  return `${label} ${requirement}${alternative ? `, or select signed-in account ${alternative.label}` : ""}.`
}

export function providerAccountSignInCommand(profile: ProviderAccountProfile): string {
  return profile.auth_state === "unknown"
    ? `/provider accounts refresh ${profile.provider} ${profile.profile_id}`
    : `/provider login ${profile.provider} ${profile.profile_id}`
}
