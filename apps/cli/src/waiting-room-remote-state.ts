import { waitingRoomManagedCatalogRemoteState } from "./waiting-room-managed-environments.js"
import type { WaitingRoomTargetState } from "./waiting-room-types.js"

/** MP-08/MP-11: every TUI surface uses the same home/worker placement identity. */
export function waitingRoomRemoteTargets(targets: WaitingRoomTargetState) {
  return {
    homeKernelId: targets.homeKernelId ?? null,
    ...(targets.workspaceId ? { workspaceId: targets.workspaceId } : {}),
    ...(targets.worktreeId ? { worktreeId: targets.worktreeId } : {}),
    ...waitingRoomManagedCatalogRemoteState(targets.managedEnvironmentCatalog),
  }
}
