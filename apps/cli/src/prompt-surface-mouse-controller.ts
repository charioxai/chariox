export type PromptSurfaceMouseControllerDeps<TimerHandle, MouseEvent> = {
  delayMs: number
  scheduleTimer: (callback: () => void, delayMs: number) => TimerHandle
  isPrimaryButton: (event: MouseEvent) => boolean
  copyText: (text: string) => boolean
  retainPromptFocus: () => void
}

export type PromptSurfaceMouseController<MouseEvent> = {
  handleSelection(text: string): void
  handleMouseUp(event: MouseEvent): void
}

export function createPromptSurfaceMouseController<TimerHandle, MouseEvent extends { isDragging?: boolean }>(
  deps: PromptSurfaceMouseControllerDeps<TimerHandle, MouseEvent>,
): PromptSurfaceMouseController<MouseEvent> {
  let copyPending = false
  return {
    handleSelection(text) {
      // Only a drag released on this surface authorizes automatic copying.
      if (!copyPending) return
      copyPending = false
      // MP-08 / MP-10: the renderer completes selection synchronously before
      // dispatching later edits in this stdin chunk. Retain only its text.
      if (text) deps.scheduleTimer(() => { deps.copyText(text) }, deps.delayMs)
    },
    handleMouseUp(event) {
      if (!deps.isPrimaryButton(event)) {
        return
      }
      copyPending = event.isDragging === true
      // MP-08 / MP-10: focus before the next decoded edit in this chunk.
      // Focus preserves the renderer selection; the following edit clears it.
      deps.retainPromptFocus()
    },
  }
}
