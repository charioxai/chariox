export type TranscriptScrollMonitorDecision = {
  shouldLoadOlderHistory: boolean
  nextLastScrollTop: number
}

type TranscriptScrollMonitorOptions = {
  hasScrollbox: boolean
  pendingHistoryScrollRestore: number
  currentScrollTop: number
  lastTranscriptScrollTop: number
  hasMoreHistory: boolean
  loadingHistory: boolean
}

type ShortViewportHistoryOptions = {
  hasScrollbox: boolean
  attached: boolean
  loadingHistory: boolean
  hasMoreHistory: boolean
  scrollTop: number
  scrollHeight: number
  viewportHeight: number
}

export function evaluateTranscriptScrollMonitor(
  options: TranscriptScrollMonitorOptions,
): TranscriptScrollMonitorDecision {
  if (!options.hasScrollbox || options.pendingHistoryScrollRestore > 0) {
    return {
      shouldLoadOlderHistory: false,
      nextLastScrollTop: options.lastTranscriptScrollTop,
    }
  }

  return {
    shouldLoadOlderHistory:
      options.currentScrollTop === 0
      && options.lastTranscriptScrollTop > 0
      && options.hasMoreHistory
      && !options.loadingHistory,
    nextLastScrollTop: options.currentScrollTop,
  }
}

export function shouldLoadShortViewportHistory(
  options: ShortViewportHistoryOptions,
): boolean {
  if (
    !options.hasScrollbox
    || !options.attached
    || options.loadingHistory
    || !options.hasMoreHistory
  ) {
    return false
  }
  return options.scrollTop === 0 && options.scrollHeight <= options.viewportHeight
}

export function nextWaitingRoomIntroStep(
  attached: boolean,
  introStep: number,
  maxIntroStep = 12,
): number | null {
  if (attached || introStep >= maxIntroStep) {
    return null
  }
  return introStep + 1
}
