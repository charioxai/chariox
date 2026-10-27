type ResponsePaneRepaintControllerOptions<TimerHandle> = {
  scheduleTimer: (callback: () => void, delayMs: number) => TimerHandle
  repaint: () => void
}

export type ResponsePaneRepaintController = {
  refreshFocus(): void
}

export function createResponsePaneRepaintController<TimerHandle>(
  options: ResponsePaneRepaintControllerOptions<TimerHandle>,
): ResponsePaneRepaintController {
  let refreshGeneration = 0

  const repaintForGeneration = (generation: number) => {
    if (generation !== refreshGeneration) {
      return
    }
    options.repaint()
  }

  return {
    refreshFocus() {
      const generation = ++refreshGeneration
      repaintForGeneration(generation)
      options.scheduleTimer(() => {
        repaintForGeneration(generation)
      }, 0)
    },
  }
}
