import type { MachineAllocationCapability } from "./machine-allocation-api.js"
import type { WaitingRoomRemoteState } from "./waiting-room-types.js"
export { NEW_MACHINE_ALLOCATION_REF } from "@chariox/kernel-client/waiting-room-runtime-placement"
import { NEW_MACHINE_ALLOCATION_REF } from "@chariox/kernel-client/waiting-room-runtime-placement"

export function waitingRoomAllocationRemoteState(capability: MachineAllocationCapability | null | undefined): WaitingRoomRemoteState {
  return { ...(capability !== undefined ? { machineAllocationCapability: capability } : {}), machineAllocationOffered: Boolean(capability?.offer && capability.actions.includes("acquire")) }
}
export function waitingRoomConfiguresNewManagedMachine(machineRef: string | null | undefined): boolean { return machineRef === NEW_MACHINE_ALLOCATION_REF }
export const waitingRoomUsesManagedMachine = waitingRoomConfiguresNewManagedMachine
