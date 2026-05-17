import {
  evaluateTranscriptScrollMonitor,
  shouldLoadShortViewportHistory,
} from "./background-effects.js"

export type TranscriptHistoryAutoloadScrollbox = {
  height: number
  scrollHeight: number
  scrollTop: number
}

type TranscriptHistoryAutoloadControllerOptions = {
  scheduleTimer: (callback: () => void, delayMs: number) => void
  getScrollbox: () => TranscriptHistoryAutoloadScrollbox | null | undefined
  isScrollRestoring: () => boolean
  isAttached: () => boolean
  isLoadingHistory: () => boolean
  hasMoreHistory: () => boolean
  getLastScrollTop: () => number
  setLastScrollTop: (scrollTop: number) => void
  loadOlderHistory: () => Promise<boolean | void> | boolean | void
}

export type TranscriptHistoryAutoloadController = {
  monitorScroll(): void
  maybeLoadForShortViewport(): void
  scheduleShortViewportCheck(): void
}

export function createTranscriptHistoryAutoloadController(
  options: TranscriptHistoryAutoloadControllerOptions,
): TranscriptHistoryAutoloadController {
  let controller: TranscriptHistoryAutoloadController
  const requestLoad = () => {
    void Promise.resolve(options.loadOlderHistory()).then((loaded) => {
      if (loaded === true) {
        controller.scheduleShortViewportCheck()
      }
    })
  }

  controller = {
    monitorScroll() {
      const scrollbox = options.getScrollbox()
      const decision = evaluateTranscriptScrollMonitor({
        hasScrollbox: Boolean(scrollbox),
        pendingHistoryScrollRestore: options.isScrollRestoring() ? 1 : 0,
        currentScrollTop: scrollbox?.scrollTop ?? 0,
        lastTranscriptScrollTop: options.getLastScrollTop(),
        hasMoreHistory: options.hasMoreHistory(),
        loadingHistory: options.isLoadingHistory(),
      })
      if (decision.shouldLoadOlderHistory) {
        requestLoad()
      }
      options.setLastScrollTop(decision.nextLastScrollTop)
    },
    maybeLoadForShortViewport() {
      const scrollbox = options.getScrollbox()
      if (shouldLoadShortViewportHistory({
        hasScrollbox: Boolean(scrollbox),
        attached: options.isAttached(),
        loadingHistory: options.isLoadingHistory(),
        hasMoreHistory: options.hasMoreHistory(),
        scrollTop: scrollbox?.scrollTop ?? 0,
        scrollHeight: scrollbox?.scrollHeight ?? 0,
        viewportHeight: scrollbox?.height ?? 0,
      })) {
        requestLoad()
      }
    },
    scheduleShortViewportCheck() {
      options.scheduleTimer(() => {
        controller.maybeLoadForShortViewport()
      }, 0)
    },
  }

  return controller
}
