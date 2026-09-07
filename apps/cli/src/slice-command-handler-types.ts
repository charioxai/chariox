import type { ResolvedAgentReference } from "@chariox/kernel-client/session-agent-resolver"
import type {
  SliceBackupRecord,
  SliceDisplayEndpoint,
  SliceLogEntry,
  SliceRecord,
  SliceSavedStateRecord,
} from "./cli-types.js"

export type FooterTone = "info" | "error"

export type SliceCreateOptions = {
  name: string
  backend?: "local_docker" | "ssh_docker"
  os?: string
  displayMode?: "headless" | "headed"
  workspaceId?: string | null
  worktreeId?: string | null
  workspaceMount?: string | null
  workerKernelRef?: string | null
  displayUrl?: string | null
  fromSavedState?: string | null
  base?: "default" | "clean" | null
}

export type SliceProviderLogin = {
  provider: string
  login_kind: string
  auth_url?: string | null
  verification_url?: string | null
  user_code?: string | null
  status: string
  message: string
}

export type SliceCommandHandlerDeps = {
  currentWorkspaceTarget: () => string
  currentWorktreeTarget: () => string
  focusedAgentId: () => string | null
  resolveSessionAgent: (reference?: string | null) => ResolvedAgentReference
  flashFooter: (message: string, tone: FooterTone) => void
  appendNotice: (message: string) => void
  openExternalUrl?: (url: string) => Promise<boolean>
  listSlices?: () => Promise<SliceRecord[]>
  createSlice?: (options: SliceCreateOptions) => Promise<SliceRecord>
  getSlice?: (sliceRef: string) => Promise<SliceRecord>
  startSlice?: (sliceRef: string) => Promise<SliceRecord>
  stopSlice?: (sliceRef: string) => Promise<SliceRecord>
  deleteSlice?: (sliceRef: string) => Promise<SliceRecord>
  importSliceProviderAuth?: (sliceRef: string, provider: string, accountProfile: string) => Promise<{ slice: SliceRecord; provider: string; status: string }>
  removeSliceProviderAuth?: (sliceRef: string, provider: string, accountProfile: string) => Promise<{ slice: SliceRecord; provider: string; status: string }>
  startSliceProviderLogin?: (sliceRef: string, provider: string, accountProfile: string) => Promise<{ slice: SliceRecord; login: SliceProviderLogin }>
  getSliceDisplayEndpoint?: (sliceRef: string) => Promise<SliceDisplayEndpoint>
  getSliceLogs?: (sliceRef: string, tailLines?: number | null) => Promise<{ slice: SliceRecord; entries: SliceLogEntry[] }>
  listSliceAudit?: (sliceRef: string, limit?: number | null) => Promise<Record<string, unknown>[]>
  saveSliceState?: (sliceRef: string, mode?: "restart_agents" | "shutdown" | null, scope?: "this_slice" | "future_slices" | null) => Promise<{ slice: SliceRecord; state: SliceSavedStateRecord }>
  getSliceStateStatus?: (sliceRef: string) => Promise<{ slice: SliceRecord; state: SliceSavedStateRecord | null }>
  resetSliceState?: (sliceRef: string) => Promise<{ slice: SliceRecord; removed_state: SliceSavedStateRecord | null }>
  createSliceBackup?: (sliceRef: string, name?: string | null) => Promise<{ slice: SliceRecord; backup: SliceBackupRecord; instructions: string }>
  restoreSliceBackup?: (sliceRef: string, backupRef: string) => Promise<{ slice: SliceRecord; backup: SliceBackupRecord }>
}
