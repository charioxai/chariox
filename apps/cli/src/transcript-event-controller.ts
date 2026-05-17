import type { AgentInstance, TranscriptEntry } from "./cli-types.js"
import { computeNextTurnId } from "./transcript-preview.js"
import { trimSingleTrailingNewline } from "./transcript-text.js"

export type TranscriptEventControllerDeps = {
  recordTurnActivity: (activityType: string) => void
  resetTurnCompletion: () => void
  cancelPendingTurnCompletion: () => void
  focusedAgentId: () => string | null
  visibleTranscriptAgentId: () => string | null | undefined
  responsePrimaryAgent: () => AgentInstance | null
  splitAgentResponseMode: () => boolean
  isAttached: () => boolean
  entries: () => TranscriptEntry[]
  nextTurnId: () => number
  setNextTurnId: (turnId: number) => void
  setCurrentTurnId: (turnId: number | null) => void
  setSubmittingAgentId: (agentId: string | null) => void
  setStreamingAgentId: (agentId: string | null) => void
  markAgentBusy: (agentId: string | null | undefined) => void
  clearAgentBusy: (agentId: string | null | undefined) => void
  currentAgentPaneEntries: (agentId: string) => TranscriptEntry[]
  collapseLatestTurnForAgent: (agentId: string | null | undefined, paneEntries: TranscriptEntry[]) => readonly number[]
  appendTranscriptEntryToAgentPane: (
    agentId: string,
    entry: Omit<TranscriptEntry, "id">,
    turnIds?: readonly number[],
  ) => void
  appendEntry: (entry: Omit<TranscriptEntry, "id">, turnIds?: readonly number[]) => unknown
  setSubmitting: (value: boolean) => void
  setWorking: (value: boolean) => void
  renderSessionChromeBoundary: () => void
  syncVisibleTranscriptPreview: () => void
  scrollTranscriptToBottom: () => void
  updateSessionChrome: () => void
  setWaitingRoomCloudNotice: (text: string) => void
  rebuildTranscript: () => void
}

export function createTranscriptEventController(deps: TranscriptEventControllerDeps) {
  const appendUserPrompt = (text: string, agentId?: string | null) => {
    deps.recordTurnActivity("prompt_submit")
    deps.resetTurnCompletion()

    const targetAgentId = agentId ?? deps.focusedAgentId()
    deps.setSubmittingAgentId(targetAgentId)
    deps.setStreamingAgentId(targetAgentId)
    deps.markAgentBusy(targetAgentId)

    if (
      deps.splitAgentResponseMode()
      && targetAgentId
      && targetAgentId !== deps.responsePrimaryAgent()?.id
    ) {
      const paneEntries = deps.currentAgentPaneEntries(targetAgentId)
      const nextTurnIds = deps.collapseLatestTurnForAgent(targetAgentId, paneEntries)
      deps.appendTranscriptEntryToAgentPane(targetAgentId, {
        role: "user",
        text: trimSingleTrailingNewline(text),
        turnId: computeNextTurnId(paneEntries),
      }, nextTurnIds)
      setPromptWorkActive(deps)
      return
    }

    const turnId = deps.nextTurnId()
    deps.setNextTurnId(turnId + 1)
    deps.setCurrentTurnId(turnId)
    const nextTurnIds = deps.collapseLatestTurnForAgent(targetAgentId, deps.entries().filter(Boolean))
    deps.appendEntry({ role: "user", text: trimSingleTrailingNewline(text), turnId }, nextTurnIds)
    deps.syncVisibleTranscriptPreview()
    setPromptWorkActive(deps)
    deps.scrollTranscriptToBottom()
  }

  const appendNotice = (text: string, emphasis: TranscriptEntry["emphasis"] = "muted") => {
    deps.appendEntry({ role: "notice", text, emphasis })
    deps.syncVisibleTranscriptPreview()
    deps.updateSessionChrome()
  }

  const appendCloudNotice = (text: string) => {
    if (deps.isAttached()) {
      appendNotice(text)
      return
    }
    deps.setWaitingRoomCloudNotice(text)
    deps.rebuildTranscript()
    deps.updateSessionChrome()
  }

  const appendProviderError = (text: string) => {
    const normalized = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim()
    if (!normalized) {
      return
    }
    deps.cancelPendingTurnCompletion()
    deps.setWorking(false)
    deps.setSubmitting(false)
    deps.clearAgentBusy(deps.visibleTranscriptAgentId())
    deps.setSubmittingAgentId(null)
    deps.appendEntry({ role: "error", text: normalized, emphasis: "error" })
    deps.syncVisibleTranscriptPreview()
    deps.renderSessionChromeBoundary()
    deps.scrollTranscriptToBottom()
  }

  return {
    appendCloudNotice,
    appendNotice,
    appendProviderError,
    appendUserPrompt,
  }
}

function setPromptWorkActive(deps: Pick<TranscriptEventControllerDeps, "setSubmitting" | "setWorking" | "renderSessionChromeBoundary">) {
  deps.setSubmitting(true)
  deps.setWorking(true)
  deps.renderSessionChromeBoundary()
}
