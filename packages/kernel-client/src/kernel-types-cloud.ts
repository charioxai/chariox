import type { ExternalProviderSessionRecord } from "./external-provider-sessions.js"
import type {
  WaitingRoomPublicProjectSummary,
  WaitingRoomPublicSessionSummary,
} from "./kernel-types-session.js"

export type RelayStatus = {
  configured: boolean
  connected: boolean
  relay_url?: string | null
  relay_token_configured: boolean
  daemon_id: string
  daemon_alias?: string | null
  machine_id: string
  machine_alias?: string | null
}

export type CloudRelayProfile = {
  api_url: string
  email: string
  account_id: string
  user_id: string
  account_slug: string
  realm_id: string
  relay_url: string
  issuer_id: string
  client_id?: string | null
  client_alias?: string | null
  machine_id?: string | null
  machine_alias?: string | null
  machine_credential?: string | null
  cloud_session_token?: string | null
  cloud_session_expires_at_ms?: number | null
  token_expires_at_ms?: number | null
}

export type CloudRelayLoginStart = {
  api_url: string
  device_code: string
  user_code: string
  verification_url: string
  expires_at: string
  interval_seconds: number
}

export type CloudRelayLoginPoll = {
  status: "authorization_pending" | "expired_token" | "approved"
  interval_seconds?: number | null
  expires_at?: string | null
  profile?: CloudRelayProfile | null
}

export type CloudSessionInvite = {
  invite_id: string
  invite_token: string
  session_id: string
  account_id: string
  created_by_user_id: string
  expires_at?: string | null
  max_uses?: number | null
}

export type CloudSessionInviteDetails = {
  invite_id: string
  session_id: string
  account_id: string
  created_by_user_id: string
  display_name?: string | null
  expires_at?: string | null
  max_uses?: number | null
  used_count: number
  status: string
}

export type CloudSessionInviteAcceptance = {
  session_id: string
  account_id: string
  user_id: string
  invited_by_user_id: string
  joined_at: string
}

export type CloudSessionMember = {
  user_id: string
  email: string
  display_name?: string | null
  invited_by_user_id?: string | null
  joined_at: string
}

export type CloudCollaborator = {
  user_id: string
  email: string
  display_name?: string | null
  last_collaborated_at: string
  shared_session_count: number
}

export type CloudRelayRuntimeToken = {
  relay_url: string
  relay_token: string
  token_expires_at: string
}

export type KernelClientConnection = {
  relay_url: string
  relay_token: string
  target_daemon_id?: string | null
  target_daemon_alias?: string | null
  token_expires_at?: string | null
  machine_id?: string | null
  kernel_id?: string | null
}

export type RemoteMachineRecord = {
  machine_id: string
  machine_alias?: string | null
  registry_alias?: string | null
  display_name: string
  trust_status: "approved" | "pending" | "forgotten"
  online: boolean
  pending: boolean
  kernel_count: number
  available_providers?: string[]
  provider_accounts?: ProviderAccountSummary[]
}

export type SliceRecord = {
  id: string
  name: string
  owner_kernel_id: string
  owner_machine_id: string
  session_id?: string | null
  session_ids?: string[]
  agent_ids?: string[]
  backend: "local_docker" | "ssh_docker"
  os: string
  display_mode?: "headless" | "headed"
  status: "stopped" | "starting" | "stopping" | "running" | "unhealthy"
  last_operation?: string | null
  last_operation_status?: "accepted" | "in_progress" | "completed" | "failed" | "reconciled" | null
  last_error?: string | null
  last_operation_at_ms?: number | null
  workspace_id?: string | null
  worktree_id?: string | null
  workspace_mount?: string | null
  worker_kernel_ref: string
  worker_kernel_id?: string | null
  worker_machine_id?: string | null
  relay_endpoint?: SliceRelayEndpoint | null
  local_docker_ports?: SliceLocalDockerPorts | null
  providers?: string[]
  provider_auth?: SliceProviderAuthSummary[]
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

export type SliceProviderAuthSummary = {
  provider: string
  account_profile: string
  state: "unknown" | "not_configured" | "configured" | "authenticated"
  auth_type?: string | null
  account_id?: string | null
  email?: string | null
  organization_id?: string | null
  organization_name?: string | null
  subscription_type?: string | null
  source: string
}

export type SliceRelayEndpoint = {
  url: string
  private?: boolean
}

export type SliceDisplayEndpoint = {
  slice_id: string
  kind: "novnc" | "chariox_viewer" | "external"
  url: string
  access: "local" | "tunnel" | "public"
  expires_at_ms?: number | null
  capabilities?: string[]
}

export type PairedClientRecord = {
  client_id: string
  alias?: string | null
  terminal_type?: TerminalType | null
  public_key_thumbprint: string
  paired_at_ms: number
  revoked: boolean
}

export type PairingInviteIntent = "client" | "machine"
export type TerminalType = "cli" | "web" | "ios" | "android"

export type PairingInviteRecord = {
  intent: PairingInviteIntent
  invite_id: string
  invite_token: string
  relay_url: string
  target_daemon_id: string
  target_daemon_alias?: string | null
  issued_at_ms: number
  expires_at_ms: number
}

export type PairingJoinRecord = {
  intent: PairingInviteIntent
  subject_id: string
  relay_url: string
  target_daemon_id: string
  alias?: string | null
  public_key_thumbprint: string
  paired_at_ms: number
}

export type TerminalRecord = {
  terminal_id: string
  terminal_type: TerminalType
  alias?: string | null
  paired_at_ms: number
  revoked: boolean
}

export type TerminalPairingLinkRecord = {
  terminal_id: string
  pairing_link: string
  pairing_code: string
  invite_id: string
  relay_url: string
  target_daemon_id: string
  target_daemon_alias?: string | null
  terminal_type: TerminalType
  issued_at_ms: number
  expires_at_ms: number
}

export type RelayKernelPresence = {
  kernel_id: string
  machine_id: string
  machine_alias?: string | null
  relay_alias?: string | null
  kernel_alias?: string | null
  available_providers?: string[]
  provider_accounts?: ProviderAccountSummary[]
  capabilities?: string[]
  accepting_remote_leases?: boolean
  leased_agent_count?: number
  local_session_count?: number
}

export type ProviderAccountSummary = {
  provider: string
  state: string
  auth_type?: string | null
  account_id?: string | null
  email?: string | null
  organization_id?: string | null
  organization_name?: string | null
  subscription_type?: string | null
  alias?: string | null
}

export type WaitingRoomGitCredentialSummary = {
  credentialId: string
  hostname: string
  label: string
}

export type WaitingRoomInventorySnapshot = {
  inventory_version: string
  structural_version: string
  activity_revision: string
  sessions: WaitingRoomPublicSessionSummary[]
  projects: WaitingRoomPublicProjectSummary[]
  relay_status: RelayStatus
  remote_machines?: RemoteMachineRecord[]
  remote_kernels?: RelayKernelPresence[]
  terminals?: TerminalRecord[]
  launch_target?: {
    workspace_id: string
    worktree_id: string
    workspace_label?: string | null
    directory?: string | null
    worktree_label?: string | null
  } | null
  provider_accounts?: import("./kernel-types-provider.js").ProviderAccountProfile[]
  git_credentials?: WaitingRoomGitCredentialSummary[]
}

export type WaitingRoomPublicSnapshot = WaitingRoomInventorySnapshot & {
  schema_version: number
  generated_at_ms: number
  external_provider_sessions?: ExternalProviderSessionRecord[]
  external_provider_sessions_has_more?: boolean
  external_provider_sessions_next_cursor?: string | null
}

export type WorkspaceWorktreeRecord = {
  path: string
  branch?: string | null
  label?: string | null
  current: boolean
}

export type WorkspaceGitCompareRef = {
  name: string
  detail?: string | null
  selected: boolean
}

export type WorkspaceGitFileChange = {
  path: string
  status: string
  additions: number
  deletions: number
}

export type WorkspaceGitChangeTotals = {
  files: number
  additions: number
  deletions: number
}

export type WorkspaceGitOverview = {
  workspace_id: string
  worktree_id: string
  repo_root?: string | null
  repo_label?: string | null
  branch?: string | null
  compare_ref: string
  compare_refs: WorkspaceGitCompareRef[]
  totals: WorkspaceGitChangeTotals
  files: WorkspaceGitFileChange[]
  generated_at_ms: number
}

export type WorkspaceRepoFileEntry = {
  path: string
  name: string
  kind: "directory" | "file" | string
  changed: boolean
  status?: string | null
  additions: number
  deletions: number
}

export type WorkspaceRepoFileListing = {
  workspace_id: string
  worktree_id: string
  path_prefix: string
  compare_ref: string
  total_entries: number
  truncated: boolean
  entries: WorkspaceRepoFileEntry[]
  generated_at_ms: number
}

export type WorkspaceFileContent = {
  workspace_id: string
  worktree_id: string
  path: string
  name: string
  language: string
  mime: string
  encoding: "utf-8" | "base64" | string
  content_text?: string | null
  content_base64?: string | null
  size_bytes: number
  mtime_ms: number
  fingerprint: string
  sha256?: string | null
  truncated: boolean
  status?: string | null
  additions: number
  deletions: number
  compare_ref: string
  generated_at_ms: number
}

export type WorkspaceGitActionResult = {
  workspace_id: string
  worktree_id: string
  action: string
  message: string
  commit_sha?: string | null
  branch?: string | null
  generated_at_ms: number
}

export type WorkspacePullRequestRecord = {
  workspace_id: string
  worktree_id: string
  branch: string
  base_ref: string
  url: string
  title?: string | null
  draft: boolean
  generated_at_ms: number
}
