import { externalProviderSessionPage } from "@chariox/kernel-client/external-provider-sessions"
import type {
  SliceRecord,
  ExternalProviderSessionRecord,
  WaitingRoomRemoteKernelView,
  WaitingRoomRemoteMachineView,
  WaitingRoomPublicSessionSummary,
  WaitingRoomPublicSnapshot,
  ProviderAccountProfile,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import { getWaitingRoomPublicSnapshotRequest } from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"
import type { RelayStatusView, TerminalView } from "./relay-api.js"
import { listSlices } from "./slice-api.js"
import type { WaitingRoomProjectSummary } from "./waiting-room-projects.js"
import { listManagedEnvironmentCatalog } from "./managed-environment-api.js"
import type { ManagedEnvironmentCatalog } from "@chariox/kernel-client/ipc-managed-environment-requests"

export type RemoteMachineView = WaitingRoomRemoteMachineView

export type RemoteKernelView = WaitingRoomRemoteKernelView

export type WaitingRoomInventory = {
  schemaVersion: number
  inventoryVersion: string
  structuralVersion: string
  activityRevision: string
  kernelId: string
  kernelAlias?: string | null
  machineId: string
  machineAlias?: string | null
  launchTarget?: {
    workspaceId: string
    worktreeId: string
  } | null
  sessions: WaitingRoomPublicSessionSummary[]
  projects?: WaitingRoomProjectSummary[]
  relayStatus: RelayStatusView
  remoteMachines: RemoteMachineView[]
  remoteKernels: RemoteKernelView[]
  terminals: TerminalView[]
  slices: SliceRecord[]
  externalProviderSessions?: ExternalProviderSessionRecord[]
  externalProviderSessionsHasMore?: boolean
  externalProviderSessionsNextCursor?: string | null
  providerAccounts?: ProviderAccountProfile[]
  managedEnvironmentCatalog?: ManagedEnvironmentCatalog
}

export async function getWaitingRoomInventory(client: LocalIpcClient): Promise<WaitingRoomInventory> {
  const response = await client.send<Record<string, unknown>>(getWaitingRoomPublicSnapshotRequest())
  const payload = expectVariant<{
    snapshot: WaitingRoomPublicSnapshot
  }>(response, "WaitingRoomPublicSnapshot").snapshot
  const [slices, managedEnvironmentCatalog] = await Promise.all([
    listSlices(client).catch(() => []),
    listManagedEnvironmentCatalog(client).catch(() => undefined),
  ])
  const externalProviderSessions = externalProviderSessionPage({
    ...(payload.external_provider_sessions !== undefined ? { sessions: payload.external_provider_sessions } : {}),
    ...(payload.external_provider_sessions_has_more !== undefined ? { has_more: payload.external_provider_sessions_has_more } : {}),
    ...(payload.external_provider_sessions_next_cursor !== undefined ? { next_cursor: payload.external_provider_sessions_next_cursor } : {}),
  })
  return {
    schemaVersion: payload.schema_version,
    inventoryVersion: payload.inventory_version,
    structuralVersion: payload.structural_version,
    activityRevision: payload.activity_revision,
    kernelId: payload.relay_status.daemon_id,
    kernelAlias: payload.relay_status.daemon_alias ?? null,
    machineId: payload.relay_status.machine_id,
    machineAlias: payload.relay_status.machine_alias ?? null,
    launchTarget: payload.launch_target
      ? {
          workspaceId: payload.launch_target.workspace_id,
          worktreeId: payload.launch_target.worktree_id,
        }
      : null,
    sessions: (payload.sessions ?? []).map((session) => ({
      ...session,
      kernel_id: payload.relay_status.daemon_id,
      kernel_alias: payload.relay_status.daemon_alias ?? null,
      machine_id: payload.relay_status.machine_id,
      machine_alias: payload.relay_status.machine_alias ?? null,
    })).sort((left, right) => right.created_at_ms - left.created_at_ms),
    projects: payload.projects ?? [],
    relayStatus: payload.relay_status,
    remoteMachines: payload.remote_machines ?? [],
    remoteKernels: payload.remote_kernels ?? [],
    terminals: payload.terminals ?? [],
    slices,
    externalProviderSessions: externalProviderSessions.sessions,
    externalProviderSessionsHasMore: externalProviderSessions.hasMore,
    externalProviderSessionsNextCursor: externalProviderSessions.nextCursor,
    providerAccounts: payload.provider_accounts ?? [],
    ...(managedEnvironmentCatalog ? { managedEnvironmentCatalog } : {}),
  }
}
