export type BackgroundPollerStartupLogger = {
  info: (message: string, fields?: Record<string, unknown>) => void
}

export type BackgroundPollerStartupControllerDeps = {
  logger?: BackgroundPollerStartupLogger | null
  ready: () => boolean
  promptMounted: () => boolean
  transcriptScrollTop: () => number
  setLastTranscriptScrollTop: (scrollTop: number) => void
  isAttached: () => boolean
  rebuildTranscript: () => void
  syncPromptPlaceholder: () => void
  focusPrompt: () => void
  blurPrompt: () => void
  addResizeListener: () => void
  removeResizeListener: () => void
  supportsKernelEventStream: () => boolean
  syncKernelEventSubscription: () => Promise<unknown> | unknown
  pollOutput: () => Promise<unknown> | unknown
  pollNotices: () => Promise<unknown> | unknown
  pollSessionState: () => Promise<unknown> | unknown
  startConnectionWatchdog: () => void
  stopConnectionWatchdog: () => void
  logViewDebug: (message: string, fields?: Record<string, unknown>) => void
}

export type BackgroundPollerStartupController = {
  ensureStarted(): void
  stop(): void
  started(): boolean
}

export function createBackgroundPollerStartupController(
  deps: BackgroundPollerStartupControllerDeps,
): BackgroundPollerStartupController {
  let pollersStarted = false

  return {
    ensureStarted() {
      if (pollersStarted) {
        deps.logViewDebug("ensure pollers:already started")
        return
      }
      if (!deps.ready()) {
        deps.logViewDebug("ensure pollers:missing refs", {
          has_prompt_input: deps.promptMounted(),
        })
        return
      }
      pollersStarted = true
      deps.logViewDebug("ensure pollers:starting")
      deps.rebuildTranscript()
      deps.syncPromptPlaceholder()
      if (deps.isAttached()) {
        deps.focusPrompt()
      } else {
        deps.blurPrompt()
      }
      deps.setLastTranscriptScrollTop(deps.transcriptScrollTop())
      deps.addResizeListener()
      if (deps.supportsKernelEventStream()) {
        deps.logger?.info("starting kernel event stream")
        void deps.syncKernelEventSubscription()
      } else {
        deps.logger?.info("starting background pollers")
        void deps.pollOutput()
        void deps.pollNotices()
        void deps.pollSessionState()
      }
      deps.startConnectionWatchdog()
    },

    stop() {
      deps.stopConnectionWatchdog()
      if (pollersStarted) {
        deps.removeResizeListener()
      }
    },

    started() {
      return pollersStarted
    },
  }
}
