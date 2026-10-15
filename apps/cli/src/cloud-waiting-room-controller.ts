import { cloudDirectoryProjection, type CloudClient } from "./cloud-client.js"
import type { WaitingRoomRemoteState } from "./waiting-room-types.js"

export function createCloudWaitingRoomController(options: {
  client?: CloudClient | undefined; isKernelConnected: () => boolean
  setMachines: (machines: NonNullable<WaitingRoomRemoteState["machines"]>) => void
  setKernels: (kernels: NonNullable<WaitingRoomRemoteState["kernels"]>) => void
  setStatus: (status: "loading" | "ready" | "error") => void
  reconcile: () => void
  hydrateKernel?: (kernelId: string, machineId: string, isActive: () => boolean) => Promise<void>
  captureActivity?: () => () => boolean
  preferredKernelRef?: () => string | undefined
}) {
  let pending: Promise<void> | undefined
  let initialHydration = !options.isKernelConnected()
  return async (): Promise<void> => {
    if (!options.client || options.isKernelConnected()) return
    if (pending) return pending
    const isActive = options.captureActivity?.() ?? (() => true)
    pending = (async () => {
      const profile = await options.client!.profile()
      if (!isActive()) { initialHydration = false; return }
      if (options.isKernelConnected()) return
      if (!profile) {
        options.setMachines([]); options.setKernels([]); options.setStatus("ready"); options.reconcile()
        return
      }
      options.setStatus("loading")
      try {
        const projection = cloudDirectoryProjection(await options.client!.directory())
        // A kernel pivot may have completed while the Cloud request was pending.
        if (!isActive()) { initialHydration = false; return }
        if (options.isKernelConnected()) return
        options.setMachines(projection.machines); options.setKernels(projection.kernels)
        if (initialHydration && options.hydrateKernel && projection.kernels.length) {
          options.reconcile()
          const kernel = projection.kernels.find(kernel => kernel.kernel_id === options.preferredKernelRef?.()) ?? projection.kernels[0]!
          await options.hydrateKernel(kernel.kernel_id, kernel.machine_id, isActive)
          initialHydration = false
          return
        }
        options.setStatus("ready"); options.reconcile()
      } catch {
        if (!isActive() || options.isKernelConnected()) return
        // The login ended during the request: one signed-out room, not a directory error.
        const current = await options.client!.profile()
        if (!isActive() || options.isKernelConnected()) return
        if (!current) { options.setMachines([]); options.setKernels([]); options.setStatus("ready"); options.reconcile() }
        else options.setStatus("error")
      }
    })().finally(() => { pending = undefined })
    return pending
  }
}
