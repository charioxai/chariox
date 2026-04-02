import type {
  CliOptions,
  RuntimeAttachment,
  RuntimeProviderRun,
  RuntimeSession,
  SessionHistoryCursor,
} from "./cli-types.js"
import type {
  AttachedCliTransitionState,
  DetachedCliTransitionState,
} from "./session-state.js"
import { sessionResponseLayout } from "./session-state.js"
import type { MultiAgentResponseLayout } from "./preferences.js"
import type { WaitingRoomState } from "./waiting-room.js"

type ProviderCatalog = Record<string, unknown>
type LaunchSelection = { model: string; effort: string }

type SessionLifecycleDeps = {
  cliOptions: CliOptions
  connectedStatus: string
  waitingRoomState: () => WaitingRoomState
  attachmentState: () => RuntimeAttachment | null
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
  setNextHistoryCursor: (value: SessionHistoryCursor | null) => void
  setHistoryLoadingState: (value: boolean) => void
  setStatusLine: (value: string) => void
  updateSessionChrome: () => void
  attachToSession: (sessionId: string, clientId: string) => Promise<RuntimeAttachment>
  getSessionState: (sessionId: string) => Promise<RuntimeSession>
  launchProviderRun: (
    sessionId: string,
    accountProfile: string,
    model: string,
    effort: string,
    targetAgentId?: string | null,
  ) => Promise<RuntimeProviderRun>
  tryGetProviderRun: (
    providerRunId: string,
  ) => Promise<RuntimeProviderRun | null>
  setProviderCatalogState: (catalog: ProviderCatalog) => void
  getProviderCatalog: () => Promise<ProviderCatalog>
  maybeResize: (sessionId: string) => Promise<void>
  catchUpAttachedSession: (
    sessionId: string,
    attachmentId: string,
    session: RuntimeSession,
  ) => Promise<void>
  refreshAgentPanes: (session: RuntimeSession) => Promise<void>
  setAvailableSessions: (sessions: RuntimeSession[]) => void
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
    deps.setSubmitting(nextAttachedState.submitting)
    deps.setWorking(nextAttachedState.working)
    deps.setStatusLine(nextAttachedState.statusLine)
    deps.updateSessionChrome()
    deps.focusPromptInput()
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

  const attachBinding = async (
    session: Pick<RuntimeSession, "id">,
    createdSession: boolean,
    launch: LaunchSelection = { model: deps.cliOptions.model, effort: deps.cliOptions.effort },
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
    const attachment = await deps.attachToSession(session.id, deps.cliOptions.clientId)
    const attachedSession = await deps.getSessionState(session.id)
    if (!attachedSession.active_provider_run_id) {
      deps.cliOptions.model = launch.model
      deps.cliOptions.effort = launch.effort
      const run = await deps.launchProviderRun(
        session.id,
        deps.cliOptions.accountProfile,
        launch.model,
        launch.effort,
        attachedSession.focused_agent_id,
      )
      deps.logAttachedProviderRun?.("launched", run, {
        session_id: session.id,
        requested_model: launch.model,
        requested_variant: launch.effort,
      })
      deps.setProviderRunState(run)
    } else {
      const run = await deps.tryGetProviderRun(attachedSession.active_provider_run_id)
      deps.logAttachedProviderRun?.("loaded", run, {
        session_id: session.id,
        requested_model: deps.cliOptions.model,
      })
      deps.setProviderRunState(run)
    }

    applyAttachedState(attachedSession, attachment, createdSession)

    try {
      await deps.syncKernelEventSubscription?.()
    } catch (error) {
      deps.logWarning?.("failed to synchronize kernel event subscription after attach", {
        session_id: session.id,
        attachment_id: attachment.id,
        error: formatError(error),
      })
    }

    try {
      deps.setProviderCatalogState(await deps.getProviderCatalog())
    } catch (error) {
      deps.logWarning?.("failed to refresh provider catalog after attach", {
        session_id: session.id,
        error: formatError(error),
      })
    }

    deps.reconcileWaitingRoom(deps.waitingRoomState())

    try {
      await deps.maybeResize(session.id)
    } catch (error) {
      deps.logWarning?.("failed to resize attached session", {
        session_id: session.id,
        error: formatError(error),
      })
    }

    try {
      await deps.catchUpAttachedSession(session.id, attachment.id, attachedSession)
    } catch (error) {
      deps.logWarning?.("failed to catch up attached session", {
        session_id: session.id,
        attachment_id: attachment.id,
        error: formatError(error),
      })
    }

    let hydratedSession = attachedSession
    try {
      hydratedSession = await deps.getSessionState(session.id)
    } catch (error) {
      deps.logWarning?.("failed to hydrate attached session after attach", {
        session_id: session.id,
        error: formatError(error),
      })
    }

    try {
      await deps.refreshAgentPanes(hydratedSession)
    } catch (error) {
      deps.logWarning?.("failed to refresh agent panes after attach", {
        session_id: session.id,
        error: formatError(error),
      })
    }

    applyAttachedState(hydratedSession, attachment, createdSession)

    try {
      deps.setAvailableSessions(await deps.listSessions())
    } catch (error) {
      deps.logWarning?.("failed to refresh session list after attach", {
        session_id: session.id,
        error: formatError(error),
      })
    }

    deps.scheduleShortViewportHistoryCheck()
  }

  return {
    transitionToNoSession,
    detachCurrentAttachment,
    attachBinding,
  }
}
