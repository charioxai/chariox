import { cloudDirectoryProjection, type CloudClient } from "./cloud-client.js"
import type { WaitingRoomRemoteState } from "./waiting-room-types.js"

export function createCloudWaitingRoomController(options: {
  client?: CloudClient | undefined; isKernelConnected: () => boolean
  setMachines: (machines: NonNullable<WaitingRoomRemoteState["machines"]>) => void
  setKernels: (kernels: NonNullable<WaitingRoomRemoteState["kernels"]>) => void
  setStatus: (status: "loading" | "ready" | "error") => void
  reconcile: () => void
  hydrateKernel?: (kernelId: string, machineId: string) => Promise<void>
  preferredKernelRef?: () => string | undefined
}) {
  let pending: Promise<void> | undefined
  let initialHydration = !options.isKernelConnected()
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
        if (initialHydration && options.hydrateKernel && projection.kernels.length) {
          options.reconcile()
          const kernel = projection.kernels.find(kernel => kernel.kernel_id === options.preferredKernelRef?.()) ?? projection.kernels[0]!
          await options.hydrateKernel(kernel.kernel_id, kernel.machine_id)
          initialHydration = false
          return
        }
        options.setStatus("ready"); options.reconcile()
      } catch { if (!options.isKernelConnected()) options.setStatus("error") }
    })().finally(() => { pending = undefined })
    return pending
  }
}
