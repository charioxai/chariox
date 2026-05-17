export type FooterFlash = {
  message: string
  tone: "info" | "error"
}

type FooterFlashControllerOptions<TimerHandle> = {
  delayMs: number
  scheduleTimer: (callback: () => void, delayMs: number) => TimerHandle
  clearTimer: (timer: TimerHandle) => void
  setFooterFlash: (flash: FooterFlash | null) => void
  onFooterFlashChange: () => void
}

export type FooterFlashController = {
  flash(message: string, tone: FooterFlash["tone"]): void
  clearTimer(): void
}

export function createFooterFlashController<TimerHandle>(
  options: FooterFlashControllerOptions<TimerHandle>,
): FooterFlashController {
  let pendingTimer: TimerHandle | undefined

  const clearPendingTimer = () => {
    if (pendingTimer === undefined) {
      return
    }
    options.clearTimer(pendingTimer)
    pendingTimer = undefined
  }

  return {
    flash(message, tone) {
      clearPendingTimer()
      options.setFooterFlash({ message, tone })
      options.onFooterFlashChange()
      pendingTimer = options.scheduleTimer(() => {
        pendingTimer = undefined
        options.setFooterFlash(null)
        options.onFooterFlashChange()
      }, options.delayMs)
    },
    clearTimer() {
      clearPendingTimer()
    },
  }
}
