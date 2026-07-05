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
  ArrobaUserConfig as KernelArrobaUserConfig,
  ArrobaUserConfigPayload as KernelArrobaUserConfigPayload,
  ArrobaUserConfigSchemaPayload as KernelArrobaUserConfigSchemaPayload,
  CollaborationLevel as KernelCollaborationLevel,
  AgentPromptState as KernelAgentPromptState,
  AgentQueuedPromptControl as KernelAgentQueuedPromptControl,
  AgentRuntimeActivity as KernelAgentRuntimeActivity,
  CompletedGitTurnActionProjection as KernelCompletedGitTurnActionProjection,
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
  RuntimeProviderRun as KernelRuntimeProviderRun,
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
  RelayKernelPresence as KernelRelayKernelPresence,
  RelayStatus as KernelRelayStatus,
  RemoteMachineRecord as KernelRemoteMachineRecord,
  TerminalRecord as KernelTerminalRecord,
  TerminalCommandCatalog,
  TerminalOutputRecord as KernelTerminalOutputRecord,
  TurnUndoResult as KernelTurnUndoResult,
  UserConfigMutationEffect as KernelUserConfigMutationEffect,
  UserConfigSchemaEntry as KernelUserConfigSchemaEntry,
  QueuedPromptCancelledPayload as KernelQueuedPromptCancelledPayload,
  QueuedPromptSteeredPayload as KernelQueuedPromptSteeredPayload,
  QueuedPromptUpdatedPayload as KernelQueuedPromptUpdatedPayload,
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

export type ArrobaMcpServerConfig = {
  name: string
  transport: Record<string, unknown>
  enabled?: boolean
  required?: boolean
  startup_timeout_sec?: number | null
  tool_timeout_sec?: number | null
  enabled_tools?: string[] | null
  disabled_tools?: string[] | null
  tools?: Record<string, unknown>
}

export type McpImportSkip = {
  name: string
  reason: string
}

export type McpImportOutcome = {
  imported: ArrobaMcpServerConfig[]
  skipped: McpImportSkip[]
}

export type ArrobaSkillMetadata = {
  name: string
  description: string
  short_description?: string | null
  path: string
}

export type ArrobaEnvironmentConfig = {
  name: string
  runtime: Record<string, unknown>
}

export type ArrobaScriptMetadata = {
  name: string
  runtime: "python" | "typescript" | string
  path: string
  description: string
  input_schema: Record<string, unknown>
  definition_hash: string
  timeout_sec?: number | null
}

export type ArrobaConnectorDefinition = {
  kind: "connector" | string
  name: string
  description: string
  adapter: string
  credential?: { required?: boolean } | null
  timeout_ms?: number | null
  max_response_bytes?: number | null
  operations: Array<Record<string, unknown>>
}

export type ArrobaConnectorAdapterDefinition = {
  kind: "connector_adapter" | string
  name: string
  version?: string | null
  adapter_protocol: string
  command: string
  args?: string[]
  description?: string | null
  source?: "user" | "bundled" | string | null
  manifest_path?: string | null
}

export type ArrobaCredentialConfig = {
  id: string
  description?: string | null
  source: Record<string, unknown>
  allowed_hosts?: string[]
  allowed_uses?: string[]
  injection: Record<string, unknown>
}

export type ExtensionKind = "mcp" | "skill" | "script" | "connector"

export type ExtensionGrant = {
  kind: ExtensionKind
  name: string
  environment?: string | null
  credential?: string | null
  max_safety?: "read" | "write" | "destructive" | string | null
}

export type SkillImportSkip = {
  name: string
  path: string
  reason: string
}

export type SkillImportOutcome = {
  imported: ArrobaSkillMetadata[]
  skipped: SkillImportSkip[]
}

export type RuntimeSession = {
  id: string
  alias?: string | null
  workspace_id: string
  worktree_id: string
  owner_user_id?: string
  host_machine_id?: string | null
  host_daemon_id?: string | null
  members?: SessionMember[]
  invites?: SessionInvite[]
  workspace_label?: string | null
  directory?: string | null
  worktree_label?: string | null
  created_at_ms: number
  last_used_at_ms?: number | null
  last_prompt_sent_at_ms?: number | null
  hidden?: boolean
  status: string
  agent_defaults?: SessionAgentDefaults
  active_provider_run_id: string | null
  attachment_ids: string[]
  active_prompt: PromptQueueItem | null
  queued_prompts: PromptQueueItem[]
  prompt_states?: Record<string, AgentPromptState>
  agent_activity?: Record<string, AgentRuntimeActivity>
  agent_activity_revision?: number
  active_interactions?: RuntimeInteraction[]
  metaagent_tasks?: MetaagentTask[]
  focused_agent_id: string | null
  max_agents: number
  agents: AgentInstance[]
  collaboration_agent_counts?: SessionCollaborationAgentCounts | null
  config_state: SessionConfigState
  workflows?: WorkflowDefinition[]
  workflow_runs?: WorkflowRun[]
  workflow_prompt_queues?: WorkflowPromptQueueDefinition[]
  workflow_queued_prompts?: WorkflowQueuedPrompt[]
  workflow_schedules?: WorkflowScheduleDefinition[]
  workflow_watchdogs?: WorkflowWatchdogDefinition[]
  workflow_consoles?: WorkflowConsole[]
  workspace_links?: WorkspaceLinkDefinition[]
  workspace_live_sync_mode?: "managed" | "tracked" | "unrestricted" | null
  external_provider_imports?: ExternalProviderImportMetadata[]
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

export type WorkspaceLinkAttachment = {
  link_id: string
  user_id: string
  machine_id: string
  kernel_id: string
  repo_root: string
  branch?: string | null
  repo_fingerprint?: string | null
  attached_at_ms: number
}

export type WorkspaceLinkDefinition = {
  link_id: string
  session_id: string
  name: string
  created_by_user_id: string
  created_at_ms: number
  attachments?: WorkspaceLinkAttachment[]
}

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

export type SliceDisplayEndpoint = {
  slice_id: string
  kind: "novnc" | "arroba_viewer" | "external"
  url: string
  access: "local" | "tunnel" | "public"
  expires_at_ms?: number | null
  capabilities?: string[]
}

export type SliceRelayEndpoint = {
  url: string
  private?: boolean
}

export type SliceRecord = {
  id: string
  name: string
  owner_kernel_id: string
  owner_machine_id: string
  backend: "local_docker" | "ssh_docker"
  os: string
  status: "stopped" | "starting" | "stopping" | "running" | "unhealthy"
  workspace_mount?: string | null
  workspace_id?: string | null
  worktree_id?: string | null
  session_ids?: string[]
  agent_ids?: string[]
  display_mode?: "headless" | "headed"
  last_operation?: string | null
  last_operation_status?: "accepted" | "in_progress" | "completed" | "failed" | "reconciled" | null
  last_error?: string | null
  last_operation_at_ms?: number | null
  worker_kernel_ref: string
  worker_kernel_id?: string | null
  worker_machine_id?: string | null
  relay_endpoint?: SliceRelayEndpoint | null
  local_docker_ports?: SliceLocalDockerPorts | null
  providers?: string[]
  provider_auth?: Array<{
    provider: string
    state: "unknown" | "not_configured" | "configured" | "authenticated"
    alias?: string | null
    account_id?: string | null
    email?: string | null
    organization_id?: string | null
    organization_name?: string | null
    subscription_type?: string | null
    auth_type?: string | null
    source?: string | null
    checked_at_ms?: number | null
  }>
  saved_state_ref?: string | null
  saved_state_status?: "saved" | "missing" | "failed" | null
  saved_state_updated_at_ms?: number | null
  display_endpoint?: SliceDisplayEndpoint | null
  created_at_ms: number
  updated_at_ms: number
}

export type SliceSavedStateRecord = {
  id: string
  slice_name: string
  source_slice_id: string
  backend: "local_docker" | "ssh_docker"
  os: string
  image_ref: string
  home_archive_path: string
  created_at_ms: number
  updated_at_ms: number
  last_operation?: string | null
  last_operation_status?: "accepted" | "in_progress" | "completed" | "failed" | "reconciled" | null
  last_error?: string | null
}

export type SliceBackupRecord = {
  id: string
  name: string
  source_slice_id: string
  source_state_id: string
  image_ref: string
  home_archive_path: string
  created_at_ms: number
  size_bytes?: number | null
}

export type SliceLocalDockerPorts = {
  codex: number
  opencode: number
  kernel: number
  mcp: number
  relay: number
  novnc: number
  codex_range_start: number
  opencode_range_start: number
}

export type SliceLogEntry = {
  source: string
  path?: string | null
  text: string
  truncated?: boolean
}

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

export type StoredTransferArtifact = {
  artifact_id: string
  stored_path: string
  display_name: string
}

export type CaptureScreenshotResult = {
  status: string
  artifact_path: string | null
  message: string
}

export type RuntimeNoticeRecord = {
  message: string
}

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
