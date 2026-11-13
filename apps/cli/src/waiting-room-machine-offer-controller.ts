import { waitingRoomConfiguresNewManagedMachine } from "./waiting-room-machine-allocation.js"
import type { WaitingRoomRemoteState, WaitingRoomState } from "./waiting-room-types.js"
export type WaitingRoomMachineOfferKeyEvent = { name: string; eventType?: string }
export function createWaitingRoomMachineOfferController(deps: {
  isOpen(): boolean
  state(): WaitingRoomState
  remote(): WaitingRoomRemoteState
  openOverlay(): void
  closeOverlay(): void
}) {
  return {
    open(): boolean {
      if (!waitingRoomConfiguresNewManagedMachine(deps.state().selectedMachineRef) || !deps.remote().machineAllocationCapability?.offer) return false
      deps.openOverlay()
      return true
    },
    handleKey(event: WaitingRoomMachineOfferKeyEvent): boolean {
      if (!deps.isOpen()) return false
      if (event.eventType !== "release" && ["return", "enter"].includes(event.name)) deps.closeOverlay()
      return true
    },
  }
}
