export type WaitingRoomHiddenKernelControllerOptions = {
  initialHiddenKernelIds: readonly string[]
  persistHiddenKernelIds: (kernelIds: string[]) => void
}

export function createWaitingRoomHiddenKernelController(options: WaitingRoomHiddenKernelControllerOptions) {
  const hiddenKernelIds = new Set(options.initialHiddenKernelIds)

  const sortedHiddenKernelIds = () => [...hiddenKernelIds].sort()

  return {
    isKernelHidden: (kernelId: string) => hiddenKernelIds.has(kernelId),
    hideKernel: (kernelId: string) => {
      hiddenKernelIds.add(kernelId)
      options.persistHiddenKernelIds(sortedHiddenKernelIds())
    },
  }
}
