export type KernelResourceTelemetryMetadata = {
  scope: string
  authoritative: boolean
  targetId: string
  source: string
}

export type KernelResourceTelemetryMemory = {
  usedBytes: number
  totalBytes: number
  availableBytes: number
}

export type KernelResourceTelemetryDisk = {
  usedBytes: number
  totalBytes: number
  availableBytes: number
}

export type KernelResourceTelemetryProcess = {
  count: number
  rssBytes: number
}

export type KernelResourceTelemetryLogs = {
  bytes: number
}

export type KernelResourceTelemetrySnapshot = {
  schema: string
  capturedAt: string
  capturedAtMonotonicMs: number
  telemetry: KernelResourceTelemetryMetadata
  cpuPercent: number
  cpuSampleWindowMs: number
  memory: KernelResourceTelemetryMemory
  disk: KernelResourceTelemetryDisk
  process: KernelResourceTelemetryProcess
  logs: KernelResourceTelemetryLogs
}

export type KernelResourceTelemetryResponse = {
  KernelResourceTelemetry: {
    snapshot: KernelResourceTelemetrySnapshot
  }
}
