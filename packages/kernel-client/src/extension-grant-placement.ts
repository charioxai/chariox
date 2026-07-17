import type { RemoteExtensionManifestSyncStatus } from "./kernel-types.js"
import { extensionGrantSource } from "./extension-grant-source.js"

type ExtensionPlacementGrant = {
  readonly source?: "home" | "worker"
  readonly kind: string
}

export function hasActiveHomeProxyExtensionGrants(
  grants: readonly ExtensionPlacementGrant[] | null | undefined,
): boolean {
  return Boolean(grants?.some((grant) => extensionGrantSource(grant) === "home"
    && (grant.kind === "mcp" || grant.kind === "script" || grant.kind === "connector")))
}

export function hasWorkerExtensionGrants(
  grants: readonly ExtensionPlacementGrant[] | null | undefined,
): boolean {
  return Boolean(grants?.some((grant) => extensionGrantSource(grant) === "worker"))
}

export function shouldShowRemoteExtensionManifestSync(
  grants: readonly ExtensionPlacementGrant[] | null | undefined,
  status: RemoteExtensionManifestSyncStatus | null | undefined,
): boolean {
  return Boolean(status?.pending_revoke || hasActiveHomeProxyExtensionGrants(grants))
}

export function shouldShowWorkerExtensionGrantSync(
  grants: readonly ExtensionPlacementGrant[] | null | undefined,
  status: RemoteExtensionManifestSyncStatus | null | undefined,
): boolean {
  return Boolean(status?.pending_revoke || hasWorkerExtensionGrants(grants))
}

export function formatExtensionGrantPlacementSummary(
  grants: readonly ExtensionPlacementGrant[] | null | undefined,
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
  grants: readonly ExtensionPlacementGrant[] | null | undefined,
  remote: boolean,
): string {
  const visibleGrants = grants ?? []
  if (!remote) {
    const homeLocal = visibleGrants.some((grant) => extensionGrantSource(grant) === "home")
    const workerLocal = hasWorkerExtensionGrants(visibleGrants)
    return [
      homeLocal || !workerLocal ? "home-local" : null,
      workerLocal ? "worker-local" : null,
    ].filter(Boolean).join("; ")
  }
  const activeHomeProxy = hasActiveHomeProxyExtensionGrants(visibleGrants)
  const passiveSkillSnapshot = visibleGrants.some((grant) => extensionGrantSource(grant) === "home" && grant.kind === "skill")
  const workerLocal = hasWorkerExtensionGrants(visibleGrants)
  return [
    workerLocal ? "worker-local" : null,
    activeHomeProxy ? "active tools home-proxy" : null,
    passiveSkillSnapshot ? "skills snapshot" : null,
  ].filter(Boolean).join("; ") || "home-proxy"
}

export function formatExtensionGrantRuntimeDetail(
  grants: readonly ExtensionPlacementGrant[] | null | undefined,
  remote: boolean,
): string {
  const visibleGrants = grants ?? []
  if (!remote) {
    const homeLocal = visibleGrants.some((grant) => extensionGrantSource(grant) === "home")
    const workerLocal = hasWorkerExtensionGrants(visibleGrants)
    if (homeLocal && workerLocal) {
      return "source-local: each grant's definition, credentials, and execution stay on its source kernel"
    }
    return workerLocal
      ? "worker-local: definition, credentials, and execution stay on the worker kernel"
      : "home-local: definition, credentials, and execution stay on the home kernel"
  }
  const activeHomeProxy = hasActiveHomeProxyExtensionGrants(visibleGrants)
  const passiveSkillSnapshot = visibleGrants.some((grant) => extensionGrantSource(grant) === "home" && grant.kind === "skill")
  const workerLocal = hasWorkerExtensionGrants(visibleGrants)
  const workerDetail = workerLocal
    ? "worker-local extensions use worker definitions, credentials, and execution"
    : null
  if (activeHomeProxy && passiveSkillSnapshot) {
    return [workerDetail, "home-proxy tools execute on home with home-owned grants and credentials; skills are passive snapshots"].filter(Boolean).join("; ")
  }
  if (activeHomeProxy) {
    return [workerDetail, "home-proxy: home owns definition, grants, credentials, and execution; worker receives projected tools"].filter(Boolean).join("; ")
  }
  if (passiveSkillSnapshot) {
    return [workerDetail, "skill snapshot: home projects passive content; executable helpers require a separate tool grant"].filter(Boolean).join("; ")
  }
  if (workerDetail) {
    return workerDetail
  }
  return "home-proxy: home remains authoritative for projected remote extension tools"
}

export function formatExtensionAuthorityBoundaryDetail(
  grants: readonly ExtensionPlacementGrant[] | null | undefined,
  remote: boolean,
): string {
  const visibleGrants = grants ?? []
  if (!remote) {
    const homeLocal = visibleGrants.some((grant) => extensionGrantSource(grant) === "home")
    const workerLocal = hasWorkerExtensionGrants(visibleGrants)
    if (homeLocal && workerLocal) {
      return "each source resolves, validates, and executes locally; credentials never cross kernels"
    }
    return workerLocal
      ? "worker resolves, validates, and executes locally; credentials stay on worker"
      : "home resolves, validates, and executes locally; credentials stay on home"
  }
  const activeHomeProxy = hasActiveHomeProxyExtensionGrants(visibleGrants)
  const passiveSkillSnapshot = visibleGrants.some((grant) => extensionGrantSource(grant) === "home" && grant.kind === "skill")
  const workerLocal = hasWorkerExtensionGrants(visibleGrants)
  if (activeHomeProxy) {
    return workerLocal
      ? "each source validates and executes its own grants; credentials never cross kernels"
      : "home validates every call; credentials never leave home"
  }
  if (passiveSkillSnapshot) {
    return workerLocal
      ? "worker grants stay worker-local; home skill content is passive and credentials never cross kernels"
      : "passive content only; executable helpers require separate tool grants"
  }
  if (workerLocal) {
    return "worker resolves, validates, and executes locally; worker credentials stay on worker"
  }
  return "home remains authoritative for projected remote extension tools"
}
