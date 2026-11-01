// MD-2: the same sessionless kernel contract for web, TUI and native clients.
import type { KernelBrowserCommand, KernelBrowserRequest, KernelBrowserResult } from "./kernel-types.js"

// MP-08 / MP-10 / MP-11: 487 main has no KernelBrowser union; 494 supplies this contract.
export const kernelDesktopDisplayMinimumProtocolVersion = 474
export const kernelComputerMinimumProtocolVersion = 494
export const kernelBrowserMinimumProtocolVersion = 443
export const userDomainAccessMinimumProtocolVersion = 443
// MP-08 / MP-11: typed HOME -> leased desktop grants are allocated 500.
export const leasedComputerMinimumProtocolVersion = 500
// MP-08/MP-11: retain the integrated agent-tab provenance floor.
export const kernelBrowserAgentTabsMinimumProtocolVersion = 486
// MP-08/MP-10/MP-11: allocated shared producer resumption.
export const kernelDisplayResumeMinimumProtocolVersion = 525
// MP-08/MP-10/MP-11 (P1): one page display per Tab, parked while the DOM mirror paints.
export const kernelTabDisplayMinimumProtocolVersion = 720

export function kernelBrowserRequest(command: KernelBrowserCommand): KernelBrowserRequest {
  return { KernelBrowser: { command } }
}

export type KernelBrowserResponse = { KernelBrowser: { result: KernelBrowserResult } }
