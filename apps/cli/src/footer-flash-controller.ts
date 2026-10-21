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
  /** Drops the visible flash when it matches; a later unrelated flash stays. */
  clearIf(matches: (flash: FooterFlash) => boolean): void
  clearTimer(): void
}

export function createFooterFlashController<TimerHandle>(
  options: FooterFlashControllerOptions<TimerHandle>,
): FooterFlashController {
  let pendingTimer: TimerHandle | undefined
  let current: FooterFlash | null = null

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
      current = { message, tone, ...(display ? { display } : {}) }
      options.setFooterFlash(current)
      options.onFooterFlashChange()
      pendingTimer = options.scheduleTimer(() => {
        pendingTimer = undefined
        current = null
        options.setFooterFlash(null)
        options.onFooterFlashChange()
      }, display === "toast" ? 30_000 : options.delayMs)
    },
    clearIf(matches) {
      if (!current || !matches(current)) return
      clearPendingTimer()
      current = null
      options.setFooterFlash(null)
      options.onFooterFlashChange()
    },
    clearTimer() {
      clearPendingTimer()
    },
  }
}
