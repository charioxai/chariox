import type {
  ExtensionGrant,
  ExtensionKind,
  ExtensionSource,
} from "./kernel-types.js"

export type ExtensionGrantIdentity = Pick<ExtensionGrant, "source" | "kind" | "name">

/** Legacy grants without a serialized source are home-owned. */
export function extensionGrantSource(
  grant: Pick<ExtensionGrant, "source"> | null | undefined,
): ExtensionSource {
  return grant?.source === "worker" ? "worker" : "home"
}

export function extensionGrantKey(
  grant: ExtensionGrantIdentity,
): string {
  return `${extensionGrantSource(grant)}:${grant.kind}:${grant.name}`
}

export function extensionIdentityKey(
  source: ExtensionSource,
  kind: ExtensionKind,
  name: string,
): string {
  return `${source}:${kind}:${name}`
}
