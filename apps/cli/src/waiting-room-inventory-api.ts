import { externalProviderSessionPage } from "@arroba/kernel-client/external-provider-sessions"
import type {
  SliceRecord,
  ExternalProviderSessionRecord,
  WaitingRoomRemoteKernelView,
  WaitingRoomRemoteMachineView,
  WaitingRoomPublicSessionSummary,
  WaitingRoomPublicSnapshot,
} from "./cli-types.js"
import type { LocalIpcClient } from "./ipc.js"
import { getWaitingRoomPublicSnapshotRequest } from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"
import type { RelayStatusView, TerminalView } from "./relay-api.js"
import { listSlices } from "./slice-api.js"

export type RemoteMachineView = WaitingRoomRemoteMachineView

export type RemoteKernelView = WaitingRoomRemoteKernelView

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
  const externalProviderSessions = externalProviderSessionPage({
    sessions: payload.external_provider_sessions,
    has_more: payload.external_provider_sessions_has_more,
    next_cursor: payload.external_provider_sessions_next_cursor,
  })
  return {
    inventoryVersion: payload.inventory_version,
    sessions: (payload.sessions ?? []).slice().sort((left, right) => right.created_at_ms - left.created_at_ms),
    relayStatus: payload.relay_status,
    remoteMachines: payload.remote_machines ?? [],
    remoteKernels: payload.remote_kernels ?? [],
    terminals: payload.terminals ?? [],
    slices,
    externalProviderSessions: externalProviderSessions.sessions,
    externalProviderSessionsHasMore: externalProviderSessions.hasMore,
    externalProviderSessionsNextCursor: externalProviderSessions.nextCursor,
  }
}
