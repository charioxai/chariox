export type PromptFocusRetentionControllerDeps<TimerHandle> = {
  delayMs: number
  scheduleTimer: (callback: () => void, delayMs: number) => TimerHandle
  isAttached: () => boolean
  focusPromptInput: () => void
}

export type PromptFocusRetentionController = {
  retainFocus(): void
}

export function createPromptFocusRetentionController<TimerHandle>(
  deps: PromptFocusRetentionControllerDeps<TimerHandle>,
): PromptFocusRetentionController {
  return {
    retainFocus() {
      if (!deps.isAttached()) {
        return
      }
      deps.scheduleTimer(() => {
        deps.focusPromptInput()
      }, deps.delayMs)
    },
  }
}
