import type { LocalIpcClient } from "./ipc.js"
import type { CharioxLogger } from "./logging.js"
import type { ProviderCatalogExecutionLocation } from "./ipc-requests.js"
import { getProviderCatalog } from "./provider-api.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

export type WaitingRoomCatalogSelection = {
  providerId: string
  accountProfileId?: string | null
}

export function providerCatalogExecutionLocation(
  state: Pick<WaitingRoomState, "sliceSelectionId" | "selectedKernelRef">,
  activeDirectTargetKernelId?: string | null,
): ProviderCatalogExecutionLocation {
  const sliceRef = state.sliceSelectionId?.trim()
  if (sliceRef && !["none", "new"].includes(sliceRef)) {
    return { kind: "slice", slice_ref: sliceRef }
  }
  const kernelRef = state.selectedKernelRef?.trim()
  const activeTargetKernelRef = activeDirectTargetKernelId?.trim()
  if (
    !kernelRef
    || kernelRef === "local"
    || (activeTargetKernelRef && kernelRef === activeTargetKernelRef)
  ) {
    return { kind: "local" }
  }
  return { kind: "worker", kernel_ref: kernelRef }
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
