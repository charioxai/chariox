import type { ManagedContextTransferStatus } from "@chariox/kernel-client/ipc-managed-context-requests"
import type { LocalIpcClient } from "./ipc.js"
import { getManagedContextTransferStatusRequest } from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export async function getManagedContextTransferStatus(
  client: LocalIpcClient,
  contextId: string,
): Promise<ManagedContextTransferStatus> {
  const response = await client.send<Record<string, unknown>>(
    getManagedContextTransferStatusRequest(contextId),
  )
  return expectVariant<{ status: ManagedContextTransferStatus }>(
    response,
    "ManagedContextTransferStatus",
  ).status
}
