export type PromptSurfaceMouseControllerDeps<TimerHandle, MouseEvent> = {
  delayMs: number
  scheduleTimer: (callback: () => void, delayMs: number) => TimerHandle
  isPrimaryButton: (event: MouseEvent) => boolean
  copySelection: () => boolean
  hasSelection?: () => boolean
  retainPromptFocus: () => void
}

export type PromptSurfaceMouseController<MouseEvent> = {
  handleMouseUp(event: MouseEvent): void
}

export function createPromptSurfaceMouseController<TimerHandle, MouseEvent>(
  deps: PromptSurfaceMouseControllerDeps<TimerHandle, MouseEvent>,
): PromptSurfaceMouseController<MouseEvent> {
  return {
    handleMouseUp(event) {
      if (!deps.isPrimaryButton(event)) {
        return
      }
      deps.scheduleTimer(() => {
        if (!deps.copySelection() && !deps.hasSelection?.()) deps.retainPromptFocus()
      }, deps.delayMs)
    },
  }
}
