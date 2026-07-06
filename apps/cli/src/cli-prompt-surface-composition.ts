import { createEffect, untrack } from "solid-js"

import {
  ATTACHED_PROMPT_PLACEHOLDER,
} from "./cli-runtime-tuning.js"
import { createPromptAttachmentController } from "./prompt-attachment-controller.js"
import {
  createPromptAttachmentHighlightController,
} from "./prompt-attachment-highlight-controller.js"
import {
  promptAttachmentTokenKind,
  promptAttachmentTokenStyleIds,
} from "./prompt-attachment-tokens.js"
import {
  createPromptContentChangeController,
} from "./prompt-content-change-controller.js"
import {
  createPromptDraftPersistController,
} from "./prompt-draft-persist-controller.js"
import { createPromptFocusRetentionController } from "./prompt-focus-retention-controller.js"
import { createPromptHistoryAttachmentController } from "./prompt-history-attachment-controller.js"
import {
  createPromptHistoryNavigationController,
} from "./prompt-history-navigation-controller.js"
import { createPromptHistoryRestoreController } from "./prompt-history-restore-controller.js"
import {
  createPromptInputHistoryRefreshController,
} from "./prompt-input-history-refresh-controller.js"
import { createPromptInputHistoryController } from "./prompt-input-history-controller.js"
import { createPromptHistoryHydrationController } from "./prompt-history-hydration-controller.js"
import { createPromptChromeProjectionController } from "./prompt-chrome-projection-controller.js"
import { createPromptSessionHistoryController } from "./prompt-session-history-controller.js"
import { createPromptSessionStatePersistenceController } from "./prompt-session-state-persistence-controller.js"
import {
  createPromptPlaceholderSyncController,
} from "./prompt-surface-state.js"
import {
  derivePromptInputMaxHeight,
} from "@arroba/kernel-client/prompt-surface-state"
import {
  createPromptSubmissionUiController,
} from "./prompt-submission-ui-controller.js"
import { saveSessionPromptState } from "./preferences.js"
import {
  getPromptInputHistory,
  recordPromptInputHistory,
} from "./session-history-api.js"
import {
  SESSION_NEW_PLACEHOLDER,
} from "./sessions.js"
import { theme } from "./theme.js"

type AnyFn = (...args: any[]) => any

export type CliPromptSurfaceCompositionDeps = {
  client: any
  appLogger: any
  formatError: AnyFn
  scheduleTimer: AnyFn
  clearTimer: AnyFn
  daemonDisconnected: AnyFn
  working: AnyFn
  anyPromptWork: AnyFn
  anyTurnWork: AnyFn
  submitting: AnyFn
  focusedQueueDepth: AnyFn
  fatalError: AnyFn
  focusedActivePrompt: AnyFn
  statusLine: AnyFn
  isAttached: AnyFn
  workflowScreenShowing: AnyFn
  workflowPromptState: AnyFn
  themeRevision: AnyFn
  preferencesState: AnyFn
  setPreferencesState: AnyFn
  setPromptHistoryEntries: AnyFn
  setPromptHistoryIndex: AnyFn
  setPromptHistoryDraft: AnyFn
  promptTextController: {
    clear: AnyFn
    currentText: AnyFn
    isProgrammaticMutation: AnyFn
    setSnapshot: AnyFn
    setText: AnyFn
    snapshot: AnyFn
    syncSnapshot: AnyFn
  }
  attachmentState: AnyFn
  promptHistoryEntries: AnyFn
  promptHistoryIndex: AnyFn
  promptHistoryDraft: AnyFn
  promptInputRefController: {
    currentOrNull: AnyFn
    focus: AnyFn
    hasInput: AnyFn
  }
  pendingAttachments: AnyFn
  setPendingAttachments: AnyFn
  terminalHeight: AnyFn
  requestRender: AnyFn
  updateSessionChrome: AnyFn
  syncCommandCenter: AnyFn
  clearCommandCenter: AnyFn
  attachPromptFiles: AnyFn
  getCwd: AnyFn
  flashFooter: AnyFn
}

export function createCliPromptSurfaceComposition(deps: CliPromptSurfaceCompositionDeps) {
  const promptChromeProjectionController = createPromptChromeProjectionController({
    daemonDisconnected: deps.daemonDisconnected,
    working: deps.working,
    hasActiveTurnWork: deps.anyTurnWork,
    submitting: deps.submitting,
    queueDepth: deps.focusedQueueDepth,
    fatalError: deps.fatalError,
    activePromptId: () => deps.focusedActivePrompt()?.id ?? null,
    statusLine: deps.statusLine,
    isAttached: deps.isAttached,
    workflowScreenActive: deps.workflowScreenShowing,
    workflowPromptState: deps.workflowPromptState,
    attachedPlaceholder: ATTACHED_PROMPT_PLACEHOLDER,
    detachedPlaceholder: SESSION_NEW_PLACEHOLDER,
    trackThemeRevision: () => deps.themeRevision(),
    attachedBackground: () => theme.backgroundPanel,
    detachedBackground: () => theme.backgroundElement,
    workflowBackground: () => theme.backgroundElement,
  })
  const promptPlaceholder = promptChromeProjectionController.promptPlaceholder

  const promptHistoryRestoreController = createPromptHistoryRestoreController({
    getPreferences: () => untrack(deps.preferencesState),
    setPromptHistoryEntries: deps.setPromptHistoryEntries,
    resetPromptHistoryNavigation: () => {
      deps.setPromptHistoryIndex(null)
      deps.setPromptHistoryDraft(null)
    },
    setPromptText: (text) => {
      setPromptText(text)
    },
  })
  const restorePromptHistory = promptHistoryRestoreController.restore

  const promptSessionStatePersistenceController = createPromptSessionStatePersistenceController({
    updatePreferences: (updater) => {
      deps.setPreferencesState((current: any) => updater(current))
    },
    savePromptState: saveSessionPromptState,
  })
  const persistSessionPromptState = promptSessionStatePersistenceController.persist

  const promptDraftPersistController = createPromptDraftPersistController({
    delayMs: 300,
    scheduleTimer: deps.scheduleTimer,
    clearTimer: deps.clearTimer,
    persistPromptDraft: ({ sessionId, promptDraft }) =>
      persistSessionPromptState(sessionId, { promptDraft }),
    onPersistError: (error, request) => {
      deps.appLogger?.warn("failed to persist prompt draft", {
        session_id: request.sessionId,
        error: deps.formatError(error),
      })
    },
  })
  const clearPendingPromptDraftPersist = promptDraftPersistController.clearTimer
  const flushPendingPromptDraftPersist = promptDraftPersistController.flush
  const schedulePromptDraftPersist = promptDraftPersistController.schedule
  const clearPromptDraftPersistQueue = promptDraftPersistController.clearPending

  const promptInputHistoryController = createPromptInputHistoryController({
    getCurrentSessionId: () => deps.attachmentState()?.session_id ?? null,
    getAttachmentId: () => deps.attachmentState()?.id ?? null,
    getEntries: deps.promptHistoryEntries,
    setEntries: deps.setPromptHistoryEntries,
    resetNavigation: () => {
      deps.setPromptHistoryIndex(null)
      deps.setPromptHistoryDraft(null)
    },
    clearDraftPersistQueue: clearPromptDraftPersistQueue,
    persistPromptState: persistSessionPromptState,
    recordPromptInputHistory: (sessionId, attachmentId, kind, text) =>
      recordPromptInputHistory(deps.client, sessionId, attachmentId, kind, text),
    onSharedHistoryPersistFailed: (sessionId, error) => {
      deps.appLogger?.warn("failed to persist shared prompt input history", {
        session_id: sessionId,
        error: deps.formatError(error),
      })
    },
    onPromptEchoPersistFailed: (sessionId, error) => {
      deps.appLogger?.warn("failed to persist prompt echo history", {
        session_id: sessionId,
        error: deps.formatError(error),
      })
    },
    onPromptStatePersistFailed: (sessionId, error) => {
      deps.appLogger?.warn("failed to persist session prompt state", {
        session_id: sessionId,
        error: deps.formatError(error),
      })
    },
    onRecordSharedHistoryFailed: (sessionId, error) => {
      deps.appLogger?.warn("failed to record shared prompt input history", {
        session_id: sessionId,
        error: deps.formatError(error),
      })
    },
  })

  const promptHistoryHydrationController = createPromptHistoryHydrationController({
    loadHistory: (sessionId) => getPromptInputHistory(deps.client, sessionId),
    isCurrentSession: (sessionId) => deps.attachmentState()?.session_id === sessionId,
    applyHistory: async (sessionId, nextEntries, latestSequence) => {
      await promptInputHistoryController.replaceFromHydration(sessionId, nextEntries, latestSequence)
    },
  })
  const hydratePromptHistoryFromSession = (sessionId: string): Promise<void> =>
    promptHistoryHydrationController.hydrate(sessionId)
  const appendSharedPromptInputHistory = promptInputHistoryController.appendShared
  const appendPromptEchoToSharedHistory = promptInputHistoryController.appendEcho

  const promptInputHistoryRefreshController = createPromptInputHistoryRefreshController({
    delayMs: 1500,
    scheduleTimer: deps.scheduleTimer,
    clearTimer: deps.clearTimer,
    refreshHistory: async (sessionId) => {
      const history = await getPromptInputHistory(deps.client, sessionId, promptInputHistoryController.latestSequence(), 500)
      appendSharedPromptInputHistory(sessionId, history.entries)
    },
    onRefreshError: (error, sessionId) => {
      deps.appLogger?.warn("failed to refresh shared prompt input history", {
        session_id: sessionId,
        error: deps.formatError(error),
      })
    },
  })

  const promptSessionHistoryController = createPromptSessionHistoryController({
    currentSessionId: () => deps.attachmentState()?.session_id ?? null,
    navigationDraft: deps.promptHistoryDraft,
    currentPromptText: deps.promptTextController.currentText,
    scheduleHistoryRefresh: promptInputHistoryRefreshController.schedule,
  })
  const scheduleSharedPromptInputHistoryRefresh = promptSessionHistoryController.scheduleSharedRefresh
  const persistablePromptDraft = promptSessionHistoryController.persistableDraft
  const recordPromptAreaHistoryEntry = promptInputHistoryController.recordPromptAreaEntry
  const syncPromptTextSnapshot = deps.promptTextController.syncSnapshot

  const promptAttachmentHighlightController = createPromptAttachmentHighlightController({
    getPromptInput: deps.promptInputRefController.currentOrNull,
    getPendingAttachments: deps.pendingAttachments,
    styleIdForKind: (kind) => promptAttachmentTokenStyleIds[promptAttachmentTokenKind(kind)],
  })
  const refreshPromptAttachmentHighlights = promptAttachmentHighlightController.refresh
  const setPromptText = deps.promptTextController.setText
  const promptInputMaxHeight = () => derivePromptInputMaxHeight({
    attached: deps.isAttached(),
    terminalHeight: deps.terminalHeight(),
  })

  const promptFocusRetentionController = createPromptFocusRetentionController({
    delayMs: 0,
    scheduleTimer: deps.scheduleTimer,
    isAttached: deps.isAttached,
    focusPromptInput: () => {
      deps.promptInputRefController.focus()
    },
  })
  const retainPromptFocus = promptFocusRetentionController.retainFocus

  const promptHistoryNavigationController = createPromptHistoryNavigationController({
    getPromptText: deps.promptTextController.currentText,
    getEntries: deps.promptHistoryEntries,
    getNavigationIndex: deps.promptHistoryIndex,
    getNavigationDraft: deps.promptHistoryDraft,
    setNavigationIndex: deps.setPromptHistoryIndex,
    setNavigationDraft: deps.setPromptHistoryDraft,
    setPromptText,
    getSessionId: () => deps.attachmentState()?.session_id ?? null,
    schedulePromptDraftPersist,
    retainPromptFocus,
  })
  const navigatePromptHistoryInput = promptHistoryNavigationController.navigate

  const promptPlaceholderSyncController = createPromptPlaceholderSyncController({
    getPromptInput: deps.promptInputRefController.currentOrNull,
    getPlaceholder: promptPlaceholder,
  })
  const syncPromptPlaceholder = promptPlaceholderSyncController.sync
  createEffect(() => {
    promptPlaceholder()
    syncPromptPlaceholder()
  })

  const promptAttachmentController = createPromptAttachmentController({
    getPromptInput: deps.promptInputRefController.currentOrNull,
    getPromptText: deps.promptTextController.currentText,
    setPromptText,
    pendingAttachments: deps.pendingAttachments,
    setPendingAttachments: (attachments) => deps.setPendingAttachments(attachments),
    updatePendingAttachments: (updater) => deps.setPendingAttachments((current: any) => updater(current)),
    refreshHighlights: refreshPromptAttachmentHighlights,
    updateSessionChrome: () => deps.updateSessionChrome(),
    requestRender: deps.requestRender,
  })
  const clearPendingPromptAttachments = promptAttachmentController.clear
  const syncPendingPromptAttachmentsFromText = promptAttachmentController.syncFromText
  const removeLastPendingPromptAttachment = promptAttachmentController.removeLast
  const addPendingPromptAttachments = promptAttachmentController.addStoredFiles
  const removePromptAttachmentsForEdit = promptAttachmentController.removeForEdit

  const promptSubmissionUiController = createPromptSubmissionUiController({
    getSessionId: () => deps.attachmentState()?.session_id ?? null,
    getPendingAttachments: deps.pendingAttachments,
    resetPromptHistoryNavigation: () => {
      deps.setPromptHistoryIndex(null)
      deps.setPromptHistoryDraft(null)
    },
    clearDraftPersistQueue: clearPromptDraftPersistQueue,
    clearPromptText: () => {
      deps.promptTextController.clear()
    },
    setPromptText,
    syncPromptTextSnapshot,
    clearPendingAttachments: clearPendingPromptAttachments,
    setPendingAttachments: (attachments) => deps.setPendingAttachments(attachments),
    refreshAttachmentHighlights: refreshPromptAttachmentHighlights,
    syncCommandCenter: deps.syncCommandCenter,
    retainPromptFocus,
    clearCommandCenter: deps.clearCommandCenter,
    schedulePromptDraftPersist,
    updateSessionChrome: () => deps.updateSessionChrome(),
  })
  const beginSubmittedPromptUi = promptSubmissionUiController.begin
  const restoreFailedPromptUi = promptSubmissionUiController.restore

  const promptHistoryAttachmentController = createPromptHistoryAttachmentController({
    getAttachedSessionId: () => deps.attachmentState()?.session_id ?? null,
    restorePromptHistory,
    invalidateHydration: promptHistoryHydrationController.invalidate,
    hydratePromptHistory: hydratePromptHistoryFromSession,
    isCurrentSession: (sessionId) => deps.attachmentState()?.session_id === sessionId,
    warnHydrationError: (sessionId, error) => {
      deps.appLogger?.warn("failed to hydrate prompt history from session history", {
        session_id: sessionId,
        error: deps.formatError(error),
      })
    },
  })
  createEffect(() => {
    void promptHistoryAttachmentController.sync()
  })

  const promptContentChangeController = createPromptContentChangeController({
    getPromptText: () => deps.promptInputRefController.hasInput() ? deps.promptTextController.currentText() : null,
    isAttached: deps.isAttached,
    getPreviousSnapshot: deps.promptTextController.snapshot,
    isProgrammaticMutation: deps.promptTextController.isProgrammaticMutation,
    isPromptHistoryActive: () => deps.promptHistoryIndex() !== null || deps.promptHistoryDraft() !== null,
    getSessionId: () => deps.attachmentState()?.session_id,
    getCwd: deps.getCwd,
    setPromptTextSnapshot: deps.promptTextController.setSnapshot,
    resetPromptHistory: (draft) => {
      deps.setPromptHistoryIndex(null)
      deps.setPromptHistoryDraft(draft)
    },
    syncPendingAttachmentsFromText: syncPendingPromptAttachmentsFromText,
    setPromptText,
    syncCommandCenter: deps.syncCommandCenter,
    schedulePromptDraftPersist,
    attachPromptFiles: deps.attachPromptFiles,
    onDropFailed: (error, files) => {
      deps.appLogger?.warn("prompt attachment drop failed", {
        error: deps.formatError(error),
        paths: files.map((file: { path: string }) => file.path),
      })
      deps.flashFooter(`failed to attach files: ${deps.formatError(error)}`, "error")
    },
  })

  return {
    addPendingPromptAttachments,
    appendPromptEchoToSharedHistory,
    beginSubmittedPromptUi,
    clearPendingPromptAttachments,
    clearPendingPromptDraftPersist,
    flushPendingPromptDraftPersist,
    footerHint: promptChromeProjectionController.footerHint,
    handlePromptContentChange: promptContentChangeController.handleChange,
    navigatePromptHistoryInput,
    persistablePromptDraft,
    persistSessionPromptState,
    promptAreaBackground: promptChromeProjectionController.promptAreaBackground,
    promptHistoryHydrationController,
    promptInputHistoryRefreshController,
    promptInputMaxHeight,
    promptPlaceholder,
    recordPromptAreaHistoryEntry,
    refreshPromptAttachmentHighlights,
    removeLastPendingPromptAttachment,
    removePromptAttachmentsForEdit,
    restoreFailedPromptUi,
    retainPromptFocus,
    schedulePromptDraftPersist,
    scheduleSharedPromptInputHistoryRefresh,
    sessionStatusMode: promptChromeProjectionController.sessionStatusMode,
    setPromptText,
    syncPendingPromptAttachmentsFromText,
    syncPromptPlaceholder,
    syncPromptTextSnapshot,
  }
}
