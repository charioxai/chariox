export type SliceAuthProviderSeparator = ":" | " " | "="

export type SliceProviderAuthLike = {
  readonly provider: string
  readonly state: string
  readonly auth_type?: string | null
  readonly account_id?: string | null
  readonly email?: string | null
  readonly organization_id?: string | null
  readonly organization_name?: string | null
  readonly subscription_type?: string | null
  readonly alias?: string | null
}

export type SliceRecordLike = {
  readonly workspace_id?: string | null
  readonly worktree_id?: string | null
  readonly workspace_mount?: string | null
  readonly provider_auth?: readonly SliceProviderAuthLike[] | null
  readonly relay_endpoint?: {
    readonly url?: string | null
    readonly private?: boolean | null
  } | null
  readonly last_operation?: string | null
  readonly last_operation_status?: string | null
  readonly last_error?: string | null
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

export function formatSliceAuthIdentity(entry: SliceProviderAuthLike): string {
  const identity = entry.email
    || entry.account_id
    || entry.auth_type
    || entry.state
  if (entry.alias && entry.alias !== identity) {
    return `${entry.alias} (${identity})`
  }
  return identity
}

function sliceAuthNeedsAttention(state: string): boolean {
  return state !== "configured" && state !== "authenticated"
}
