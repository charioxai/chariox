import type { LocalIpcClient } from "./ipc.js"
import type { CharioxLogger } from "./logging.js"
import { getProviderCatalog } from "./provider-api.js"
import type { ProviderCatalog } from "./provider-catalog.js"

export type WaitingRoomCatalogSelection = {
  providerId: string
  accountProfileId?: string | null
}

/**
 * Load the catalog from the kernel represented by `client`.
 *
 * A client pivot is not a catalog event: the target kernel can have a
 * different provider installation and account materialization. Keep this
 * request strict so a target discovery failure cannot turn into a local
 * fallback or a catalog from the source kernel.
 */
export function loadProviderCatalogForKernel(
  client: LocalIpcClient,
  logger: CharioxLogger | null | undefined,
  selection: WaitingRoomCatalogSelection,
): Promise<ProviderCatalog> {
  return getProviderCatalog(client, logger, {
    provider: selection.providerId,
    accountProfile: selection.accountProfileId ?? "default",
    executionLocation: { kind: "local" },
  }, false)
}
