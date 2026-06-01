import type { SliceRecord } from "./cli-types.js"

export type SliceAuthProviderSeparator = ":" | " " | "="

export function formatSliceScope(slice: SliceRecord): string {
  return slice.worktree_id || slice.workspace_mount || slice.workspace_id || "-"
}

export function formatSliceRelayLabel(
  slice: SliceRecord,
  options: { includeUrl?: boolean; emptyLabel?: string } = {},
): string {
  const endpoint = slice.relay_endpoint
  if (!endpoint?.url) {
    return options.emptyLabel ?? ""
  }
  const label = endpoint.private ? "private" : "shared"
  return options.includeUrl ? `${label}:${endpoint.url}` : label
}

export function formatSliceOperation(slice: SliceRecord): string {
  if (!slice.last_operation && !slice.last_operation_status && !slice.last_error) {
    return ""
  }
  const status = slice.last_operation_status ? `:${slice.last_operation_status}` : ""
  const error = slice.last_error ? ` error=${slice.last_error}` : ""
  return `${slice.last_operation ?? "operation"}${status}${error}`
}

export function formatSliceDiagnostics(slice: SliceRecord): string {
  const operation = formatSliceOperation(slice)
  return operation ? ` last_operation=${operation}` : ""
}

export function formatSliceProviderAuth(
  entry: NonNullable<SliceRecord["provider_auth"]>[number],
  options: {
    separator?: SliceAuthProviderSeparator
    includeOrgPlan?: boolean
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

export function formatSliceAuthIdentity(entry: NonNullable<SliceRecord["provider_auth"]>[number]): string {
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
