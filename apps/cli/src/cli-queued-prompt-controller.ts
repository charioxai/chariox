import { createSignal } from "solid-js"
import { reconcile } from "solid-js/store"

import type {
  RuntimeSession,
  TranscriptEntry,
} from "./cli-types.js"
import {
  cancelQueuedPrompt,
  steerQueuedPrompt,
} from "./prompt-runtime-api.js"
import {
  queuedPromptStripItemsForAgent,
  queuedPromptStripItemToTranscriptEntry,
  syncQueuedPromptTranscriptEntriesByAgentWithPreviews as syncQueuedPromptEntriesByAgent,
  syncQueuedPromptTranscriptEntriesForAgent as syncQueuedPromptEntriesForAgent,
  type QueuedPromptStripItem,
  type QueuedPromptStripStatusOverride,
} from "@arroba/kernel-client/queued-prompt-strip-state"
import {
  nextQueuedPromptSelectionId,
  selectedQueuedPromptIndex,
} from "@arroba/kernel-client/queued-prompt-selection"

type AnyFn = (...args: any[]) => any

export type CliQueuedPromptControllerDeps = Record<string, any> & {
  appendSteeredPrompt: AnyFn
  applyResponseLayout: AnyFn
  applySessionState: AnyFn
  flashFooter: AnyFn
  formatError: AnyFn
  renderAgentInteractions: AnyFn
  replaceTranscriptEntries: AnyFn
  updateSessionChrome: AnyFn
}

export function createCliQueuedPromptController(deps: CliQueuedPromptControllerDeps) {
  const [selectedQueuedPromptIdsByAgent, setSelectedQueuedPromptIdsByAgent] =
    createSignal<Record<string, string>>({})
  const [queuedPromptStatusOverridesByAgent, setQueuedPromptStatusOverridesByAgent] =
    createSignal<Record<string, Record<string, QueuedPromptStripStatusOverride>>>({})

  function syncQueuedPromptsForSession(session: RuntimeSession) {
    let changed = false
    const byAgent = syncQueuedPromptEntriesByAgent(deps.agentPaneEntries(), session)
    if (byAgent.changed) {
      changed = true
      deps.setAgentPaneEntries(reconcile(byAgent.entriesByAgent))
      deps.setAgentPanePreviews((current: Record<string, unknown>) => ({
        ...current,
        ...byAgent.previews,
      }))
    }

    const visibleAgentId = deps.visibleTranscriptAgentId()
    if (!visibleAgentId) {
      if (changed) {
        deps.renderAgentInteractions()
        deps.applyResponseLayout()
      }
      return
    }
    const visibleSync = syncQueuedPromptEntriesForAgent(
      deps.transcriptEntryProjectionController.renderableEntries(),
      session,
      visibleAgentId,
    )
    if (visibleSync.changed) {
      changed = true
      deps.replaceTranscriptEntries(visibleSync.entries, visibleAgentId)
    }
    if (changed) {
      deps.renderAgentInteractions()
      deps.applyResponseLayout()
    }
  }

  function handleQueuedPromptAction(entry: TranscriptEntry, action: "steer" | "cancel") {
    const queuedPrompt = entry.queuedPrompt
    if (!queuedPrompt) {
      return
    }
    if (action === "steer" ? !queuedPrompt.canSteer : !queuedPrompt.canCancel) {
      deps.flashFooter(
        action === "steer"
          ? queuedPrompt.steerDisabledReason ?? "Queued prompt steering is unavailable."
          : queuedPrompt.cancelDisabledReason ?? "Queued prompt cancellation is unavailable.",
        "info",
      )
      return
    }
    const attachment = deps.attachmentState()
    if (!attachment) {
      deps.flashFooter("No session attached.", "error")
      return
    }

    updateQueuedPromptEntryStatus(
      queuedPrompt.agentId,
      queuedPrompt.promptId,
      action === "steer" ? "steering" : "cancelling",
    )
    void (async () => {
      try {
        const payload = action === "steer"
          ? await steerQueuedPrompt(
            deps.client,
            deps.sessionState().id,
            attachment.id,
            queuedPrompt.agentId,
            queuedPrompt.promptId,
          )
          : await cancelQueuedPrompt(
            deps.client,
            deps.sessionState().id,
            attachment.id,
            queuedPrompt.agentId,
            queuedPrompt.promptId,
          )
        if (action === "steer") {
          const promptOrigin = payload.prompt.prompt_origin ?? queuedPrompt.promptOrigin
          deps.appendSteeredPrompt(payload.prompt.prompt, queuedPrompt.agentId, {
            promptId: payload.prompt.id,
            sourceAttachmentId: payload.prompt.source_attachment_id,
            ...(promptOrigin !== undefined ? { promptOrigin } : {}),
          })
        }
        deps.applySessionState(payload.session)
        updateQueuedPromptStatusOverride(queuedPrompt.agentId, queuedPrompt.promptId, "queued")
        deps.updateSessionChrome()
      } catch (error) {
        updateQueuedPromptEntryStatus(queuedPrompt.agentId, queuedPrompt.promptId, "queued")
        deps.flashFooter(deps.formatError(error), "error")
      }
    })()
  }

  function handleQueuedPromptStripAction(item: QueuedPromptStripItem, action: "steer" | "cancel") {
    handleQueuedPromptAction(queuedPromptStripItemToTranscriptEntry(item), action)
  }

  function queuedPromptStripItemsForAgentId(agentId: string | null | undefined): QueuedPromptStripItem[] {
    return queuedPromptStripItemsForAgent(
      deps.sessionState(),
      agentId ? (deps.agentPaneEntries()[agentId] ?? []) : [],
      agentId,
      queuedPromptStatusOverridesForAgent(agentId),
    )
  }

  function selectedQueuedPromptIndexForAgent(agentId: string | null | undefined): number {
    if (!agentId) {
      return -1
    }
    return selectedQueuedPromptIndex(
      queuedPromptStripItemsForAgentId(agentId),
      selectedQueuedPromptIdsByAgent()[agentId],
    )
  }

  function handleQueuedPromptStripKey(event: {
    name?: string
    eventType?: string
    ctrl?: boolean
    meta?: boolean
    alt?: boolean
    shift?: boolean
    preventDefault?: () => void
    stopPropagation?: () => void
  }) {
    if (
      !deps.isAttached()
      || deps.commandCenterOpen()
      || event.eventType === "release"
      || event.ctrl
      || event.meta
      || event.shift
      || !event.alt
    ) {
      return false
    }
    const agentId = deps.focusedAgentId()
    if (!agentId) {
      return false
    }
    const selectionDelta = event.name === "down" || event.name === "j"
      ? 1
      : event.name === "up" || event.name === "k"
        ? -1
        : null
    if (selectionDelta !== null) {
      if (!selectQueuedPromptByDelta(agentId, selectionDelta)) {
        return false
      }
      event.preventDefault?.()
      event.stopPropagation?.()
      return true
    }
    const action = event.name === "s"
      ? "steer"
      : event.name === "c"
        ? "cancel"
        : null
    if (!action) {
      return false
    }
    const items = queuedPromptStripItemsForAgentId(agentId)
    const selectedIndex = selectedQueuedPromptIndex(items, selectedQueuedPromptIdsByAgent()[agentId])
    const item = selectedIndex >= 0 ? items[selectedIndex] : undefined
    if (!item) {
      return false
    }
    if (selectedQueuedPromptIdsByAgent()[agentId] !== item.promptId) {
      setSelectedQueuedPromptIdsByAgent((current) => ({
        ...current,
        [agentId]: item.promptId,
      }))
    }
    event.preventDefault?.()
    event.stopPropagation?.()
    handleQueuedPromptStripAction(item, action)
    return true
  }

  function queuedPromptStatusOverridesForAgent(agentId: string | null | undefined): QueuedPromptStripStatusOverride[] {
    return agentId ? Object.values(queuedPromptStatusOverridesByAgent()[agentId] ?? {}) : []
  }

  function selectQueuedPromptByDelta(agentId: string, delta: number): boolean {
    const items = queuedPromptStripItemsForAgentId(agentId)
    const nextPromptId = nextQueuedPromptSelectionId(items, selectedQueuedPromptIdsByAgent()[agentId], delta)
    if (!nextPromptId) {
      return false
    }
    setSelectedQueuedPromptIdsByAgent((current) => ({
      ...current,
      [agentId]: nextPromptId,
    }))
    deps.renderAgentInteractions()
    deps.applyResponseLayout()
    return true
  }

  function updateQueuedPromptEntryStatus(
    agentId: string,
    promptId: string,
    status: "queued" | "steering" | "cancelling",
  ) {
    updateQueuedPromptStatusOverride(agentId, promptId, status)
    const updateEntries = (currentEntries: TranscriptEntry[]) => currentEntries.map((candidate) => {
      if (candidate.queuedPrompt?.agentId !== agentId || candidate.queuedPrompt.promptId !== promptId) {
        return candidate
      }
      return {
        ...candidate,
        queuedPrompt: {
          ...candidate.queuedPrompt,
          status,
          steerDisabled: candidate.queuedPrompt.steerDisabled,
          canSteer: false,
          canCancel: false,
          steerDisabledReason: "This prompt is no longer waiting in the queue.",
          cancelDisabledReason: "This prompt is no longer waiting in the queue.",
        },
      }
    })
    if (deps.visibleTranscriptAgentId() === agentId) {
      deps.replaceTranscriptEntries(
        updateEntries(deps.transcriptEntryProjectionController.renderableEntries()),
        agentId,
      )
    }
    deps.setAgentTranscriptEntries(agentId, updateEntries(deps.currentAgentPaneEntries(agentId)))
    deps.renderAgentInteractions()
    deps.applyResponseLayout()
  }

  function updateQueuedPromptStatusOverride(
    agentId: string,
    promptId: string,
    status: "queued" | "steering" | "cancelling",
  ) {
    setQueuedPromptStatusOverridesByAgent((current) => {
      const next = { ...current }
      const agentOverrides = { ...(next[agentId] ?? {}) }
      if (status === "queued") {
        delete agentOverrides[promptId]
      } else {
        const pendingReason = status === "steering"
          ? "This prompt is currently being steered."
          : "This prompt is currently being cancelled."
        agentOverrides[promptId] = {
          promptId,
          agentId,
          status,
          steerDisabled: true,
          canSteer: false,
          canCancel: false,
          steerDisabledReason: pendingReason,
          cancelDisabledReason: pendingReason,
        }
      }
      if (Object.keys(agentOverrides).length === 0) {
        delete next[agentId]
      } else {
        next[agentId] = agentOverrides
      }
      return next
    })
  }

  return {
    syncQueuedPromptsForSession,
    handleQueuedPromptAction,
    handleQueuedPromptStripAction,
    queuedPromptStripItemsForAgentId,
    selectedQueuedPromptIndexForAgent,
    handleQueuedPromptStripKey,
  }
}
