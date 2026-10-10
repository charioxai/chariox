import { cloudDirectoryProjection, type CloudClient } from "./cloud-client.js"
import type { WaitingRoomRemoteState } from "./waiting-room-types.js"

export function createCloudWaitingRoomController(options: {
  client?: CloudClient | undefined; isKernelConnected: () => boolean
  setMachines: (machines: NonNullable<WaitingRoomRemoteState["machines"]>) => void
  setKernels: (kernels: NonNullable<WaitingRoomRemoteState["kernels"]>) => void
  setStatus: (status: "loading" | "ready" | "error") => void
  reconcile: () => void
}) {
  let pending: Promise<void> | undefined
  return async (): Promise<void> => {
    if (!options.client || options.isKernelConnected()) return
    if (pending) return pending
    pending = (async () => {
      const profile = await options.client!.profile()
      if (options.isKernelConnected()) return
      if (!profile) {
        options.setMachines([]); options.setKernels([]); options.setStatus("ready"); options.reconcile()
        return
      }
      options.setStatus("loading")
      try {
        const projection = cloudDirectoryProjection(await options.client!.directory())
        // A kernel pivot may have completed while the Cloud request was pending.
        if (options.isKernelConnected()) return
        options.setMachines(projection.machines); options.setKernels(projection.kernels)
        options.setStatus("ready"); options.reconcile()
      } catch { if (!options.isKernelConnected()) options.setStatus("error") }
    })().finally(() => { pending = undefined })
    return pending
  }
}
