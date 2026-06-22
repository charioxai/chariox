import type {
  SliceRecord,
  ExternalProviderSessionRecord,
  WaitingRoomPublicSessionSummary,
  WaitingRoomPublicSnapshot,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import { getWaitingRoomPublicSnapshotRequest } from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"
import type { RelayStatusView, TerminalView } from "./relay-api.js"
import { listSlices } from "./slice-api.js"

export type RemoteMachineView = {
  machine_id: string
  machine_alias?: string | null
  registry_alias?: string | null
  display_name: string
  trust_status: "approved" | "pending" | "forgotten"
  online: boolean
  pending: boolean
  kernel_count: number
  available_providers?: string[]
}

export type RemoteKernelView = {
  kernel_id: string
  machine_id: string
  machine_alias?: string | null
  kernel_alias?: string | null
  relay_alias?: string | null
  available_providers?: string[]
  capabilities?: string[]
  accepting_remote_leases?: boolean
  leased_agent_count?: number
  local_session_count?: number
}

export type WaitingRoomInventory = {
  inventoryVersion: string
  sessions: WaitingRoomPublicSessionSummary[]
  relayStatus: RelayStatusView
  remoteMachines: RemoteMachineView[]
  remoteKernels: RemoteKernelView[]
  terminals: TerminalView[]
  slices: SliceRecord[]
  externalProviderSessions?: ExternalProviderSessionRecord[]
  externalProviderSessionsHasMore?: boolean
  externalProviderSessionsNextCursor?: string | null
}

export async function getWaitingRoomInventory(client: LocalIpcClient): Promise<WaitingRoomInventory> {
  const response = await client.send<Record<string, unknown>>(getWaitingRoomPublicSnapshotRequest())
  const payload = expectVariant<{
    snapshot: WaitingRoomPublicSnapshot
  }>(response, "WaitingRoomPublicSnapshot").snapshot
  const slices = await listSlices(client).catch(() => [])
  return {
    inventoryVersion: payload.inventory_version,
    sessions: (payload.sessions ?? []).slice().sort((left, right) => right.created_at_ms - left.created_at_ms),
    relayStatus: payload.relay_status,
    remoteMachines: payload.remote_machines ?? [],
    remoteKernels: payload.remote_kernels ?? [],
    terminals: payload.terminals ?? [],
    slices,
    externalProviderSessions: payload.external_provider_sessions ?? [],
    externalProviderSessionsHasMore: payload.external_provider_sessions_has_more ?? false,
    externalProviderSessionsNextCursor: payload.external_provider_sessions_next_cursor ?? null,
  }
}
