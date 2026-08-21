import type { WaitingRoomState } from "./waiting-room-types.js"

export function createWaitingRoomLaunchOwnershipTracker(initialState: WaitingRoomState) {
  let signature = waitingRoomLaunchIntentSignature(initialState)
  let revision = 0
  return {
    update(state: WaitingRoomState): void {
      const nextSignature = waitingRoomLaunchIntentSignature(state)
      if (nextSignature !== signature) {
        signature = nextSignature
        revision += 1
      }
    },
    revision: () => revision,
  }
}

function waitingRoomLaunchIntentSignature(state: WaitingRoomState): string {
  return JSON.stringify([
    state.selectedMachineRef,
    state.providerId,
    state.accountProfileId,
    state.modelId,
    state.effort,
    state.executionMode,
    state.permissionLevel,
    state.workspaceLiveSyncMode,
    state.projectSelectionId,
    state.worktreeSelectionId,
    state.sliceSelectionId,
    state.managedComputeClass,
    state.managedRegion,
    state.managedKernelContext,
    state.managedContextSourceTargetId,
    state.managedDevelopmentMode,
    state.managedRepositoryMode,
    state.managedProviderAccountSource,
    state.managedGitCredentialSource,
    state.managedAutoStopPreset,
    state.managedCustomMinimumRuntimeSeconds,
    state.managedCustomIdleDelaySeconds,
  ])
}
