import { computePrependedHistoryScrollTop } from "./history-viewport.js"

export type HistoryScrollRestoreScrollbox = {
  height: number
  scrollHeight: number
  scrollLeft: number
  scrollTop: number
  scrollTo(position: { x: number, y: number }): void
  requestRender(): void
}

type HistoryScrollRestoreControllerOptions = {
  scheduleTimer: (callback: () => void, delayMs: number) => void
  getScrollbox: () => HistoryScrollRestoreScrollbox | null | undefined
  setLastScrollTop: (scrollTop: number) => void
  maxAttempts?: number
  frameDelayMs?: number
}

type RestorePrependedHistoryOptions = {
  scrollbox: HistoryScrollRestoreScrollbox
  previousScrollTop: number
  previousScrollHeight: number
  previousViewportHeight: number
}

export type HistoryScrollRestoreController = {
  cancel(): void
  isRestoring(): boolean
  restorePrependedHistory(options: RestorePrependedHistoryOptions): Promise<void>
}

export function createHistoryScrollRestoreController(
  options: HistoryScrollRestoreControllerOptions,
): HistoryScrollRestoreController {
  const maxAttempts = options.maxAttempts ?? 10
  const frameDelayMs = options.frameDelayMs ?? 16
  let restoreGeneration = 0

  const finishGeneration = (generation: number, resolve: () => void) => {
    if (restoreGeneration === generation) {
      restoreGeneration = 0
    }
    resolve()
  }

  return {
    cancel() {
      restoreGeneration = 0
    },
    isRestoring() {
      return restoreGeneration > 0
    },
    restorePrependedHistory(restoreOptions) {
      const generation = ++restoreGeneration
      return new Promise<void>((resolve) => {
        const restoreScroll = (remainingAttempts: number, lastHeight = -1, stableFrames = 0) => {
          const currentScrollbox = options.getScrollbox()
          if (
            !currentScrollbox
            || currentScrollbox !== restoreOptions.scrollbox
            || generation !== restoreGeneration
          ) {
            finishGeneration(generation, resolve)
            return
          }

          const nextScrollTop = computePrependedHistoryScrollTop(
            restoreOptions.previousScrollTop,
            restoreOptions.previousScrollHeight,
            restoreOptions.scrollbox.scrollHeight,
            restoreOptions.previousViewportHeight,
          )
          restoreOptions.scrollbox.scrollTo({ x: restoreOptions.scrollbox.scrollLeft, y: nextScrollTop })
          restoreOptions.scrollbox.requestRender()
          options.setLastScrollTop(restoreOptions.scrollbox.scrollTop)

          const closeEnough = Math.abs(restoreOptions.scrollbox.scrollTop - nextScrollTop) <= 1
          const nextStableFrames = restoreOptions.scrollbox.scrollHeight === lastHeight ? stableFrames + 1 : 0
          if ((closeEnough && nextStableFrames >= 1) || remainingAttempts <= 1) {
            finishGeneration(generation, resolve)
            return
          }

          options.scheduleTimer(
            () => restoreScroll(remainingAttempts - 1, restoreOptions.scrollbox.scrollHeight, nextStableFrames),
            frameDelayMs,
          )
        }

        restoreOptions.scrollbox.requestRender()
        options.scheduleTimer(() => restoreScroll(maxAttempts), 0)
      })
    },
  }
}
