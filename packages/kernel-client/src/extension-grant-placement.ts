import type { RemoteExtensionManifestSyncStatus } from "./kernel-types.js"

export function hasActiveHomeProxyExtensionGrants(
  grants: readonly { readonly kind: string }[] | null | undefined,
): boolean {
  return Boolean(grants?.some((grant) => grant.kind === "mcp" || grant.kind === "script" || grant.kind === "connector"))
}

export function shouldShowRemoteExtensionManifestSync(
  grants: readonly { readonly kind: string }[] | null | undefined,
  status: RemoteExtensionManifestSyncStatus | null | undefined,
): boolean {
  return Boolean(status || hasActiveHomeProxyExtensionGrants(grants))
}
