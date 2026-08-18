export type SliceAuthProviderSeparator = ":" | " " | "="

export type SliceProviderAuthLike = {
  readonly provider: string
  readonly account_profile?: string | null
  readonly alias?: string | null
  readonly state: string
  readonly auth_type?: string | null
  readonly account_id?: string | null
  readonly email?: string | null
  readonly organization_id?: string | null
  readonly organization_name?: string | null
  readonly subscription_type?: string | null
}

export type SliceRecordLike = {
  readonly workspace_id?: string | null
  readonly worktree_id?: string | null
  readonly workspace_mount?: string | null
  readonly providers?: readonly string[] | null
  readonly provider_auth?: readonly SliceProviderAuthLike[] | null
  readonly relay_endpoint?: {
    readonly url?: string | null
    readonly private?: boolean | null
  } | null
  readonly last_operation?: string | null
  readonly last_operation_status?: string | null
  readonly last_error?: string | null
}

export type SliceProviderAuthCoverage = {
  readonly providers: readonly string[]
  readonly authProviders: readonly string[]
  readonly missingProviders: readonly string[]
  readonly staleProviders: readonly string[]
  readonly hasHealthyCoverage: boolean
  readonly needsAttention: boolean
}

export function formatSliceScope(slice: SliceRecordLike): string {
  return slice.worktree_id || slice.workspace_mount || slice.workspace_id || "-"
}

export function formatSliceRelayLabel(
  slice: SliceRecordLike,
  options: { readonly includeUrl?: boolean; readonly emptyLabel?: string } = {},
): string {
  const endpoint = slice.relay_endpoint
  if (!endpoint?.url) {
    return options.emptyLabel ?? ""
  }
  const label = endpoint.private === true
    ? "private"
    : endpoint.private === false
      ? "shared"
      : "unknown"
  return options.includeUrl ? `${label}:${endpoint.url}` : label
}

export function formatSliceOperation(slice: SliceRecordLike): string {
  if (!slice.last_operation && !slice.last_operation_status && !slice.last_error) {
    return ""
  }
  const status = slice.last_operation_status ? `:${slice.last_operation_status}` : ""
  const error = slice.last_error ? ` error=${slice.last_error}` : ""
  return `${slice.last_operation ?? "operation"}${status}${error}`
}

export function formatSliceDiagnostics(slice: SliceRecordLike): string {
  const operation = formatSliceOperation(slice)
  return operation ? ` last_operation=${operation}` : ""
}

export function formatSliceProviderAuth(
  entry: SliceProviderAuthLike,
  options: {
    readonly separator?: SliceAuthProviderSeparator
    readonly includeOrgPlan?: boolean
  } = {},
): string {
  const separator = options.separator ?? ":"
  const identity = formatSliceAuthIdentity(entry)
  const stateDetail = sliceAuthNeedsAttention(entry.state) ? `state=${entry.state}` : ""
  const details = options.includeOrgPlan === false
    ? [stateDetail].filter(Boolean)
    : [
        stateDetail,
        entry.organization_name || entry.organization_id ? `org=${entry.organization_name || entry.organization_id}` : "",
        entry.subscription_type ? `plan=${entry.subscription_type}` : "",
      ].filter(Boolean)
  return [`${entry.provider}${separator}${identity}`, ...details].join("/")
}

export function formatSliceProviderAccounts(slice: SliceRecordLike): string {
  const accounts = slice.provider_auth ?? []
  if (accounts.length === 0) {
    return "none"
  }
  return accounts
    .map((entry) => formatSliceProviderAuth(entry, {
      separator: "=",
      includeOrgPlan: false,
    }))
    .join(", ")
}

export function formatSliceBackendProviderAccount(
  slice: SliceRecordLike,
  provider: string,
): string | null {
  return formatProviderAccountForBackend(slice.provider_auth ?? [], provider, slice.providers ?? [])
}

export function formatProviderAccountForBackend(
  accounts: readonly SliceProviderAuthLike[] | null | undefined,
  provider: string,
  advertisedProviders: readonly string[] | null | undefined = undefined,
): string | null {
  const providerId = provider.trim()
  if (!providerId) {
    return null
  }
  const account = (accounts ?? []).find((entry) => sliceProviderMatches(entry.provider, providerId))
  if (account) {
    return account.account_profile?.trim()
      || account.alias?.trim()
      || account.email?.trim()
      || shortAccountId(account.account_id)
      || fallbackSliceAuthIdentity(account.state)
  }
  const targeted = sliceProviderNames(advertisedProviders ?? []).some((target) => sliceProviderMatches(target, providerId))
  return targeted ? "auth missing" : null
}

export function formatSliceProviderAuthStatus(slice: SliceRecordLike): string | null {
  const coverage = sliceProviderAuthCoverage(slice)
  const accounts = slice.provider_auth ?? []
  if (accounts.length > 0) {
    const gaps = [
      coverage.missingProviders.length > 0 ? `missing ${formatSliceProviderList(coverage.missingProviders)}` : "",
      coverage.staleProviders.length > 0 ? `refresh ${formatSliceProviderList(coverage.staleProviders)}` : "",
    ].filter(Boolean)
    return [`auth ${formatSliceProviderAccounts(slice)}`, ...gaps].join("; ")
  }
  if (coverage.providers.length === 0) {
    return null
  }
  return `auth missing ${formatSliceProviderList(coverage.providers)}`
}

export function formatSliceProviderAuthReadiness(slice: SliceRecordLike): string {
  const coverage = sliceProviderAuthCoverage(slice)
  const parts = [
    coverage.missingProviders.length > 0 ? `missing ${formatSliceProviderList(coverage.missingProviders)}` : "",
    coverage.staleProviders.length > 0 ? `refresh ${formatSliceProviderList(coverage.staleProviders)}` : "",
  ].filter(Boolean)
  if (parts.length > 0) {
    return parts.join("; ")
  }
  if (coverage.providers.length > 0) {
    return `ready ${formatSliceProviderList(coverage.providers)}`
  }
  return "no provider targets"
}

export function formatSliceAuthIdentity(entry: SliceProviderAuthLike): string {
  const identity = entry.email
    || entry.account_id
    || entry.auth_type
    || fallbackSliceAuthIdentity(entry.state)
  const profile = entry.account_profile?.trim() || entry.alias?.trim()
  if (profile && profile !== identity) {
    return `${profile} (${identity})`
  }
  return identity
}

export function sliceProviderAuthCoverage(slice: SliceRecordLike): SliceProviderAuthCoverage {
  const providers = sliceProviderNames(slice.providers ?? [])
  const authProviders = sliceProviderNames((slice.provider_auth ?? []).map((entry) => entry.provider))
  const derivedProviders = providers.length > 0 ? providers : authProviders
  const missingProviders = derivedProviders.filter((provider) => !authProviders.some((authProvider) => sliceProviderMatches(authProvider, provider)))
  const staleProviders = sliceProviderNames((slice.provider_auth ?? [])
    .filter((entry) => sliceAuthNeedsAttention(entry.state))
    .map((entry) => entry.provider)
    .filter((authProvider) => derivedProviders.some((provider) => sliceProviderMatches(authProvider, provider))))
  return {
    providers: derivedProviders,
    authProviders,
    missingProviders,
    staleProviders,
    hasHealthyCoverage: derivedProviders.length > 0 && missingProviders.length === 0 && staleProviders.length === 0,
    needsAttention: derivedProviders.length === 0 || missingProviders.length > 0 || staleProviders.length > 0,
  }
}

export function formatSliceProviderList(providers: readonly string[], limit = 3): string {
  const names = sliceProviderNames(providers)
  const visible = names.slice(0, limit).join(", ")
  const suffix = names.length > limit ? `, +${names.length - limit} more` : ""
  return `${visible}${suffix}`
}

export function sliceAuthNeedsAttention(state: string): boolean {
  return state !== "configured" && state !== "authenticated"
}

function sliceProviderNames(providers: readonly string[]): string[] {
  return [...new Set(providers.map((provider) => provider.trim()).filter(Boolean))]
}

function sliceProviderMatches(authProvider: string, advertisedProvider: string): boolean {
  return authProvider === advertisedProvider || authProvider.startsWith(`${advertisedProvider}:`)
}

function shortAccountId(accountId: string | null | undefined): string | null {
  const value = accountId?.trim()
  if (!value) {
    return null
  }
  return value.length <= 12 ? value : `${value.slice(0, 8)}...${value.slice(-4)}`
}

function fallbackSliceAuthIdentity(state: string): string {
  return sliceAuthNeedsAttention(state) ? "auth missing" : "account unknown"
}
