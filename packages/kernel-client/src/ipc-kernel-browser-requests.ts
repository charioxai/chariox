// MD-2: the same sessionless kernel contract for web, TUI and native clients.
import type { KernelBrowserCommand, KernelBrowserRequest, KernelBrowserResult } from "./kernel-types.js"

// MP-08 / MP-10 / MP-11: 487 main has no KernelBrowser union; 494 supplies this contract.
export const kernelDesktopDisplayMinimumProtocolVersion = 474
export const kernelComputerMinimumProtocolVersion = 494
export const kernelBrowserMinimumProtocolVersion = 494
export const userDomainAccessMinimumProtocolVersion = 494

export function kernelBrowserRequest(command: KernelBrowserCommand): KernelBrowserRequest {
  return { KernelBrowser: { command } }
}

export type KernelBrowserResponse = { KernelBrowser: { result: KernelBrowserResult } }
