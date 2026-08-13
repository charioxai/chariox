import type { ExternalProviderSessionRecord, SliceRecord } from "./cli-types.js"
import type { BackendProviderId } from "./provider-catalog.js"
import type { ProviderAccountSummary } from "@chariox/kernel-client"
import type { ThemeName } from "./theme-registry.js"
import type { WaitingRoomProjectSummary } from "./waiting-room-projects.js"

export type WaitingRoomFocus =
  | "new"
  | "launch-machine"
  | "launch-kernel"
  | "project"
  | "project-entry"
  | "archived-projects"
  | "provider"
  | "model"
  | "effort"
  | "workspace"
  | "worktree"
  | "live-sync"
  | "collaborators"
  | "slice"
  | "slice-display"
  | "theme"
  | "join-sessions"
  | "session"
  | "external-session"
  | "external-session-more"
  | "relay"
  | "machine"
  | "remote-kernel"
  | "slice-entry"
  | "terminal"
  | "add-terminal"

export type WaitingRoomKeyState = {
  up: boolean
  down: boolean
  left: boolean
  right: boolean
}

export type WaitingRoomState = {
  focus: WaitingRoomFocus
  sessionIndex: number
  projectIndex?: number
  externalSessionIndex?: number
  machineIndex: number
  remoteKernelIndex: number
  sliceIndex?: number
  terminalIndex: number
  worktreeSelectionId: string
  workspaceLiveSyncMode: "off" | "managed" | "tracked"
  selectedMachineRef?: string
  selectedKernelRef?: string
  projectSelectionId?: string
  showArchivedProjects?: boolean
  sliceSelectionId?: string
  sliceDisplayMode?: "headless" | "headed"
  providerId: BackendProviderId
  modelId: string
  effort: string
  executionMode?: "build" | "plan"
  permissionLevel?: "required" | "yolo"
  themeId: ThemeName
  introStep: number
  keyState: WaitingRoomKeyState
}

export type WaitingRoomRemoteMachine = {
  machine_id: string
  machine_alias?: string | null
  registry_alias?: string | null
  display_name?: string
  trust_status?: "approved" | "pending" | "forgotten"
  online?: boolean
  kernel_count: number
  available_providers?: string[]
  provider_accounts?: ProviderAccountSummary[]
  pending?: boolean
}

export type WaitingRoomRemoteKernel = {
  kernel_id: string
  machine_id: string
  machine_alias?: string | null
  kernel_alias?: string | null
  relay_alias?: string | null
  available_providers?: string[]
  provider_accounts?: ProviderAccountSummary[]
  accepting_remote_leases?: boolean
  leased_agent_count?: number
  local_session_count?: number
}

export type WaitingRoomRemoteState = {
  workspaceId?: string
  inventoryStatus?: "loading" | "ready" | "error"
  loadingFrame?: number
  cloudNotice?: string | null
  collaborationBackend?: "cloud" | "relay" | "local"
  relay?: {
    configured: boolean
    connected: boolean
    relay_url?: string | null
  } | null
  machines?: WaitingRoomRemoteMachine[]
  kernels?: WaitingRoomRemoteKernel[]
  terminals?: WaitingRoomTerminal[]
  slices?: SliceRecord[]
  externalProviderSessions?: ExternalProviderSessionRecord[]
  externalProviderSessionsHasMore?: boolean
  externalProviderSessionsNextCursor?: string | null
  projects?: WaitingRoomProjectSummary[]
}

export type WaitingRoomTerminalType = "cli" | "web" | "ios" | "android"

export type WaitingRoomTerminal = {
  terminal_id: string
  terminal_type: WaitingRoomTerminalType
  alias?: string | null
  paired_at_ms: number
  revoked: boolean
}

export type WaitingRoomTargetState = {
  workspacePath: string
  worktreePath: string
}

export type WaitingRoomRow = {
  id: string
  title: string
  value: string
  titleWidth: number
  columns?: string[]
  indent: number
  focused: boolean
  selectable: boolean
  scrollbar: string
}
