import type { WaitingRoomActivationControllerDeps } from "./waiting-room-activation-controller.js"
import type { createSlice } from "./slice-api.js"

export function cliWaitingRoomSliceApiOptions(
  options: Parameters<NonNullable<WaitingRoomActivationControllerDeps["createSlice"]>>[0],
): Parameters<typeof createSlice>[1] {
  return {
    name: options.name,
    displayMode: options.displayMode,
    workspaceId: options.workspaceId,
    worktreeId: options.worktreeId,
    workspaceMount: options.workspaceMount,
    ...(options.developmentSetup !== undefined
      ? { developmentSetup: options.developmentSetup }
      : {}),
    ...(options.workerKernelRef !== undefined
      ? { workerKernelRef: options.workerKernelRef }
      : {}),
  }
}
