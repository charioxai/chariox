export type FooterFlash = {
  message: string
  tone: "info" | "error"
  display?: "toast"
}

type FooterFlashControllerOptions<TimerHandle> = {
  delayMs: number
  scheduleTimer: (callback: () => void, delayMs: number) => TimerHandle
  clearTimer: (timer: TimerHandle) => void
  setFooterFlash: (flash: FooterFlash | null) => void
  onFooterFlashChange: () => void
}

export type FooterFlashController = {
  flash(message: string, tone: FooterFlash["tone"], display?: "toast"): void
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
    flash(message, tone, display) {
      clearPendingTimer()
      options.setFooterFlash({ message, tone, ...(display ? { display } : {}) })
      options.onFooterFlashChange()
      pendingTimer = options.scheduleTimer(() => {
        pendingTimer = undefined
        options.setFooterFlash(null)
        options.onFooterFlashChange()
      }, display === "toast" ? 30_000 : options.delayMs)
    },
    clearTimer() {
      clearPendingTimer()
    },
  }
}
