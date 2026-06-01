import {
  normalizeRuntimeSession,
  type RuntimeSession,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import { deleteKernelRequest, getDaemonHealthRequest } from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"
import type { DaemonHealthProjection } from "@arroba/kernel-client"

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
