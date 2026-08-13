import {
  normalizeRuntimeSession,
  type RuntimeSession,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import { deleteKernelRequest, exportDebugBundleRequest, getDaemonHealthRequest } from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"
import type { DaemonHealthProjection } from "@chariox/kernel-client"

export async function deleteKernel(client: LocalIpcClient): Promise<{
  kernelId: string
  deletedSessions: RuntimeSession[]
}> {
  const response = await client.send<Record<string, unknown>>(deleteKernelRequest())
  const payload = expectVariant<{ kernel_id: string; deleted_sessions: RuntimeSession[] }>(response, "KernelDeleted")
  return {
    kernelId: payload.kernel_id,
    deletedSessions: payload.deleted_sessions.map(normalizeRuntimeSession),
  }
}

export async function getDaemonHealth(client: LocalIpcClient): Promise<DaemonHealthProjection> {
  const response = await client.send<Record<string, unknown>>(getDaemonHealthRequest())
  return expectVariant<{ projection: DaemonHealthProjection }>(response, "DaemonHealth").projection
}

export type ExportDebugBundleResult = {
  bundleDir: string
  manifestPath: string
  logsPath: string
  logRoot: string
  recordCount: number
  limit: number
}

export async function exportDebugBundle(
  client: LocalIpcClient,
  sessionId: string,
  label: string | null,
): Promise<ExportDebugBundleResult> {
  const response = await client.send<Record<string, unknown>>(exportDebugBundleRequest(sessionId, { bundleLabel: label }))
  const payload = expectVariant<{
    bundle_dir: string
    manifest_path: string
    logs_path: string
    log_root: string
    record_count: number
    limit: number
  }>(response, "DebugBundleExported")
  return {
    bundleDir: payload.bundle_dir,
    manifestPath: payload.manifest_path,
    logsPath: payload.logs_path,
    logRoot: payload.log_root,
    recordCount: payload.record_count,
    limit: payload.limit,
  }
}
