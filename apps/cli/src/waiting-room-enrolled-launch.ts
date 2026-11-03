import type { ManagedEnvironmentSummary } from "@chariox/kernel-client/ipc-managed-environment-requests"
import type { WaitingRoomPreparedManagedLaunch } from "./waiting-room-activation-controller.js"
import type { WaitingRoomLaunchConfig } from "./waiting-room-controller.js"
import type { PreparedManagedEnvironmentLaunch } from "./waiting-room-managed-environment-launch-controller.js"

export function waitingRoomManagedEnvironmentKernelRef(
  launch: WaitingRoomLaunchConfig,
  environment: ManagedEnvironmentSummary,
): string | null {
  return launch.managedEnvironment?.kind === "existing" && environment.contextManifestDigest && launch.ownerKernelRef
    ? launch.ownerKernelRef
    : environment.runtimeKernelId
}

export function prepareWaitingRoomEnrolledLaunch(options: {
  launch: WaitingRoomLaunchConfig
  prepared: Extract<PreparedManagedEnvironmentLaunch, { kind: "enrolled" }>
  assertActive(): void
}): WaitingRoomPreparedManagedLaunch {
  if (options.launch.ownerKernelRef && options.launch.ownerKernelRef !== options.prepared.kernelId) {
    throw new Error("The connected enrolled kernel does not match the Waiting Room owner selection.")
  }
  const { managedEnvironment: _managedEnvironment, ...ordinaryLaunch } = options.launch
  return {
    launch: {
      ...ordinaryLaunch,
      ownerMachineRef: options.prepared.environment.runtimeMachineId,
      ownerKernelRef: options.prepared.kernelId,
    },
    assertActive: options.assertActive,
    verifyProjectBinding: (session) => {
      options.assertActive()
      const selection = options.launch.projectSelection
      if (selection?.kind === "existing" && session.project_id !== selection.project_id) {
        throw new Error("The created session is attached to a different Project than the Waiting Room selection.")
      }
    },
    commit: options.prepared.commit,
    rollback: options.prepared.rollback,
  }
}
