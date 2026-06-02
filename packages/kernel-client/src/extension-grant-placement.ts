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

export function formatExtensionGrantPlacementSummary(
  grants: readonly { readonly kind: string }[] | null | undefined,
  options: {
    readonly remote: boolean
    readonly countSeparator?: string
  },
): string {
  const visibleGrants = grants ?? []
  const countSeparator = options.countSeparator ?? " "
  const counts = visibleGrants.reduce<Record<string, number>>((acc, grant) => {
    acc[grant.kind] = (acc[grant.kind] ?? 0) + 1
    return acc
  }, {})
  const byKind = ["mcp", "script", "connector", "skill"]
    .map((kind) => counts[kind] ? `${kind}${countSeparator}${counts[kind]}` : null)
    .filter(Boolean)
    .join(", ")
  const placement = formatExtensionGrantPlacement(visibleGrants, options.remote)
  return `${visibleGrants.length} grant${visibleGrants.length === 1 ? "" : "s"} (${placement}${byKind ? `; ${byKind}` : ""})`
}

export function formatExtensionGrantPlacement(
  grants: readonly { readonly kind: string }[] | null | undefined,
  remote: boolean,
): string {
  if (!remote) {
    return "worker-local"
  }
  const visibleGrants = grants ?? []
  const activeHomeProxy = hasActiveHomeProxyExtensionGrants(visibleGrants)
  const passiveSkillSnapshot = visibleGrants.some((grant) => grant.kind === "skill")
  return [
    activeHomeProxy ? "active tools home-proxy" : null,
    passiveSkillSnapshot ? "skills snapshot" : null,
  ].filter(Boolean).join("; ") || "home-proxy"
}

export function formatExtensionGrantRuntimeDetail(
  grants: readonly { readonly kind: string }[] | null | undefined,
  remote: boolean,
): string {
  if (!remote) {
    return "worker-local: definition and execution stay on the selected kernel"
  }
  const visibleGrants = grants ?? []
  const activeHomeProxy = hasActiveHomeProxyExtensionGrants(visibleGrants)
  const passiveSkillSnapshot = visibleGrants.some((grant) => grant.kind === "skill")
  if (activeHomeProxy && passiveSkillSnapshot) {
    return "home-proxy tools execute on home with home-owned grants and credentials; skills are passive snapshots"
  }
  if (activeHomeProxy) {
    return "home-proxy: home owns definition, grants, credentials, and execution; worker receives projected tools"
  }
  if (passiveSkillSnapshot) {
    return "skill snapshot: home projects passive content; executable helpers require a separate tool grant"
  }
  return "home-proxy: home remains authoritative for projected remote extension tools"
}

export function formatExtensionAuthorityBoundaryDetail(
  grants: readonly { readonly kind: string }[] | null | undefined,
  remote: boolean,
): string {
  if (!remote) {
    return "definition and execution stay on the selected kernel"
  }
  const visibleGrants = grants ?? []
  const activeHomeProxy = hasActiveHomeProxyExtensionGrants(visibleGrants)
  const passiveSkillSnapshot = visibleGrants.some((grant) => grant.kind === "skill")
  if (activeHomeProxy) {
    return "home validates every call; credentials never leave home"
  }
  if (passiveSkillSnapshot) {
    return "passive content only; executable helpers require separate tool grants"
  }
  return "home remains authoritative for projected remote extension tools"
}
