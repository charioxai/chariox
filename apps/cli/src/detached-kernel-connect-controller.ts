import type { FooterFlash } from "./footer-flash-controller.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import type { ProviderCommandCatalogs } from "./provider-command-catalog.js"

type DetachedKernelConnectControllerOptions = {
  logInfo?: (message: string, fields?: Record<string, unknown>) => void
  flashFooter: (message: string, tone: FooterFlash["tone"]) => void
  getProviderCatalog: () => Promise<ProviderCatalog>
  getProviderCommandCatalogs: () => Promise<ProviderCommandCatalogs>
  invalidateWaitingRoomInventory: () => void
  setProviderCatalog: (catalog: ProviderCatalog) => void
  setProviderCommandCatalogs: (catalogs: ProviderCommandCatalogs) => void
  setKernelConnected: (next: boolean) => void
  setDaemonDisconnected: (next: boolean) => void
  refreshWaitingRoomData: () => Promise<void>
}

export function createDetachedKernelConnectController(
  options: DetachedKernelConnectControllerOptions,
) {
  const connect = async () => {
    options.logInfo?.("connecting detached cli to configured kernel endpoint")
    options.flashFooter("connecting to kernel...", "info")
    const [catalog, commandCatalogs] = await Promise.all([
      options.getProviderCatalog(),
      options.getProviderCommandCatalogs(),
    ])
    options.invalidateWaitingRoomInventory()
    options.setProviderCatalog(catalog)
    options.setProviderCommandCatalogs(commandCatalogs)
    options.setKernelConnected(true)
    options.setDaemonDisconnected(false)
    await options.refreshWaitingRoomData()
    options.flashFooter("connected to kernel", "info")
  }

  return { connect }
}
