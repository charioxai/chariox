import type { DaemonHealthResponse } from "./kernel-types.js"

export function deleteKernelRequest() {
  return { DeleteKernel: null }
}

export type GetDaemonHealthRequest = { GetDaemonHealth: null }

export function getDaemonHealthRequest(): GetDaemonHealthRequest {
  return { GetDaemonHealth: null }
}

export type GetDaemonHealthResponse = DaemonHealthResponse
