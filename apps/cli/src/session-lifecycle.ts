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
  setProviderCatalogState: (catalog: ProviderCatalog) => void
  setTerminalCommandCatalogState: (catalog: TerminalCommandCatalog) => void
  syncCliProviderSelection: (selection: ProviderSelectionState) => void
  getProviderCatalog: () => Promise<ProviderCatalog>
  getTerminalCommandCatalog: () => Promise<TerminalCommandCatalog>
  primeAttachedSessionBinding?: (session: RuntimeSession) => Promise<void>
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

export function createSessionLifecycleController(deps: SessionLifecycleDeps) {
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

  const detachCurrentAttachment = async () => {
    const attachment = deps.attachmentState()
    if (!attachment) {
      return
    }
    await deps.detachAttachment(attachment.id)
    deps.setAttachmentState(null)
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

  const attachBinding = async (
    session: Pick<RuntimeSession, "id"> & Partial<RuntimeSession>,
    createdSession: boolean,
    launch: LaunchSelection = { provider: deps.cliOptions.provider ?? "opencode", model: deps.cliOptions.model, effort: deps.cliOptions.effort },
  ) => {
    const currentAttachment = deps.attachmentState()
    if (currentAttachment?.session_id === session.id) {
      return
    }
    if (currentAttachment) {
      await detachCurrentAttachment()
    }
    deps.clearPendingPromptAttachments()
    deps.bumpHistoryLoadGeneration()
    let sessionHydratingCleared = false
    const clearSessionHydrating = () => {
      if (sessionHydratingCleared) {
        return
      }
      sessionHydratingCleared = true
      deps.setSessionHydratingState(false)
    }
    deps.setSessionHydratingState(true)
    try {
      const attachment = await deps.attachToSession(session.id, deps.cliOptions.clientId)

      const provisionalSession = isCompleteSessionSnapshot(session)
        ? session
        : null
      if (provisionalSession) {
        applyAttachedState(provisionalSession, attachment, createdSession)
      }

      let attachedSession = await deps.getSessionState(session.id)

      applyAttachedState(attachedSession, attachment, createdSession)

      try {
        await deps.primeAttachedSessionBinding?.(attachedSession)
      } catch (error) {
        deps.logWarning?.("failed to prime attached session view", {
          session_id: session.id,
          attachment_id: attachment.id,
          error: formatError(error),
        })
      }
      clearSessionHydrating()

      try {
        await deps.syncKernelEventSubscription?.()
      } catch (error) {
        deps.logWarning?.("failed to synchronize kernel event subscription after attach", {
          session_id: session.id,
          attachment_id: attachment.id,
          error: formatError(error),
        })
      }

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
            deps.setStatusLine("Chariox vault locked. Run /credential vault manage.")
          }
          deps.setProviderRunState(null)
          break
        default: {
          const exhaustive: never = providerSettlement
          throw new Error(`unhandled attach provider launch decision ${String(exhaustive)}`)
        }
      }

      try {
        deps.setProviderCatalogState(await deps.getProviderCatalog())
      } catch (error) {
        deps.logWarning?.("failed to refresh provider catalog after attach", {
          session_id: session.id,
          error: formatError(error),
        })
      }
      try {
        deps.setTerminalCommandCatalogState(await deps.getTerminalCommandCatalog())
      } catch (error) {
        deps.logWarning?.("failed to refresh terminal command catalog after attach", {
          session_id: session.id,
          error: formatError(error),
        })
      }

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

      applyAttachedState(hydratedSession, attachment, createdSession)

      await refreshAttachedSessionRow(hydratedSession)

      deps.scheduleShortViewportHistoryCheck()
    } finally {
      clearSessionHydrating()
    }
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
    detachCurrentAttachment,
    rollbackAttachedSession,
    attachBinding,
  }
}
