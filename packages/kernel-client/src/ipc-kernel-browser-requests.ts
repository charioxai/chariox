// MD-2: the same sessionless kernel contract for web, TUI and native clients.
import type { KernelBrowserCommand, KernelBrowserRequest, KernelBrowserResult } from "./kernel-types.js"

export const kernelBrowserMinimumProtocolVersion = 443
export const userDomainAccessMinimumProtocolVersion = 443

export function kernelBrowserRequest(command: KernelBrowserCommand): KernelBrowserRequest {
  return { KernelBrowser: { command } }
}

export type KernelBrowserResponse = { KernelBrowser: { result: KernelBrowserResult } }
