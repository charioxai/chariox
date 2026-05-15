import {
  normalizeRuntimeSession,
  type RuntimeSession,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import { deleteKernelRequest } from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

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
