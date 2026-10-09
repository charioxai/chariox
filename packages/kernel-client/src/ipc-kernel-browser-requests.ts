// MD-2: the same sessionless kernel contract for web, TUI and native clients.
import type { KernelBrowserCommand, KernelBrowserRequest, KernelBrowserResult } from "./kernel-types.js"

// MP-08/MP-11: coordinator allocation for the combined, unmerged PR937.
export const kernelBrowserAgentTabsMinimumProtocolVersion = 481
export const kernelBrowserMinimumProtocolVersion = 443
export const userDomainAccessMinimumProtocolVersion = 443

export function kernelBrowserRequest(command: KernelBrowserCommand): KernelBrowserRequest {
  return { KernelBrowser: { command } }
}

export type KernelBrowserResponse = { KernelBrowser: { result: KernelBrowserResult } }
