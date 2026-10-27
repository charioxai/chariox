import type {
  CliOptions,
  RuntimeAttachment,
  RuntimeProviderRun,
  RuntimeSession,
  SessionHistoryCursorState,
} from "./cli-types.js"
import type {
  AttachedCliTransitionState,
  DetachedCliTransitionState,
} from "./session-state.js"
import { sessionResponseLayout } from "@chariox/kernel-client/session-config-projection"
import {
  isCompleteSessionSnapshot,
  sessionListEntryFromSession,
  upsertSessionListEntry,
  type SessionLifecycleLaunchSelection,
} from "@chariox/kernel-client/session-lifecycle-state"
import { settleAttachProviderRun } from "./attach-provider-run.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import type { SessionListEntry } from "./sessions.js"

type ProviderCatalog = Record<string, unknown>
type TerminalCommandCatalog = Record<string, unknown>
type LaunchSelection = SessionLifecycleLaunchSelection

type ProviderSelectionState = {
  provider: string
  model: string
  effort: string
}

type SessionLifecycleDeps = {
  cliOptions: CliOptions
  connectedStatus: string
  waitingRoomState: () => WaitingRoomState
  attachmentState: () => RuntimeAttachment | null
  sessionState: () => RuntimeSession
  deriveDetachedCliTransitionState: (options: {
    cliOptions: CliOptions
    waitingRoomState: WaitingRoomState
    message: string
  }) => DetachedCliTransitionState
  deriveAttachedCliTransitionState: (options: {
    session: RuntimeSession
    createdSession: boolean
    connectedStatus: string
  }) => AttachedCliTransitionState
  clearPendingPromptAttachments: () => void
  clearActiveToolLabels: () => void
  clearWorkflows: () => void
  clearAgentPaneRuntime: () => void
  clearDirectoryTree: () => void
  clearTranscript: () => void
  refreshResponseLayout: () => void
  resetWorkspaceScreen: () => void
  resetStopRequestInFlight: () => void
  bumpHistoryLoadGeneration: () => void
  reconcileWaitingRoom: (state: WaitingRoomState) => void
  refreshWaitingRoomData: () => Promise<void>
  requestRender: () => void
  clearPromptInput: () => void
  syncPromptTextSnapshot: () => void
  blurPromptInput: () => void
  focusPromptInput: () => void
  layoutPreference?: () => MultiAgentResponseLayout | null | undefined
  setMultiAgentResponseLayout: (layout: MultiAgentResponseLayout) => void
  setAttachmentState: (attachment: RuntimeAttachment | null) => void
  setProviderRunState: (run: RuntimeProviderRun | null) => void
  setCenterMode: (mode: "transcript") => void
  setCreatedSessionState: (value: boolean) => void
  setSessionState: (session: RuntimeSession) => void
  setProviderActivityLabel: (value: string | null) => void
  setActiveStatusLabel: (value: string | null) => void
  setAgentPaneEntries: (value: Record<string, never[]>) => void
  setAgentPanePreviews: (value: Record<string, string>) => void
  setAgentActivityLabels: (value: Record<string, string | null>) => void
  setStreamingAgentId: (value: string | null) => void
  setSubmitting: (value: boolean) => void
  setWorking: (value: boolean) => void
  setFatalError: (value: string | null) => void
  setDaemonDisconnected: (value: boolean) => void
  setNextHistoryCursor: (value: SessionHistoryCursorState) => void
  setSessionHydratingState: (value: boolean) => void
  setHistoryLoadingState: (value: boolean) => void
  setStatusLine: (value: string) => void
  updateSessionChrome: () => void
  refreshSplitPaneFocusRepaint?: () => void
  attachToSession: (sessionId: string, clientId: string) => Promise<RuntimeAttachment>
  getSessionState: (sessionId: string) => Promise<RuntimeSession>
  launchProviderRun: (
    sessionId: string,
    provider: string,
    accountProfile: string,
    model: string,
    effort: string,
    targetAgentId?: string | null,
  ) => Promise<RuntimeProviderRun>
  tryGetProviderRun: (
    providerRunId: string,
  ) => Promise<RuntimeProviderRun | null>
  syncCliProviderSelection: (selection: ProviderSelectionState) => void
  primeAttachedSessionBinding?: (session: RuntimeSession, owned: () => boolean) => Promise<void>
  hydrateAttachedSessionBinding: (
    sessionId: string,
    attachmentId: string,
    session: RuntimeSession,
  ) => Promise<RuntimeSession>
  getAvailableSessions?: () => SessionListEntry[]
  setAvailableSessions: (sessions: SessionListEntry[]) => void
  listSessions: () => Promise<RuntimeSession[]>
  scheduleShortViewportHistoryCheck: () => void
  detachAttachment: (attachmentId: string) => Promise<void>
  syncKernelEventSubscription?: () => Promise<void>
  formatError?: (error: unknown) => string
  logWarning?: (message: string, fields?: Record<string, unknown>) => void
  logAttachedProviderRun?: (
    mode: "launched" | "loaded",
    run: RuntimeProviderRun | null,
    fields: Record<string, unknown>,
  ) => void
}

type SessionReference = Pick<RuntimeSession, "id"> & Partial<RuntimeSession>

// MP-08 / MP-10 OWNER-24: session selection is one state machine. Starting a
// selection takes the view and the selection guard at once and supersedes every
// older selection. Each async step of a selection applies only while its
// generation is current, and kernel attachment changes run one selection at a time.
type SelectionState =
  | { kind: "idle" }
  | { kind: "switching" | "restoring"; generation: number }

export type SessionSelection = {
  current: () => boolean
  attach: (session: SessionReference, createdSession?: boolean, launch?: LaunchSelection) => Promise<void>
  settle: () => Promise<void>
}

export function createSessionLifecycleController(deps: SessionLifecycleDeps) {
  let selection: SelectionState = { kind: "idle" }
  let selectionGeneration = 0
  // The previous attachment, moved aside while a selection loads: shown again
  // if the selection fails, detached by the selection that replaces it.
  let retained: RuntimeAttachment | null = null
  let kernelWork: Promise<unknown> = Promise.resolve()
  const serialized = <T>(work: () => Promise<T>) => {
    const result = kernelWork.then(work)
    kernelWork = result.catch(() => {})
    return result
  }
  const supersedeSelection = () => {
    selectionGeneration += 1
    selection = { kind: "idle" }
  }
  const isSessionSelecting = () => selection.kind !== "idle"
  const formatError = deps.formatError ?? ((error: unknown) => error instanceof Error ? error.message : String(error))

  const applyAttachedState = (session: RuntimeSession, attachment: RuntimeAttachment, createdSession: boolean) => {
    const nextAttachedState = deps.deriveAttachedCliTransitionState({
      session,
      createdSession,
      connectedStatus: deps.connectedStatus,
    })
    deps.setMultiAgentResponseLayout(sessionResponseLayout(session, deps.layoutPreference?.() ?? null))
    deps.setCreatedSessionState(nextAttachedState.createdSession)
    deps.setSessionState(nextAttachedState.session)
    deps.setCenterMode(nextAttachedState.centerMode)
    deps.setAttachmentState(attachment)
    deps.clearDirectoryTree()
    deps.resetWorkspaceScreen()
    deps.clearWorkflows()
    deps.clearActiveToolLabels()
    deps.setProviderActivityLabel(nextAttachedState.providerActivityLabel)
    deps.setActiveStatusLabel(nextAttachedState.activeStatusLabel)
    deps.setFatalError(nextAttachedState.fatalError)
    deps.setDaemonDisconnected(nextAttachedState.daemonDisconnected)
    deps.setStreamingAgentId(nextAttachedState.streamingAgentId)
    deps.setSubmitting(nextAttachedState.submitting)
    deps.setWorking(nextAttachedState.working)
    deps.setStatusLine(nextAttachedState.statusLine)
    deps.updateSessionChrome()
    deps.focusPromptInput()
    deps.refreshSplitPaneFocusRepaint?.()
  }

  const transitionToNoSession = async (message = "No session attached.") => {
    supersedeSelection()
    // Callers detach what they hold; a moved-aside attachment must not be restored.
    retained = null
    const nextDetachedState = deps.deriveDetachedCliTransitionState({
      cliOptions: deps.cliOptions,
      waitingRoomState: deps.waitingRoomState(),
      message,
    })
    deps.setAttachmentState(null)
    deps.setProviderRunState(null)
    deps.clearPendingPromptAttachments()
    deps.resetWorkspaceScreen()
    deps.clearWorkflows()
    deps.setCenterMode(nextDetachedState.centerMode)
    deps.clearDirectoryTree()
    deps.clearActiveToolLabels()
    deps.setProviderActivityLabel(nextDetachedState.providerActivityLabel)
    deps.setActiveStatusLabel(nextDetachedState.activeStatusLabel)
    deps.setCreatedSessionState(nextDetachedState.createdSession)
    deps.setSessionState(nextDetachedState.session)
    deps.refreshResponseLayout()
    deps.bumpHistoryLoadGeneration()
    deps.clearTranscript()
    deps.setAgentPaneEntries(nextDetachedState.agentPaneEntries as Record<string, never[]>)
    deps.setAgentPanePreviews(nextDetachedState.agentPanePreviews)
    deps.setAgentActivityLabels(nextDetachedState.agentActivityLabels)
    deps.setStreamingAgentId(nextDetachedState.streamingAgentId)
    deps.clearAgentPaneRuntime()
    deps.setSubmitting(nextDetachedState.submitting)
    deps.setWorking(nextDetachedState.working)
    deps.resetStopRequestInFlight()
    deps.setFatalError(nextDetachedState.fatalError)
    deps.setDaemonDisconnected(nextDetachedState.daemonDisconnected)
    deps.setNextHistoryCursor(nextDetachedState.nextHistoryCursor)
    deps.setSessionHydratingState(false)
    deps.setHistoryLoadingState(false)
    deps.setStatusLine(nextDetachedState.statusLine)
    deps.updateSessionChrome()
    deps.clearPromptInput()
    deps.syncPromptTextSnapshot()
    deps.blurPromptInput()
    deps.reconcileWaitingRoom(nextDetachedState.waitingRoomState)
    await deps.refreshWaitingRoomData()
    deps.requestRender()
  }

  // Cleanup must still see an attachment a pending selection moved aside.
  const currentAttachment = () => deps.attachmentState() ?? retained

  // A removed attachment is already detached. Keep real transport and
  // admission failures visible instead of preventing the next selection.
  const detachAttachment = async (attachment: RuntimeAttachment) => {
    try {
      await deps.detachAttachment(attachment.id)
    } catch (error) {
      if (!formatError(error).includes(`attachment \`${attachment.id}\` was not found`)) {
        throw error
      }
    }
  }

  const detachQuietly = async (attachment: RuntimeAttachment, message: string) => {
    try {
      await deps.detachAttachment(attachment.id)
    } catch (error) {
      deps.logWarning?.(message, { attachment_id: attachment.id, error: formatError(error) })
    }
  }

  const detachCurrentAttachment = async () => {
    const attachment = currentAttachment()
    if (!attachment) {
      return
    }
    if (deps.attachmentState()?.id === attachment.id) deps.setAttachmentState(null)
    try { await detachAttachment(attachment) } catch (error) {
      if (!deps.attachmentState() && deps.sessionState().id === attachment.session_id) deps.setAttachmentState(attachment)
      throw error
    }
    if (retained?.id === attachment.id) retained = null
  }

  const rollbackAttachedSession = async (
    sessionId: string,
    message = "Managed session launch was cancelled.",
  ) => {
    const attachment = deps.attachmentState()
    if (attachment?.session_id !== sessionId || deps.sessionState().id !== sessionId) {
      return false
    }
    let detachError: unknown
    try {
      await deps.detachAttachment(attachment.id)
    } catch (error) {
      detachError = error
    }
    if (deps.attachmentState()?.id === attachment.id && deps.sessionState().id === sessionId) {
      await transitionToNoSession(message)
    }
    if (detachError) throw detachError
    return true
  }

  // Restoring: the previous attachment survived a failed selection; show it again.
  const restoreRetained = async (current: () => boolean) => {
    const attachment = retained
    if (!attachment) return
    try {
      const session = await deps.getSessionState(attachment.session_id)
      if (!current()) return
      retained = null
      applyAttachedState(session, attachment, false)
      const run = session.active_provider_run_id ? await deps.tryGetProviderRun(session.active_provider_run_id) : null
      if (!current()) return
      deps.setProviderRunState(run)
      if (run) {
        deps.syncCliProviderSelection({
          provider: run.provider,
          model: run.model,
          effort: run.variant ?? deps.cliOptions.effort,
        })
      }
      deps.bumpHistoryLoadGeneration()
      const hydrated = await deps.hydrateAttachedSessionBinding(session.id, attachment.id, session)
      if (!current()) return
      applyAttachedState(hydrated, attachment, false)
      await deps.syncKernelEventSubscription?.()
    } catch (error) {
      if (!current()) return
      // Take it out of the view first, so no newer selection detaches it again.
      if (retained?.id === attachment.id) retained = null
      if (deps.attachmentState()?.id === attachment.id) deps.setAttachmentState(null)
      await detachQuietly(attachment, "failed to detach the previous session after restore failed")
      if (!current()) return
      await transitionToNoSession(`Could not restore the previous session: ${formatError(error)}`)
    }
  }

  // Switching: hand the view to the selected session's placeholder at once.
  const beginSessionSelection = (session: SessionReference): SessionSelection => {
    const shown = deps.attachmentState()
    if (shown?.session_id === session.id) return keepShownSelection()
    selectionGeneration += 1
    const generation = selectionGeneration
    const current = () => generation === selectionGeneration
    selection = { kind: "switching", generation }
    // The old attachment may take seconds to detach: move display ownership
    // now, with no attachment or launch authority.
    if (shown) retained = shown
    const label = `Loading session ${session.alias?.trim() || session.id}…`
    const pending = deps.deriveDetachedCliTransitionState({
      cliOptions: deps.cliOptions,
      waitingRoomState: deps.waitingRoomState(),
      message: label,
    })
    deps.setAttachmentState(null)
    deps.setProviderRunState(null)
    deps.setSessionState({ ...pending.session, ...session, active_prompt: null, queued_prompts: [], attachment_ids: [] })
    deps.clearTranscript()
    deps.clearAgentPaneRuntime()
    deps.setAgentPaneEntries({})
    deps.setAgentPanePreviews({})
    deps.setAgentActivityLabels({})
    deps.clearWorkflows()
    deps.clearActiveToolLabels()
    deps.clearDirectoryTree()
    deps.resetWorkspaceScreen()
    deps.setWorking(false)
    deps.setSubmitting(false)
    deps.setStreamingAgentId(null)
    deps.setProviderActivityLabel(null)
    deps.setActiveStatusLabel(null)
    deps.setStatusLine(label)
    deps.clearPendingPromptAttachments()
    deps.bumpHistoryLoadGeneration()
    deps.setNextHistoryCursor(null)
    deps.setSessionHydratingState(true)
    deps.updateSessionChrome()
    deps.requestRender()
    return {
      current,
      attach: (target, createdSession = false, launch) => serialized(async () => {
        if (!current()) return
        try {
          await attachSelected(current, target, createdSession, launch)
        } catch (error) {
          if (current()) throw error
        }
      }),
      // Ends the selection: a selection that left nothing attached restores the
      // previous attachment, or falls back to the waiting room.
      settle: () => serialized(async () => {
        if (!current()) return
        if (!deps.attachmentState() && retained) {
          selection = { kind: "restoring", generation }
          await restoreRetained(current)
        } else if (!deps.attachmentState()) {
          await transitionToNoSession("Could not open the selected session.")
        }
        if (!current()) return
        selection = { kind: "idle" }
        deps.setSessionHydratingState(false)
        deps.requestRender()
      }),
    }
  }

  // Selecting the shown session changes nothing, but its lookup is still owned:
  // any newer selection or return to the waiting room discards its result.
  const keepShownSelection = (): SessionSelection => {
    const generation = selectionGeneration
    const current = () => generation === selectionGeneration
    return {
      current,
      attach: async (target, createdSession = false, launch) => {
        if (!current() || deps.attachmentState()?.session_id === target.id) return
        await attachBinding(target, createdSession, launch)
      },
      settle: async () => {},
    }
  }

  const attachBinding = async (
    session: SessionReference,
    createdSession: boolean,
    launch?: LaunchSelection,
  ) => {
    const selected = beginSessionSelection(session)
    try {
      await selected.attach(session, createdSession, launch)
    } finally {
      await selected.settle()
    }
  }

  const attachSelected = async (
    current: () => boolean,
    session: SessionReference,
    createdSession: boolean,
    launch: LaunchSelection = { provider: deps.cliOptions.provider ?? "opencode", model: deps.cliOptions.model, effort: deps.cliOptions.effort },
  ) => {
    // Selecting the session that was moved aside just restores it on settle.
    if (retained?.session_id === session.id) return
    const previous = retained
    if (previous) {
      await detachAttachment(previous)
      if (retained?.id === previous.id) retained = null
      if (!current()) return
    }
    const attachment = await deps.attachToSession(session.id, deps.cliOptions.clientId)
    // Until the view holds it, the attachment belongs to this selection alone.
    const disposeUnshown = async () => {
      if (deps.attachmentState()?.id === attachment.id || retained?.id === attachment.id) return
      await detachQuietly(attachment, "failed to detach a superseded session attachment")
    }
    try {
      await attachShown(current, session, attachment, createdSession, launch)
    } catch (error) {
      await disposeUnshown()
      throw error
    }
    if (!current()) await disposeUnshown()
  }

  const attachShown = async (
    current: () => boolean,
    session: SessionReference,
    attachment: RuntimeAttachment,
    createdSession: boolean,
    launch: LaunchSelection,
  ) => {
    if (!current()) return

    const provisionalSession = isCompleteSessionSnapshot(session)
      ? session
      : null
    if (provisionalSession) {
      applyAttachedState(provisionalSession, attachment, createdSession)
    }

    let attachedSession = await deps.getSessionState(session.id)
    if (!current()) return

    applyAttachedState(attachedSession, attachment, createdSession)

    try {
      await deps.primeAttachedSessionBinding?.(attachedSession, current)
    } catch (error) {
      deps.logWarning?.("failed to prime attached session view", {
        session_id: session.id,
        attachment_id: attachment.id,
        error: formatError(error),
      })
    }
    if (!current()) return
    deps.setSessionHydratingState(false)

    try {
      await deps.syncKernelEventSubscription?.()
    } catch (error) {
      deps.logWarning?.("failed to synchronize kernel event subscription after attach", {
        session_id: session.id,
        attachment_id: attachment.id,
        error: formatError(error),
      })
    }

    if (!current()) return
    let postHydrationStatusLine: string | null = null
    const providerSettlement = await settleAttachProviderRun(
      attachedSession,
      launch,
      deps.cliOptions.accountProfile,
      createdSession,
      {
        launchProviderRun: deps.launchProviderRun,
        getSessionState: deps.getSessionState,
        tryGetProviderRun: deps.tryGetProviderRun,
      },
    )
    attachedSession = providerSettlement.session
    if (!current()) return
    switch (providerSettlement.action) {
      case "launched": {
        deps.cliOptions.provider = providerSettlement.launch.provider
        deps.cliOptions.model = providerSettlement.launch.model
        deps.cliOptions.effort = providerSettlement.launch.effort
        const run = providerSettlement.providerRun
        deps.logAttachedProviderRun?.("launched", run, {
          session_id: session.id,
          requested_model: providerSettlement.launch.model,
          requested_variant: providerSettlement.launch.effort,
        })
        deps.setProviderRunState(run)
        deps.syncCliProviderSelection({
          provider: run.provider,
          model: run.model,
          effort: run.variant ?? providerSettlement.launch.effort,
        })
        break
      }
      case "loaded": {
        const run = providerSettlement.providerRun
        deps.logAttachedProviderRun?.("loaded", run, {
          session_id: session.id,
          requested_model: deps.cliOptions.model,
        })
        deps.setProviderRunState(run)
        if (run) {
          deps.syncCliProviderSelection({
            provider: run.provider,
            model: run.model,
            effort: run.variant ?? deps.cliOptions.effort,
          })
        }
        break
      }
      case "skipped":
        deps.cliOptions.provider = providerSettlement.launch.provider
        deps.cliOptions.model = providerSettlement.launch.model
        deps.cliOptions.effort = providerSettlement.launch.effort
        if (providerSettlement.recoveredRemotePlacement) {
          deps.logWarning?.("recovered attach-time provider launch after agent moved remote", {
            session_id: session.id,
            agent_id: providerSettlement.targetAgent?.id ?? null,
            worker_kernel_id: providerSettlement.targetAgent?.remote_execution?.worker_kernel_id ?? null,
          })
        } else if (providerSettlement.reason === "no_visible_agents") {
          deps.logWarning?.("skipping provider launch because no agents are visible to this client", {
            session_id: session.id,
            focused_agent_id: attachedSession.focused_agent_id,
          })
        } else if (providerSettlement.reason === "missing_focused_agent") {
          deps.logWarning?.("skipping provider launch because focused agent is not visible to this client", {
            session_id: session.id,
            focused_agent_id: attachedSession.focused_agent_id,
          })
        } else if (providerSettlement.reason === "remote_backed_agent") {
          deps.logWarning?.("skipping attach-time provider launch for remote-backed agent", {
            session_id: session.id,
            agent_id: providerSettlement.targetAgent?.id ?? null,
            worker_kernel_id: providerSettlement.targetAgent?.remote_execution?.worker_kernel_id ?? null,
          })
        } else if (providerSettlement.reason === "credential_vault_locked") {
          deps.logWarning?.("skipping attach-time provider launch because the credential vault is locked", {
            session_id: session.id,
            agent_id: providerSettlement.targetAgent?.id ?? null,
          })
          postHydrationStatusLine = "Chariox vault locked. Run /credential vault manage."
          deps.setStatusLine(postHydrationStatusLine)
        }
        deps.setProviderRunState(null)
        break
      default: {
        const exhaustive: never = providerSettlement
        throw new Error(`unhandled attach provider launch decision ${String(exhaustive)}`)
      }
    }

    if (!current()) return
    deps.reconcileWaitingRoom(deps.waitingRoomState())

    let hydratedSession = attachedSession
    try {
      hydratedSession = await deps.hydrateAttachedSessionBinding(
        session.id,
        attachment.id,
        attachedSession,
      )
    } catch (error) {
      deps.logWarning?.("failed to hydrate attached session after attach", {
        session_id: session.id,
        attachment_id: attachment.id,
        error: formatError(error),
      })
    }

    if (!current()) return
    applyAttachedState(hydratedSession, attachment, createdSession)
    if (postHydrationStatusLine) {
      deps.setStatusLine(postHydrationStatusLine)
    }

    await refreshAttachedSessionRow(hydratedSession)
    if (!current()) return

    deps.scheduleShortViewportHistoryCheck()
  }

  const refreshAttachedSessionRow = async (session: RuntimeSession) => {
    if (deps.getAvailableSessions) {
      deps.setAvailableSessions(upsertSessionListEntry(
        deps.getAvailableSessions(),
        sessionListEntryFromSession(session),
      ))
      return
    }
    try {
      deps.setAvailableSessions(await deps.listSessions())
    } catch (error) {
      deps.logWarning?.("failed to refresh session list after attach", {
        session_id: session.id,
        error: formatError(error),
      })
    }
  }

  return {
    transitionToNoSession,
    currentAttachment,
    detachCurrentAttachment,
    rollbackAttachedSession,
    beginSessionSelection,
    isSessionSelecting,
    attachBinding,
  }
}
