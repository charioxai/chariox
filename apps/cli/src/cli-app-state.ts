import { createSignal } from "solid-js"
import { createStore } from "solid-js/store"

import type {
  AgentInstance,
  BootstrapState,
  ExternalProviderSessionRecord,
  RuntimeAttachment,
  RuntimeProviderRun,
  SessionHistoryCursorState,
  SliceRecord,
  TranscriptEntry,
  WorkspaceLiveSyncStatus,
} from "./cli-types.js"
import { formatAgentLocationLabel } from "./agent-label.js"
import type { FooterFlash } from "./footer-flash-controller.js"
import {
  mergeUiPreferences,
  resolveMaxAgentsPerScreen,
  saveUiPreferences,
  sessionPromptDraftEntry,
  sessionPromptHistoryEntries,
  type CharioxPreferences,
  type MultiAgentResponseLayout,
} from "./preferences.js"
import type {
  BackendProviderId,
  ProviderCatalog,
} from "./provider-catalog.js"
import type { ProviderCommandCatalogs } from "./provider-command-catalog.js"
import type { PendingPromptAttachment } from "./prompt-attachment-state.js"
import type {
  RelayStatusView,
  TerminalPairingLinkView,
  TerminalView,
} from "./relay-api.js"
import { createMutableLocalIpcClient } from "./mutable-local-ipc-client.js"
import { createDefaultShellContext, type ShellContext } from "@chariox/kernel-client/shell-core"
import type { TerminalCommandCatalog } from "@chariox/kernel-client/kernel-types"
import { DEFAULT_CONNECTED_STATUS } from "./runtime.js"
import { buildDetachedSessionState } from "./session-state.js"
import {
  sessionHasTurnWork,
  sessionProjectedStreamingAgentId,
} from "@chariox/kernel-client/session-prompt-work"
import {
  sessionResponseLayout,
} from "@chariox/kernel-client/session-config-projection"
import {
  sessionFocusedAgentId,
} from "@chariox/kernel-client/session-runtime-transition"
import type { SessionListEntry } from "./sessions.js"
import { applyTheme, setThemeRegistry } from "./theme.js"
import { DEFAULT_THEME_REGISTRY } from "./theme-registry.js"
import type {
  RemoteKernelView,
  RemoteMachineView,
} from "./waiting-room-inventory-api.js"
import { createWaitingRoomHiddenKernelController } from "./waiting-room-hidden-kernel-controller.js"
import { createWaitingRoomState } from "./waiting-room-state.js"
import type { WaitingRoomState } from "./waiting-room-types.js"
import { createWaitingRoomLaunchOwnershipTracker } from "./waiting-room-launch-ownership.js"
import type { WaitingRoomProjectSummary } from "./waiting-room-projects.js"
import type {
  WorkspaceScreenMode,
} from "./workspace-screen.js"
import type {
  WorkspaceShellEntry,
} from "./workspace-shell.js"
import type {
  WorkflowInspectorMode,
} from "./workflow-inspector-projection.js"
import type { WorkflowComponentSelection } from "./workflow-component-selection.js"
import type {
  WorkflowNodeInstructionsEditor,
} from "./workflow-node-instructions-editor-controller.js"

export function createCliAppState(options: {
  bootstrap: BootstrapState
  cwd: string
}) {
  const { options: cliOptions } = options.bootstrap
  const client = createMutableLocalIpcClient(options.bootstrap.client)
  const supportsKernelEventStream = client.supportsKernelEvents()
  const launchedDetached = Boolean(cliOptions.detached && !options.bootstrap.binding)
  const initialBinding = options.bootstrap.binding
  const initialSession = initialBinding?.session ?? buildDetachedSessionState(cliOptions)
  const initialEntries = initialBinding?.historyEntries ?? []
  const initialSessions = options.bootstrap.sessions
  const initialProviderCatalog = options.bootstrap.providerCatalog
  const initialProviderCommandCatalogs = options.bootstrap.providerCommandCatalogs
  const initialTerminalCommandCatalog = options.bootstrap.terminalCommandCatalog
  const initialPreferences = options.bootstrap.preferences
  const initialThemeRegistry = options.bootstrap.themeRegistry ?? DEFAULT_THEME_REGISTRY
  setThemeRegistry(initialThemeRegistry)
  const initialThemeId = applyTheme(initialPreferences.ui?.theme, initialThemeRegistry)
  const initialPromptHistory = initialBinding?.promptHistoryEntries
    ?? (initialBinding?.session
      ? sessionPromptHistoryEntries(initialPreferences, initialBinding.session.id)
      : [])
  const initialPromptDraft = initialBinding?.session
    ? sessionPromptDraftEntry(initialPreferences, initialBinding.session.id)
    : ""

  const [preferencesState, setPreferencesState] = createSignal<CharioxPreferences>(initialPreferences)
  const [themeRevision, setThemeRevision] = createSignal(0)
  const maxAgentsPerScreen = () => resolveMaxAgentsPerScreen(preferencesState().ui?.maxAgentsPerScreen)
  const [sessionState, setSessionState] = createSignal(initialSession)
  const [attachmentState, setAttachmentState] = createSignal<RuntimeAttachment | null>(initialBinding?.attachment ?? null)
  const [providerRunState, setProviderRunState] = createSignal<RuntimeProviderRun | null>(initialBinding?.providerRun ?? null)
  const [createdSessionState, setCreatedSessionState] = createSignal(initialBinding?.createdSession ?? false)
  const [availableSessions, setAvailableSessions] = createSignal<SessionListEntry[]>(initialSessions)
  const [waitingRoomProjects, setWaitingRoomProjects] = createSignal<WaitingRoomProjectSummary[]>([])
  const [providerCatalogState, setProviderCatalogState] = createSignal<ProviderCatalog>(initialProviderCatalog)
  const [providerCommandCatalogState, setProviderCommandCatalogState] = createSignal<ProviderCommandCatalogs>(initialProviderCommandCatalogs)
  const [terminalCommandCatalogState, setTerminalCommandCatalogState] = createSignal<TerminalCommandCatalog | null>(initialTerminalCommandCatalog)
  const [themeRegistryState] = createSignal(initialThemeRegistry)
  const [relayStatusState, setRelayStatusState] = createSignal<RelayStatusView | null>(null)
  const [remoteMachinesState, setRemoteMachinesState] = createSignal<RemoteMachineView[]>([])
  const [remoteKernelsState, setRemoteKernelsState] = createSignal<RemoteKernelView[]>([])
  const [providerAccountsState, setProviderAccountsState] = createSignal<import("./cli-types.js").ProviderAccountProfile[]>([])
  const [slicesState, setSlicesState] = createSignal<SliceRecord[]>([])
  const [terminalsState, setTerminalsState] = createSignal<TerminalView[]>([])
  const [externalProviderSessionsState, setExternalProviderSessionsState] = createSignal<ExternalProviderSessionRecord[]>([])
  const [externalProviderSessionsPageState, setExternalProviderSessionsPageState] = createSignal<{ hasMore: boolean; nextCursor: string | null }>({ hasMore: false, nextCursor: null })
  const [waitingRoomInventoryStatus, setWaitingRoomInventoryStatus] = createSignal<"loading" | "ready" | "error">("loading")
  const waitingRoomHiddenKernelController = createWaitingRoomHiddenKernelController({
    initialHiddenKernelIds: initialPreferences.ui?.hiddenRemoteKernelIds ?? [],
    persistHiddenKernelIds: (hiddenKernelIds) => {
      void saveUiPreferences({ hiddenRemoteKernelIds: hiddenKernelIds })
      setPreferencesState((current) => mergeUiPreferences(current, { hiddenRemoteKernelIds: hiddenKernelIds }))
    },
  })
  const [waitingRoomCloudNotice, setWaitingRoomCloudNotice] = createSignal<string | null>(null)
  const [terminalPairingOpen, setTerminalPairingOpen] = createSignal(false)
  const [terminalPairingState, setTerminalPairingState] = createSignal<TerminalPairingLinkView | null>(null)
  const [terminalPairingQrLines, setTerminalPairingQrLines] = createSignal<string[]>([])
  const [sessionBrowserOpen, setSessionBrowserOpen] = createSignal(false)
  const [managedMachineDialogOpen, setManagedMachineDialogOpen] = createSignal(false)
  const agentLocationLabel = (agent: AgentInstance | null | undefined): string | null =>
    formatAgentLocationLabel(agent, slicesState())
  const [sessionBrowserIndex, setSessionBrowserIndex] = createSignal(0)
  const [waitingRoomState, setWaitingRoomStateSignal] = createSignal<WaitingRoomState>(
    createWaitingRoomState(
      initialSessions,
      initialProviderCatalog,
      (cliOptions.provider ?? "opencode") as BackendProviderId,
      cliOptions.model,
      cliOptions.effort,
      initialThemeId,
      initialThemeRegistry,
    ),
  )
  const waitingRoomLaunchOwnership = createWaitingRoomLaunchOwnershipTracker(waitingRoomState())
  const setWaitingRoomState = ((
    value: WaitingRoomState | ((previous: WaitingRoomState) => WaitingRoomState),
  ) => {
    const next = setWaitingRoomStateSignal(value)
    waitingRoomLaunchOwnership.update(next)
    return next
  }) as typeof setWaitingRoomStateSignal
  const setWaitingRoomStateProjection = ((
    value: WaitingRoomState | ((previous: WaitingRoomState) => WaitingRoomState),
  ) => {
    const next = setWaitingRoomStateSignal(value)
    waitingRoomLaunchOwnership.synchronize(next)
    return next
  }) as typeof setWaitingRoomStateSignal
  const waitingRoomLaunchOwnershipRevision = waitingRoomLaunchOwnership.revision
  const initialWorkspaceTarget = initialSession.workspace_id || cliOptions.workspace || options.cwd
  const initialWorktreeTarget = initialSession.worktree_id || cliOptions.worktree || initialWorkspaceTarget
  const [pendingWorkspaceTarget, setPendingWorkspaceTarget] = createSignal(initialWorkspaceTarget)
  const [pendingWorktreeTarget, setPendingWorktreeTarget] = createSignal(initialWorktreeTarget)
  const [multiAgentResponseLayout, setMultiAgentResponseLayout] = createSignal<MultiAgentResponseLayout>(
    sessionResponseLayout(initialSession, preferencesState().ui?.multiAgentResponseLayout),
  )
  const [entries, setEntries] = createStore<TranscriptEntry[]>(initialEntries)
  const [activeStatusLabel, setActiveStatusLabel] = createSignal<string | null>(null)
  const [providerActivityLabel, setProviderActivityLabel] = createSignal<string | null>(null)
  const [agentActivityLabels, setAgentActivityLabels] = createSignal<Record<string, string | null>>({})
  const [streamingAgentId, setStreamingAgentId] = createSignal<string | null>(
    sessionProjectedStreamingAgentId(initialSession),
  )
  const [statusLine, setStatusLine] = createSignal(DEFAULT_CONNECTED_STATUS)
  const [fatalError, setFatalError] = createSignal<string | null>(null)
  const [submitting, setSubmitting] = createSignal(false)
  const [entryCounter, setEntryCounter] = createSignal(initialEntries.length)
  const [daemonDisconnected, setDaemonDisconnected] = createSignal(false)
  const [kernelConnected, setKernelConnected] = createSignal(!launchedDetached)
  const [nextHistoryCursor, setNextHistoryCursor] = createSignal<SessionHistoryCursorState>(null)
  const [agentPanePreviews, setAgentPanePreviews] = createSignal<Record<string, string>>({})
  const [agentPaneEntries, setAgentPaneEntries] = createSignal<Record<string, TranscriptEntry[]>>({})
  const [agentBusyLatches, setAgentBusyLatches] = createSignal<Record<string, boolean>>({})
  const [sessionHydrating, setSessionHydrating] = createSignal(false)
  const [loadingHistory, setLoadingHistory] = createSignal(false)
  const [historyLoadingMessage, setHistoryLoadingMessage] = createSignal<string | null>(null)
  const [workingAnimationFrame, setWorkingAnimationFrame] = createSignal(0)
  const [working, setWorking] = createSignal(sessionHasTurnWork(initialSession))
  const [footerFlash, setFooterFlash] = createSignal<FooterFlash | null>(null)
  const [pendingAttachments, setPendingAttachments] = createSignal<PendingPromptAttachment[]>([])
  const [promptHistoryEntries, setPromptHistoryEntries] = createSignal<string[]>(initialPromptHistory)
  const [promptHistoryIndex, setPromptHistoryIndex] = createSignal<number | null>(null)
  const [promptHistoryDraft, setPromptHistoryDraft] = createSignal<string | null>(null)
  const [hotkeysOpen, setHotkeysOpen] = createSignal(false)
  const [collapsedTurnIdsByAgent, setCollapsedTurnIdsByAgent] = createSignal<Record<string, number[]>>({})
  const [workspaceScreenMode, setWorkspaceScreenMode] = createSignal<WorkspaceScreenMode>("agents")
  const [workspaceShellContext, setWorkspaceShellContext] = createSignal<ShellContext>(createDefaultShellContext({
    workspace: initialWorkspaceTarget,
    worktree: initialWorktreeTarget,
    sessionId: initialBinding ? initialSession.id : undefined,
    agentId: initialBinding ? sessionFocusedAgentId(initialSession) ?? undefined : undefined,
    provider: cliOptions.provider ?? "opencode",
    model: cliOptions.model ?? "default",
    effort: cliOptions.effort || "medium",
  }))
  const [workspaceShellEntries, setWorkspaceShellEntries] = createSignal<WorkspaceShellEntry[]>([])
  const [workspaceShellEntryCounter, setWorkspaceShellEntryCounter] = createSignal(0)
  const [workspaceLiveSyncStatus, setWorkspaceLiveSyncStatus] = createSignal<WorkspaceLiveSyncStatus | null>(null)
  const [selectedWorkflowId, setSelectedWorkflowId] = createSignal<string | null>(initialSession.workflows?.[0]?.id ?? null)
  const [selectedWorkflowNodeId, setSelectedWorkflowNodeId] = createSignal<string | null>(null)
  const [selectedWorkflowComponent, setSelectedWorkflowComponent] = createSignal<WorkflowComponentSelection | null>({ kind: "workflow" })
  const [workflowInspectorMode, setWorkflowInspectorMode] = createSignal<WorkflowInspectorMode>("logs")
  const [workflowNodeInstructionsEditor, setWorkflowNodeInstructionsEditor] = createSignal<WorkflowNodeInstructionsEditor | null>(null)

  return {
    client,
    options: cliOptions,
    supportsKernelEventStream,
    launchedDetached,
    initialBinding,
    initialSession,
    initialEntries,
    initialPromptDraft,
    initialWorkspaceTarget,
    initialWorktreeTarget,
    preferencesState,
    setPreferencesState,
    themeRevision,
    setThemeRevision,
    maxAgentsPerScreen,
    sessionState,
    setSessionState,
    attachmentState,
    setAttachmentState,
    providerRunState,
    setProviderRunState,
    createdSessionState,
    setCreatedSessionState,
    availableSessions,
    setAvailableSessions,
    waitingRoomProjects,
    setWaitingRoomProjects,
    providerCatalogState,
    setProviderCatalogState,
    providerCommandCatalogState,
    setProviderCommandCatalogState,
    terminalCommandCatalogState,
    setTerminalCommandCatalogState,
    themeRegistryState,
    relayStatusState,
    setRelayStatusState,
    remoteMachinesState,
    setRemoteMachinesState,
    remoteKernelsState,
    setRemoteKernelsState,
    providerAccountsState,
    setProviderAccountsState,
    slicesState,
    setSlicesState,
    terminalsState,
    setTerminalsState,
    externalProviderSessionsState,
    setExternalProviderSessionsState,
    externalProviderSessionsPageState,
    setExternalProviderSessionsPageState,
    waitingRoomInventoryStatus,
    setWaitingRoomInventoryStatus,
    waitingRoomHiddenKernelController,
    waitingRoomCloudNotice,
    setWaitingRoomCloudNotice,
    terminalPairingOpen,
    setTerminalPairingOpen,
    terminalPairingState,
    setTerminalPairingState,
    terminalPairingQrLines,
    setTerminalPairingQrLines,
    sessionBrowserOpen,
    setSessionBrowserOpen,
    managedMachineDialogOpen,
    setManagedMachineDialogOpen,
    agentLocationLabel,
    sessionBrowserIndex,
    setSessionBrowserIndex,
    waitingRoomState,
    setWaitingRoomState,
    setWaitingRoomStateProjection,
    waitingRoomLaunchOwnershipRevision,
    pendingWorkspaceTarget,
    setPendingWorkspaceTarget,
    pendingWorktreeTarget,
    setPendingWorktreeTarget,
    multiAgentResponseLayout,
    setMultiAgentResponseLayout,
    entries,
    setEntries,
    activeStatusLabel,
    setActiveStatusLabel,
    providerActivityLabel,
    setProviderActivityLabel,
    agentActivityLabels,
    setAgentActivityLabels,
    streamingAgentId,
    setStreamingAgentId,
    statusLine,
    setStatusLine,
    fatalError,
    setFatalError,
    submitting,
    setSubmitting,
    entryCounter,
    setEntryCounter,
    daemonDisconnected,
    setDaemonDisconnected,
    kernelConnected,
    setKernelConnected,
    nextHistoryCursor,
    setNextHistoryCursor,
    agentPanePreviews,
    setAgentPanePreviews,
    agentPaneEntries,
    setAgentPaneEntries,
    agentBusyLatches,
    setAgentBusyLatches,
    sessionHydrating,
    setSessionHydrating,
    loadingHistory,
    setLoadingHistory,
    historyLoadingMessage,
    setHistoryLoadingMessage,
    workingAnimationFrame,
    setWorkingAnimationFrame,
    working,
    setWorking,
    footerFlash,
    setFooterFlash,
    pendingAttachments,
    setPendingAttachments,
    promptHistoryEntries,
    setPromptHistoryEntries,
    promptHistoryIndex,
    setPromptHistoryIndex,
    promptHistoryDraft,
    setPromptHistoryDraft,
    hotkeysOpen,
    setHotkeysOpen,
    collapsedTurnIdsByAgent,
    setCollapsedTurnIdsByAgent,
    workspaceScreenMode,
    setWorkspaceScreenMode,
    workspaceShellContext,
    setWorkspaceShellContext,
    workspaceShellEntries,
    setWorkspaceShellEntries,
    workspaceShellEntryCounter,
    setWorkspaceShellEntryCounter,
    workspaceLiveSyncStatus,
    setWorkspaceLiveSyncStatus,
    selectedWorkflowId,
    setSelectedWorkflowId,
    selectedWorkflowNodeId,
    setSelectedWorkflowNodeId,
    selectedWorkflowComponent,
    setSelectedWorkflowComponent,
    workflowInspectorMode,
    setWorkflowInspectorMode,
    workflowNodeInstructionsEditor,
    setWorkflowNodeInstructionsEditor,
  }
}
