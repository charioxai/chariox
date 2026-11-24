import type { MachineAllocationCapability } from "./machine-allocation-api.js"
import { listProviderAccountCopyOffers } from "./provider-api.js"
import type { ProviderAccountCopyOffer, RelayKernelPresence, RoomListProjectSummary } from "@chariox/kernel-client"
import { externalProviderSessionPage } from "@chariox/kernel-client/external-provider-sessions"
import type {
  SliceRecord,
  ExternalProviderSessionRecord,
  RoomListSessionSummary,
  RoomListSnapshot,
  ProviderAccountProfile,
  RoomListGitCredentialSummary,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import { getRoomListSnapshotRequest } from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"
import type { RelayStatusView, TerminalView } from "./relay-api.js"
import { remoteMachineView } from "./remote-machine-api.js"
import { listSlices } from "./slice-api.js"

export type RemoteMachineView = ReturnType<typeof remoteMachineView> & { displayFreshness?: "cached/refreshing" | "reconnecting" }

export type RemoteKernelView = RelayKernelPresence & { displayFreshness?: "cached/refreshing" | "reconnecting" }

export type KernelInventory = {
  schemaVersion: number
  inventoryVersion: string
  structuralVersion: string
  activityRevision: string
  kernelId: string
  machineId: string
  machineAlias?: string | null
  launchTarget?: {
    repositoryState?: "ready" | "not-repository" | "unborn" | null
    workspaceId: string
    worktreeId: string
  } | null
  sessions: RoomListSessionSummary[]
  recentSessionIds?: string[]
  sessionsTotal?: number
  sessionsNextCursor?: string | null
  projects?: RoomListProjectSummary[]
  relayStatus: RelayStatusView
  remoteMachines: RemoteMachineView[]
  remoteKernels: RemoteKernelView[]
  terminals: TerminalView[]
  slices: SliceRecord[]
  externalProviderSessions?: ExternalProviderSessionRecord[]
  externalProviderSessionsHasMore?: boolean
  externalProviderSessionsNextCursor?: string | null
  providerAccountCopyOffers?: ProviderAccountCopyOffer[]
  providerAccountCopyUnavailable?: boolean
  providerAccounts?: ProviderAccountProfile[]
  gitCredentials?: RoomListGitCredentialSummary[]
  machineAllocationCapability?: MachineAllocationCapability | null
}

export async function getKernelInventory(client: LocalIpcClient, getMachineCapability?: () => Promise<MachineAllocationCapability | null | undefined>): Promise<KernelInventory> {
  const response = await client.send<Record<string, unknown>>(getRoomListSnapshotRequest())
  const payload = expectVariant<{
    snapshot: RoomListSnapshot
  }>(response, "RoomListSnapshot").snapshot
  const [slices, machineAllocationCapability, accountCopy] = await Promise.all([
    listSlices(client).catch(() => []),
    getMachineCapability?.(),
    payload.relay_status.capabilities?.includes("provider_account_copy_pull_v1")
      ? listProviderAccountCopyOffers(client).then(offers => ({ offers, unavailable: false }), () => ({ offers: [], unavailable: true }))
      : Promise.resolve({ offers: [], unavailable: false }),
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
    machineId: payload.relay_status.machine_id,
    machineAlias: payload.relay_status.machine_alias ?? null,
    launchTarget: payload.launch_target
      ? {
          repositoryState: payload.launch_target.repository_state ?? null,
          workspaceId: payload.launch_target.workspace_id,
          worktreeId: payload.launch_target.worktree_id,
        }
      : null,
    sessions: (payload.sessions ?? []).map((session) => ({
      ...session,
      kernel_id: payload.relay_status.daemon_id,
      machine_id: payload.relay_status.machine_id,
      machine_alias: payload.relay_status.machine_alias ?? null,
    })).sort((left, right) => right.created_at_ms - left.created_at_ms),
    recentSessionIds: (payload.sessions ?? []).map(row => row.id),
    sessionsTotal: payload.sessions_total ?? payload.sessions?.length ?? 0,
    sessionsNextCursor: payload.sessions_next_cursor ?? null,
    projects: payload.projects ?? [],
    relayStatus: payload.relay_status,
    remoteMachines: (payload.remote_machines ?? []).map(remoteMachineView),
    remoteKernels: payload.remote_kernels ?? [],
    terminals: payload.terminals ?? [],
    slices,
    externalProviderSessions: externalProviderSessions.sessions,
    externalProviderSessionsHasMore: externalProviderSessions.hasMore,
    externalProviderSessionsNextCursor: externalProviderSessions.nextCursor,
    providerAccountCopyOffers: accountCopy.offers,
    providerAccountCopyUnavailable: accountCopy.unavailable,
    providerAccounts: payload.provider_accounts ?? [],
    gitCredentials: payload.git_credentials ?? [],
    ...(machineAllocationCapability !== undefined ? { machineAllocationCapability } : {}),
  }
}
