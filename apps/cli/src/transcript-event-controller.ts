import type { AgentInstance, TranscriptEntry } from "./cli-types.js"
import { compactTranscriptDisplayEntries } from "@arroba/kernel-client/transcript-display-state"
import {
  createTranscriptSteeredPromptEntry,
  createTranscriptUserPromptTurn,
  computeNextTranscriptTurnId as computeNextTurnId,
} from "@arroba/kernel-client/transcript-entry-state"

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
      const promptTurn = createTranscriptUserPromptTurn(text, computeNextTurnId(paneEntries))
      const nextTurnIds = deps.collapseLatestTurnForAgent(targetAgentId, paneEntries)
      deps.appendTranscriptEntryToAgentPane(targetAgentId, promptTurn.entry, nextTurnIds)
      setTurnWorkActive(deps)
      return
    }

    const promptTurn = createTranscriptUserPromptTurn(text, deps.nextTurnId())
    deps.setNextTurnId(promptTurn.nextTurnId)
    deps.setCurrentTurnId(promptTurn.currentTurnId)
    const nextTurnIds = deps.collapseLatestTurnForAgent(
      targetAgentId,
      compactTranscriptDisplayEntries(deps.entries()),
    )
    deps.appendEntry(promptTurn.entry, nextTurnIds)
    deps.syncVisibleTranscriptPreview()
    setTurnWorkActive(deps)
    deps.scrollTranscriptToBottom()
  }

  const appendSteeredPrompt = (
    text: string,
    agentId: string,
    metadata: { promptId?: string | null; sourceAttachmentId?: string | null } = {},
  ) => {
    const entry = createTranscriptSteeredPromptEntry(text, metadata)
    if (!entry) {
      return
    }
    deps.recordTurnActivity("queued_prompt_steer")
    deps.setStreamingAgentId(agentId)
    deps.markAgentBusy(agentId)

    if (
      deps.splitAgentResponseMode()
      && agentId
      && agentId !== deps.responsePrimaryAgent()?.id
    ) {
      deps.appendTranscriptEntryToAgentPane(agentId, entry)
      deps.updateSessionChrome()
      return
    }

    deps.appendEntry(entry)
    deps.syncVisibleTranscriptPreview()
    deps.updateSessionChrome()
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
    appendSteeredPrompt,
    appendUserPrompt,
  }
}

function setTurnWorkActive(deps: Pick<TranscriptEventControllerDeps, "setSubmitting" | "setWorking" | "renderSessionChromeBoundary">) {
  deps.setSubmitting(true)
  deps.setWorking(true)
  deps.renderSessionChromeBoundary()
}
