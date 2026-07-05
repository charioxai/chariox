import type { LocalIpcClient } from "./ipc.js"
import type { ArrobaPreferences } from "./preferences.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import type { ProviderCommandCatalogs } from "./provider-command-catalog.js"
import type {
  ExternalProviderSessionCapabilities,
  ExternalProviderSessionRecord,
} from "@arroba/kernel-client/external-provider-sessions"
import type {
  AgentInstance as KernelAgentInstance,
  AgentSubstituteProfile as KernelAgentSubstituteProfile,
  AgentSubstitutionRecord as KernelAgentSubstitutionRecord,
  ArrobaConnectorAdapterDefinition as KernelArrobaConnectorAdapterDefinition,
  ArrobaConnectorDefinition as KernelArrobaConnectorDefinition,
  ArrobaCredentialConfig as KernelArrobaCredentialConfig,
  ArrobaEnvironmentConfig as KernelArrobaEnvironmentConfig,
  ArrobaMcpServerConfig as KernelArrobaMcpServerConfig,
  ArrobaScriptMetadata as KernelArrobaScriptMetadata,
  ArrobaSkillMetadata as KernelArrobaSkillMetadata,
  ArrobaUserConfig as KernelArrobaUserConfig,
  ArrobaUserConfigPayload as KernelArrobaUserConfigPayload,
  ArrobaUserConfigSchemaPayload as KernelArrobaUserConfigSchemaPayload,
  CollaborationLevel as KernelCollaborationLevel,
  AgentPromptState as KernelAgentPromptState,
  AgentQueuedPromptControl as KernelAgentQueuedPromptControl,
  AgentRuntimeActivity as KernelAgentRuntimeActivity,
  CompletedGitTurnActionProjection as KernelCompletedGitTurnActionProjection,
  CaptureScreenshotResult as KernelCaptureScreenshotResult,
  ExternalProviderImportMetadata as KernelExternalProviderImportMetadata,
  ExternalProviderObservedCursor as KernelExternalProviderObservedCursor,
  MetaagentTask as KernelMetaagentTask,
  MetaagentTaskStatus as KernelMetaagentTaskStatus,
  PromptAttachmentPart as KernelPromptAttachmentPart,
  PromptInputHistoryEntry as KernelPromptInputHistoryEntry,
  PromptInputHistoryEntryKind as KernelPromptInputHistoryEntryKind,
  PromptInputHistoryPage as KernelPromptInputHistoryPage,
  PromptQueueItem as KernelPromptQueueItem,
  PromptSubmittedPayload as KernelPromptSubmittedPayload,
  ProviderAuthStatus as KernelProviderAuthStatus,
  ProviderLoginStart as KernelProviderLoginStart,
  ProviderLogoutResult as KernelProviderLogoutResult,
  ProviderProcessInfo as KernelProviderProcessInfo,
  RemoteExtensionManifestSyncStatus as KernelRemoteExtensionManifestSyncStatus,
  RuntimeAttachment as KernelRuntimeAttachment,
  RuntimeSession as KernelRuntimeSession,
  RuntimeInteraction as KernelRuntimeInteraction,
  RuntimeInteractionChoice as KernelRuntimeInteractionChoice,
  RuntimeInteractionCustomChoice as KernelRuntimeInteractionCustomChoice,
  RuntimeNoticeRecord as KernelRuntimeNoticeRecord,
  RuntimeProviderRun as KernelRuntimeProviderRun,
  ExtensionGrant as KernelExtensionGrant,
  ExtensionKind as KernelExtensionKind,
  SessionAgentDefaults as KernelSessionAgentDefaults,
  SessionCollaborationAgentCounts as KernelSessionCollaborationAgentCounts,
  SessionConfigState as KernelSessionConfigState,
  SessionHistoryBlobContent as KernelSessionHistoryBlobContent,
  SessionHistoryEntry as KernelSessionHistoryEntry,
  SessionHistoryExternalObservation as KernelSessionHistoryExternalObservation,
  SessionHistoryOutline as KernelSessionHistoryOutline,
  SessionHistoryOutlineAgent as KernelSessionHistoryOutlineAgent,
  SessionHistoryOutlineBlob as KernelSessionHistoryOutlineBlob,
  SessionHistoryOutlineCursor as KernelSessionHistoryOutlineCursor,
  SessionHistoryOutlineTurn as KernelSessionHistoryOutlineTurn,
  SessionHistoryPageEntry as KernelSessionHistoryPageEntry,
  SessionHistoryPromptAttachment as KernelSessionHistoryPromptAttachment,
  SessionInvite as KernelSessionInvite,
  SessionMember as KernelSessionMember,
  StoredTransferArtifact as KernelStoredTransferArtifact,
  SliceBackupRecord as KernelSliceBackupRecord,
  SliceDisplayEndpoint as KernelSliceDisplayEndpoint,
  SliceLocalDockerPorts as KernelSliceLocalDockerPorts,
  SliceLogEntry as KernelSliceLogEntry,
  SliceRecord as KernelSliceRecord,
  SliceRelayEndpoint as KernelSliceRelayEndpoint,
  SliceSavedStateRecord as KernelSliceSavedStateRecord,
  RelayKernelPresence as KernelRelayKernelPresence,
  RelayStatus as KernelRelayStatus,
  RemoteMachineRecord as KernelRemoteMachineRecord,
  TerminalRecord as KernelTerminalRecord,
  TerminalCommandCatalog,
  TerminalOutputRecord as KernelTerminalOutputRecord,
  TurnUndoResult as KernelTurnUndoResult,
  McpImportOutcome as KernelMcpImportOutcome,
  McpImportSkip as KernelMcpImportSkip,
  UserConfigMutationEffect as KernelUserConfigMutationEffect,
  UserConfigSchemaEntry as KernelUserConfigSchemaEntry,
  QueuedPromptCancelledPayload as KernelQueuedPromptCancelledPayload,
  QueuedPromptSteeredPayload as KernelQueuedPromptSteeredPayload,
  QueuedPromptUpdatedPayload as KernelQueuedPromptUpdatedPayload,
  SkillImportOutcome as KernelSkillImportOutcome,
  SkillImportSkip as KernelSkillImportSkip,
  WaitingRoomPublicAgentSummary as KernelWaitingRoomPublicAgentSummary,
  WaitingRoomPublicItemActivitySummary as KernelWaitingRoomPublicItemActivitySummary,
  WaitingRoomPublicSessionSummary as KernelWaitingRoomPublicSessionSummary,
  WaitingRoomPublicSnapshot as KernelWaitingRoomPublicSnapshot,
  WaitingRoomPublicWorkflowEdgeSummary as KernelWaitingRoomPublicWorkflowEdgeSummary,
  WaitingRoomPublicWorkflowEndpointSummary as KernelWaitingRoomPublicWorkflowEndpointSummary,
  WaitingRoomPublicWorkflowNodeSummary as KernelWaitingRoomPublicWorkflowNodeSummary,
  WaitingRoomPublicWorkflowSummary as KernelWaitingRoomPublicWorkflowSummary,
  WaitingRoomSessionActivitySummary as KernelWaitingRoomSessionActivitySummary,
  WorkflowConsole as KernelWorkflowConsole,
  WorkflowConsoleEntry as KernelWorkflowConsoleEntry,
  WorkflowDefinition as KernelWorkflowDefinition,
  WorkflowEdgeDefinition as KernelWorkflowEdgeDefinition,
  WorkflowEndpointDefinition as KernelWorkflowEndpointDefinition,
  WorkflowFailureEvent as KernelWorkflowFailureEvent,
  WorkflowMessage as KernelWorkflowMessage,
  WorkflowNodeDefinition as KernelWorkflowNodeDefinition,
  WorkflowNodeRun as KernelWorkflowNodeRun,
  WorkflowPromptQueueDefinition as KernelWorkflowPromptQueueDefinition,
  WorkflowQueuedPrompt as KernelWorkflowQueuedPrompt,
  WorkflowRun as KernelWorkflowRun,
  WorkflowScheduleDefinition as KernelWorkflowScheduleDefinition,
  WorkflowScheduleTrigger as KernelWorkflowScheduleTrigger,
  WorkflowSchemaDefinition as KernelWorkflowSchemaDefinition,
  WorkflowWatchdogDefinition as KernelWorkflowWatchdogDefinition,
  WorkspaceLinkAttachment as KernelWorkspaceLinkAttachment,
  WorkspaceLinkDefinition as KernelWorkspaceLinkDefinition,
  WorkspaceLiveSyncApplyStatus as KernelWorkspaceLiveSyncApplyStatus,
  WorkspaceLiveSyncGroupStatus as KernelWorkspaceLiveSyncGroupStatus,
  WorkspaceLiveSyncPathApplyResult as KernelWorkspaceLiveSyncPathApplyResult,
  WorkspaceLiveSyncStatus as KernelWorkspaceLiveSyncStatus,
  WorkspaceLiveSyncTargetStatus as KernelWorkspaceLiveSyncTargetStatus,
} from "@arroba/kernel-client/kernel-types"
import {
  normalizeAgentPromptState as normalizeKernelAgentPromptState,
  normalizeRuntimeSession as normalizeKernelRuntimeSession,
  normalizeRuntimeSessions as normalizeKernelRuntimeSessions,
  normalizeRuntimeSessionWithAgentActivity as normalizeKernelRuntimeSessionWithAgentActivity,
} from "@arroba/kernel-client/runtime-session-normalization"
import type { ThemeRegistry } from "./theme-registry.js"
import type { DirectoryTreeEntry } from "./tree-view.js"

export type {
  ExternalProviderSessionCapabilities,
  ExternalProviderSessionRecord,
}

export type ArrobaMcpServerConfig = KernelArrobaMcpServerConfig

export type McpImportSkip = KernelMcpImportSkip

export type McpImportOutcome = KernelMcpImportOutcome

export type ArrobaSkillMetadata = KernelArrobaSkillMetadata

export type ArrobaEnvironmentConfig = KernelArrobaEnvironmentConfig

export type ArrobaScriptMetadata = KernelArrobaScriptMetadata

export type ArrobaConnectorDefinition = KernelArrobaConnectorDefinition

export type ArrobaConnectorAdapterDefinition = KernelArrobaConnectorAdapterDefinition

export type ArrobaCredentialConfig = KernelArrobaCredentialConfig

export type ExtensionKind = KernelExtensionKind

export type ExtensionGrant = KernelExtensionGrant

export type SkillImportSkip = KernelSkillImportSkip

export type SkillImportOutcome = KernelSkillImportOutcome

export type RuntimeSession = KernelRuntimeSession & {
  workspace_label?: string | null
  directory?: string | null
  worktree_label?: string | null
  hidden?: boolean
}

export type MetaagentTaskStatus = KernelMetaagentTaskStatus

export type MetaagentTask = KernelMetaagentTask

export type ExternalProviderObservedCursor = KernelExternalProviderObservedCursor

export type ExternalProviderImportMetadata = KernelExternalProviderImportMetadata

export type SessionCollaborationAgentCounts = KernelSessionCollaborationAgentCounts

export type SessionMember = KernelSessionMember

export type SessionInvite = KernelSessionInvite

export type CollaborationLevel = KernelCollaborationLevel

export type SessionAgentDefaults = KernelSessionAgentDefaults

export type WaitingRoomPublicSessionSummary = KernelWaitingRoomPublicSessionSummary

export type WaitingRoomSessionActivitySummary = KernelWaitingRoomSessionActivitySummary

export type WaitingRoomPublicItemActivitySummary = KernelWaitingRoomPublicItemActivitySummary

export type WaitingRoomPublicAgentSummary = KernelWaitingRoomPublicAgentSummary

export type WaitingRoomPublicWorkflowSummary = KernelWaitingRoomPublicWorkflowSummary

export type WaitingRoomPublicWorkflowNodeSummary = KernelWaitingRoomPublicWorkflowNodeSummary

export type WaitingRoomPublicWorkflowEdgeSummary = KernelWaitingRoomPublicWorkflowEdgeSummary

export type WaitingRoomPublicWorkflowEndpointSummary = KernelWaitingRoomPublicWorkflowEndpointSummary

export type WaitingRoomRelayStatusView = KernelRelayStatus

export type WaitingRoomRemoteMachineView = KernelRemoteMachineRecord

export type WaitingRoomRemoteKernelView = KernelRelayKernelPresence

export type WaitingRoomTerminalView = KernelTerminalRecord

export type WaitingRoomPublicSnapshot = KernelWaitingRoomPublicSnapshot

export type WorkspaceLinkAttachment = KernelWorkspaceLinkAttachment

export type WorkspaceLinkDefinition = KernelWorkspaceLinkDefinition

export type WorkspaceLiveSyncTargetStatus = KernelWorkspaceLiveSyncTargetStatus

export type WorkspaceLiveSyncGroupStatus = KernelWorkspaceLiveSyncGroupStatus

export type WorkspaceLiveSyncStatus = KernelWorkspaceLiveSyncStatus

export type AgentPromptState = KernelAgentPromptState

export type AgentRuntimeActivity = KernelAgentRuntimeActivity

export type AgentQueuedPromptControl = KernelAgentQueuedPromptControl

export type CompletedGitTurnActionProjection = KernelCompletedGitTurnActionProjection

export type WorkspaceLiveSyncApplyStatus = KernelWorkspaceLiveSyncApplyStatus

export type WorkspaceLiveSyncPathApplyResult = KernelWorkspaceLiveSyncPathApplyResult

export type TurnUndoResult = KernelTurnUndoResult

export type AgentForkPayload = {
  source_agent_id: string
  agent: AgentInstance
  provider_run: RuntimeProviderRun
  session: RuntimeSession
}

export type RuntimeInteraction = KernelRuntimeInteraction

export type RuntimeInteractionChoice = KernelRuntimeInteractionChoice

export type RuntimeInteractionCustomChoice = KernelRuntimeInteractionCustomChoice

export type SessionConfigState = KernelSessionConfigState

export type ArrobaUserConfig = KernelArrobaUserConfig

export type ArrobaUserConfigPayload = KernelArrobaUserConfigPayload

export type ArrobaUserConfigSchemaPayload = KernelArrobaUserConfigSchemaPayload

export type UserConfigSchemaEntry = KernelUserConfigSchemaEntry

export type UserConfigMutationEffect = KernelUserConfigMutationEffect

export type SliceDisplayEndpoint = KernelSliceDisplayEndpoint

export type SliceRelayEndpoint = KernelSliceRelayEndpoint

export type SliceRecord = KernelSliceRecord

export type SliceSavedStateRecord = KernelSliceSavedStateRecord

export type SliceBackupRecord = KernelSliceBackupRecord

export type SliceLocalDockerPorts = KernelSliceLocalDockerPorts

export type SliceLogEntry = KernelSliceLogEntry

export type AgentInstance = KernelAgentInstance

export type RemoteExtensionManifestSyncStatus = KernelRemoteExtensionManifestSyncStatus

export type AgentSubstituteProfile = KernelAgentSubstituteProfile

export type AgentSubstitutionRecord = KernelAgentSubstitutionRecord

export type PromptQueueItem = KernelPromptQueueItem

export type RuntimeAttachment = KernelRuntimeAttachment

export type RuntimeProviderRun = KernelRuntimeProviderRun

export type ProviderProcessInfo = KernelProviderProcessInfo

export type ProviderAuthStatus = KernelProviderAuthStatus

export type ProviderLoginStart = KernelProviderLoginStart

export type ProviderLogoutResult = KernelProviderLogoutResult

export type PromptAttachmentPart = KernelPromptAttachmentPart

export type StoredTransferArtifact = KernelStoredTransferArtifact

export type CaptureScreenshotResult = KernelCaptureScreenshotResult

export type RuntimeNoticeRecord = KernelRuntimeNoticeRecord

export type TerminalOutputRecord = KernelTerminalOutputRecord

export type PromptSubmittedPayload = KernelPromptSubmittedPayload

export type QueuedPromptSteeredPayload = KernelQueuedPromptSteeredPayload

export type QueuedPromptCancelledPayload = KernelQueuedPromptCancelledPayload

export type QueuedPromptUpdatedPayload = KernelQueuedPromptUpdatedPayload

export type SessionHistoryPageEntry = KernelSessionHistoryPageEntry

export type SessionHistoryEntry = KernelSessionHistoryEntry

export type SessionHistoryExternalObservation = KernelSessionHistoryExternalObservation

export type SessionHistoryPromptAttachment = KernelSessionHistoryPromptAttachment

export type SessionHistoryOutline = KernelSessionHistoryOutline

export type SessionHistoryOutlineAgent = KernelSessionHistoryOutlineAgent

export type SessionHistoryOutlineCursor = KernelSessionHistoryOutlineCursor

export type SessionHistoryCursorState = {
  agentId: string
  cursor: SessionHistoryOutlineCursor
} | null

export type SessionHistoryOutlineTurn = KernelSessionHistoryOutlineTurn

export type SessionHistoryOutlineBlob = KernelSessionHistoryOutlineBlob

export type SessionHistoryBlobContent = KernelSessionHistoryBlobContent

export type PromptInputHistoryEntryKind = KernelPromptInputHistoryEntryKind

export type PromptInputHistoryEntry = KernelPromptInputHistoryEntry

export type PromptInputHistoryPage = KernelPromptInputHistoryPage

export type TranscriptEntry = {
  id: number
  role: "user" | "assistant" | "reasoning" | "tool" | "error" | "status" | "notice" | "turn_toggle"
  text: string
  promptId?: string | null
  sourceAttachmentId?: string | null
  attachments?: SessionHistoryPromptAttachment[]
  queuedPrompt?: {
    promptId: string
    agentId: string
    status: string
    attachmentCount: number
    steerDisabled: boolean
    canSteer: boolean
    canCancel: boolean
    steerDisabledReason: string | null
    cancelDisabledReason: string | null
  }
  sourceText?: string
  mergeKey?: string
  providerRunId?: string | null
  source?: "external_provider_observed" | string | null
  externalProvider?: string | null
  externalProviderSessionId?: string | null
  externalProviderTurnId?: string | null
  observedAtMs?: number | null
  externalObservation?: SessionHistoryExternalObservation | null
  emphasis?: "muted" | "warning" | "error"
  turnTracking?: "none"
  turnId?: number
  hidden?: boolean
  toggleMode?: "expand" | "collapse"
  blobCollapsible?: boolean
  blobCollapsed?: boolean
  blobTitle?: string
  blobSummary?: string
  historyBlobId?: string
  historyBlobAgentId?: string
  historyBlobSourceId?: string
  historyBlobSourceAgentId?: string
  historyBlobLoaded?: boolean
  historyBlobLoading?: boolean
  historyBlobError?: string
  historyDeferred?: boolean
  historyTurnCompletedAtMs?: number | null
  historyEntryIndex?: number
  historyFragmentStart?: number
  historyFragmentEnd?: number
  historyTotalChars?: number
}

export type WorkflowDefinition = KernelWorkflowDefinition

export type WorkflowSchemaDefinition = KernelWorkflowSchemaDefinition

export type WorkflowEndpointDefinition = KernelWorkflowEndpointDefinition

export type WorkflowScheduleTrigger = KernelWorkflowScheduleTrigger

export type WorkflowScheduleDefinition = KernelWorkflowScheduleDefinition

export type WorkflowWatchdogDefinition = KernelWorkflowWatchdogDefinition

export type WorkflowPromptQueueDefinition = KernelWorkflowPromptQueueDefinition

export type WorkflowQueuedPrompt = KernelWorkflowQueuedPrompt

export type WorkflowNodeDefinition = KernelWorkflowNodeDefinition

export type WorkflowEdgeDefinition = KernelWorkflowEdgeDefinition

export type WorkflowMessage = KernelWorkflowMessage

export type WorkflowNodeRun = KernelWorkflowNodeRun

export type WorkflowFailureEvent = KernelWorkflowFailureEvent

export type WorkflowRun = KernelWorkflowRun

export type WorkflowConsoleEntry = KernelWorkflowConsoleEntry

export type WorkflowConsole = KernelWorkflowConsole

export type ReadDirectoryTreeResult = {
  session_id: string
  root_path: string
  entries: DirectoryTreeEntry[]
}

export type CliOptions = {
  kernelUrl?: string
  socketPath?: string
  automationSocket?: string
  relayUrl?: string
  relayToken?: string
  targetDaemonId?: string
  targetDaemonAlias?: string
  detached?: boolean
  sessionId?: string
  createSession?: boolean
  deleteSessionRef?: string
  alias?: string
  clientId: string
  provider?: string
  model: string
  accountProfile: string
  effort: string
  workspace?: string
  worktree?: string
}

export type SessionBinding = {
  session: RuntimeSession
  attachment: RuntimeAttachment
  providerRun: RuntimeProviderRun | null
  createdSession: boolean
  historyEntries: TranscriptEntry[]
  promptHistoryEntries: string[]
  nextHistoryCursor: SessionHistoryCursorState
}

export type BootstrapDeferredState = {
  providerCatalog?: Promise<ProviderCatalog>
  providerCommandCatalogs?: Promise<ProviderCommandCatalogs>
  terminalCommandCatalog?: Promise<TerminalCommandCatalog>
  attachedHistory?: Promise<{
    sessionId: string
    visibleAgentId: string | null
    agentEntries: Record<string, TranscriptEntry[]>
    historyEntries: TranscriptEntry[]
    promptHistoryEntries: string[]
    nextHistoryCursor: SessionHistoryCursorState
  }>
}

export type BootstrapState = {
  client: LocalIpcClient
  binding: SessionBinding | null
  sessions: RuntimeSession[]
  providerCatalog: ProviderCatalog
  providerCommandCatalogs: ProviderCommandCatalogs
  terminalCommandCatalog: TerminalCommandCatalog | null
  options: CliOptions
  preferences: ArrobaPreferences
  themeRegistry?: ThemeRegistry
  deferred?: BootstrapDeferredState
}

export function normalizeAgentPromptState(
  state: Partial<AgentPromptState> | null | undefined,
): AgentPromptState {
  return normalizeKernelAgentPromptState(
    state as Partial<KernelAgentPromptState> | null | undefined,
  ) as unknown as AgentPromptState
}

export function normalizeRuntimeSession(session: RuntimeSession): RuntimeSession {
  return normalizeKernelRuntimeSession(session as unknown as KernelRuntimeSession) as unknown as RuntimeSession
}

export function normalizeRuntimeSessionWithAgentActivity(payload: {
  session: RuntimeSession
  agent_activity?: RuntimeSession["agent_activity"] | null | undefined
  agent_activity_revision?: number | null | undefined
}): RuntimeSession {
  return normalizeKernelRuntimeSessionWithAgentActivity(
    payload as unknown as {
      session: KernelRuntimeSession
      agent_activity?: KernelRuntimeSession["agent_activity"] | null | undefined
      agent_activity_revision?: number | null | undefined
    },
  ) as unknown as RuntimeSession
}

export function normalizeRuntimeSessions(sessions: RuntimeSession[]): RuntimeSession[] {
  return normalizeKernelRuntimeSessions(sessions as unknown as KernelRuntimeSession[]) as unknown as RuntimeSession[]
}
