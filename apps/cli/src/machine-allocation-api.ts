import {
  controlPlaneRequest,
  type ControlPlaneRequest,
  type ControlPlaneResponse,
} from "@chariox/kernel-client/ipc-control-plane-requests"
import type { LocalIpcClient } from "./ipc.js"
import { expectVariant } from "./ipc-response.js"

export type MachineAllocationCapability = NonNullable<Extract<ControlPlaneResponse, { kind: "capabilities" }>["machine_allocation"]>
export type MachineAllocation = Extract<ControlPlaneResponse, { kind: "machine" }>

export async function machineAllocationRequest(
  client: LocalIpcClient,
  request: ControlPlaneRequest,
): Promise<ControlPlaneResponse> {
  const response = await client.send<Record<string, unknown>>(controlPlaneRequest(request))
  return expectVariant<{ result: ControlPlaneResponse }>(response, "ControlPlane").result
}

export async function machineAllocationCapability(client: LocalIpcClient): Promise<MachineAllocationCapability | null> {
  const result = await machineAllocationRequest(client, { action: "capabilities" })
  if (result.kind !== "capabilities") throw new Error("The kernel returned a different connected-service operation.")
  return result.machine_allocation
}

export async function acquireMachineAllocation(
  client: LocalIpcClient,
  capabilityRevision: string,
  requestId: string,
): Promise<MachineAllocation> {
  const result = await machineAllocationRequest(client, {
    action: "acquire", capability_revision: capabilityRevision, request_id: requestId,
  })
  if (result.kind !== "machine") throw new Error("The kernel returned a different connected-service operation.")
  return result
}

export async function getMachineAllocation(client: LocalIpcClient, allocationRef: string): Promise<MachineAllocation> {
  const result = await machineAllocationRequest(client, { action: "get", allocation_ref: allocationRef })
  if (result.kind !== "machine" || result.allocation_ref !== allocationRef) {
    throw new Error("The kernel returned a different machine allocation.")
  }
  return result
}
