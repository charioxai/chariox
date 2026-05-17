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
import type { WaitingRoomState } from "./waiting-room-types.js"
import type { SessionListEntry } from "./sessions.js"

type ProviderCatalog = Record<string, unknown>
type LaunchSelection = { provider: string; model: string; effort: string }

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
  setNextHistoryCursor: (value: SessionHistoryCursor | null) => void
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
  syncCliProviderSelection: (selection: ProviderSelectionState) => void
  getProviderCatalog: () => Promise<ProviderCatalog>
  primeAttachedSessionBinding?: (session: RuntimeSession) => Promise<void>
  hydrateAttachedSessionBinding: (
    sessionId: string,
    attachmentId: string,
    session: RuntimeSession,
  ) => Promise<RuntimeSession>
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

      const attachedSession = await deps.getSessionState(session.id)

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

      if (!attachedSession.active_provider_run_id) {
        const resolvedLaunch = resolveStoredAgentLaunch(attachedSession, launch, createdSession)
        deps.cliOptions.provider = resolvedLaunch.provider
        deps.cliOptions.model = resolvedLaunch.model
        deps.cliOptions.effort = resolvedLaunch.effort
        const launchTargetAgent = resolveLaunchTargetAgent(attachedSession)
        const launchTargetAgentId = launchTargetAgent?.id ?? null
        if (attachedSession.agents.length === 0 && !createdSession) {
          deps.logWarning?.("skipping provider launch because no agents are visible to this client", {
            session_id: session.id,
            focused_agent_id: attachedSession.focused_agent_id,
          })
          deps.setProviderRunState(null)
        } else if (launchTargetAgent?.remote_execution) {
          deps.logWarning?.("skipping attach-time provider launch for remote-backed agent", {
            session_id: session.id,
            agent_id: launchTargetAgent.id,
            worker_kernel_id: launchTargetAgent.remote_execution.worker_kernel_id,
          })
          deps.setProviderRunState(null)
        } else {
          const run = await deps.launchProviderRun(
            session.id,
            resolvedLaunch.provider,
            deps.cliOptions.accountProfile,
            resolvedLaunch.model,
            resolvedLaunch.effort,
            launchTargetAgentId,
          )
          deps.logAttachedProviderRun?.("launched", run, {
            session_id: session.id,
            requested_model: resolvedLaunch.model,
            requested_variant: resolvedLaunch.effort,
          })
          deps.setProviderRunState(run)
          deps.syncCliProviderSelection({
            provider: run.provider,
            model: run.model,
            effort: run.variant ?? resolvedLaunch.effort,
          })
        }
      } else {
        const run = await deps.tryGetProviderRun(attachedSession.active_provider_run_id)
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

      try {
        deps.setAvailableSessions(await deps.listSessions())
      } catch (error) {
        deps.logWarning?.("failed to refresh session list after attach", {
          session_id: session.id,
          error: formatError(error),
        })
      }

      deps.scheduleShortViewportHistoryCheck()
    } finally {
      clearSessionHydrating()
    }
  }

  return {
    transitionToNoSession,
    detachCurrentAttachment,
    attachBinding,
  }
}

function isCompleteSessionSnapshot(
  session: Pick<RuntimeSession, "id"> & Partial<RuntimeSession>,
): session is RuntimeSession {
  return typeof session.workspace_id === "string"
    && typeof session.worktree_id === "string"
    && typeof session.created_at_ms === "number"
    && typeof session.status === "string"
    && Array.isArray(session.attachment_ids)
    && Array.isArray(session.queued_prompts)
    && Array.isArray(session.agents)
    && Array.isArray(session.workflows)
    && Array.isArray(session.workflow_runs)
    && Array.isArray(session.workflow_watchdogs)
    && Array.isArray(session.workflow_consoles)
    && typeof session.max_agents === "number"
    && typeof session.config_state === "object"
    && session.config_state !== null
}

function resolveLaunchTargetAgent(session: RuntimeSession): RuntimeSession["agents"][number] | null {
  if (session.focused_agent_id) {
    const focusedAgent = session.agents.find((agent) => agent.id === session.focused_agent_id)
    if (focusedAgent) return focusedAgent
  }
  return session.agents[0] ?? null
}

function resolveStoredAgentLaunch(
  session: RuntimeSession,
  fallback: LaunchSelection,
  createdSession: boolean,
): LaunchSelection {
  if (createdSession) {
    return resolveSessionAgentDefaults(session, fallback)
  }

  const sessionDefaults = resolveSessionAgentDefaults(session, fallback)
  const focusedAgent = session.agents.find((agent) => agent.id === session.focused_agent_id) ?? session.agents[0]
  if (!focusedAgent) {
    return sessionDefaults
  }

  return {
    provider: focusedAgent.provider && focusedAgent.provider !== "default"
      ? focusedAgent.provider
      : sessionDefaults.provider,
    model: focusedAgent.model?.trim() || sessionDefaults.model,
    effort: focusedAgent.effort?.trim() || sessionDefaults.effort,
  }
}

function resolveSessionAgentDefaults(
  session: Pick<RuntimeSession, "id"> & Partial<RuntimeSession>,
  fallback: LaunchSelection,
): LaunchSelection {
  const defaults = session.agent_defaults
  return {
    provider: defaults?.provider?.trim() && defaults.provider !== "default" ? defaults.provider : fallback.provider,
    model: defaults?.model?.trim() || fallback.model,
    effort: defaults?.effort?.trim() || fallback.effort,
  }
}
