import type {
  DaemonHealthResponse,
  DebugBundleExportedResponse,
  KernelResourceTelemetryResponse,
} from "./kernel-types.js"

export const kernelResourceTelemetryMinimumProtocolVersion = 336

export function deleteKernelRequest() {
  return { DeleteKernel: null }
}

export type GetDaemonHealthRequest = { GetDaemonHealth: null }

export function getDaemonHealthRequest(): GetDaemonHealthRequest {
  return { GetDaemonHealth: null }
}

export type GetDaemonHealthResponse = DaemonHealthResponse

export type GetKernelResourceTelemetryRequest = { GetKernelResourceTelemetry: null }

export function getKernelResourceTelemetryRequest(_options: {
  kernelRef?: string | null
  machineRef?: string | null
} = {}): GetKernelResourceTelemetryRequest {
  return { GetKernelResourceTelemetry: null }
}

export type GetKernelResourceTelemetryResponse = KernelResourceTelemetryResponse

export type ExportDebugBundleRequest = {
  ExportDebugBundle: {
    session_id: string
    bundle_label?: string | null
    limit?: number | null
  }
}

export function exportDebugBundleRequest(
  sessionId: string,
  options: {
    readonly bundleLabel?: string | null
    readonly limit?: number | null
  } = {},
): ExportDebugBundleRequest {
  return {
    ExportDebugBundle: {
      session_id: sessionId,
      bundle_label: options.bundleLabel ?? null,
      limit: options.limit ?? null,
    },
  }
}

export type ExportDebugBundleResponse = DebugBundleExportedResponse
