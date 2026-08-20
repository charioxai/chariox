import type { LocalIpcClient } from "./ipc.js"
import type {
  SliceBackupRecord,
  SliceDisplayEndpoint,
  SliceLogEntry,
  SliceRecord,
  SliceSavedStateRecord,
} from "./cli-types.js"
import {
  createSliceBackupRequest,
  createSliceRequest,
  deleteSliceRequest,
  getSliceDisplayEndpointRequest,
  getSliceLogsRequest,
  getSliceRequest,
  getSliceStateStatusRequest,
  importSliceProviderAuthRequest,
  listSliceAuditRequest,
  listSlicesRequest,
  removeSliceProviderAuthRequest,
  resetSliceStateRequest,
  saveSliceStateRequest,
  startSliceProviderLoginRequest,
  startSliceRequest,
  stopSliceRequest,
} from "./ipc-requests.js"
import { expectVariant } from "./ipc-response.js"

export async function listSlices(client: LocalIpcClient): Promise<SliceRecord[]> {
  const response = await client.send<Record<string, unknown>>(listSlicesRequest())
  return expectVariant<{ slices: SliceRecord[] }>(response, "SlicesListed").slices
}

export async function createSlice(
  client: LocalIpcClient,
  options: {
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
  },
): Promise<SliceRecord> {
  const response = await client.send<Record<string, unknown>>(createSliceRequest(options))
  return expectVariant<{ slice: SliceRecord }>(response, "SliceCreated").slice
}

export async function getSlice(client: LocalIpcClient, sliceRef: string): Promise<SliceRecord> {
  const response = await client.send<Record<string, unknown>>(getSliceRequest(sliceRef))
  return expectVariant<{ slice: SliceRecord }>(response, "Slice").slice
}

export async function startSlice(client: LocalIpcClient, sliceRef: string): Promise<SliceRecord> {
  const response = await client.send<Record<string, unknown>>(startSliceRequest(sliceRef))
  return expectVariant<{ slice: SliceRecord }>(response, "SliceStarted").slice
}

export async function stopSlice(client: LocalIpcClient, sliceRef: string): Promise<SliceRecord> {
  const response = await client.send<Record<string, unknown>>(stopSliceRequest(sliceRef))
  return expectVariant<{ slice: SliceRecord }>(response, "SliceStopped").slice
}

export async function deleteSlice(client: LocalIpcClient, sliceRef: string): Promise<SliceRecord> {
  const response = await client.send<Record<string, unknown>>(deleteSliceRequest(sliceRef))
  return expectVariant<{ slice: SliceRecord }>(response, "SliceDeleted").slice
}

export async function importSliceProviderAuth(
  client: LocalIpcClient,
  sliceRef: string,
  provider: string,
  accountProfile: string,
): Promise<{ slice: SliceRecord; provider: string; status: string }> {
  const response = await client.send<Record<string, unknown>>(importSliceProviderAuthRequest(sliceRef, provider, accountProfile))
  return expectVariant<{ slice: SliceRecord; provider: string; status: string }>(response, "SliceProviderAuthImported")
}

export async function removeSliceProviderAuth(
  client: LocalIpcClient,
  sliceRef: string,
  provider: string,
  accountProfile: string,
): Promise<{ slice: SliceRecord; provider: string; status: string }> {
  const response = await client.send<Record<string, unknown>>(removeSliceProviderAuthRequest(sliceRef, provider, accountProfile))
  return expectVariant<{ slice: SliceRecord; provider: string; status: string }>(response, "SliceProviderAuthRemoved")
}

export async function startSliceProviderLogin(
  client: LocalIpcClient,
  sliceRef: string,
  provider: string,
  accountProfile: string,
): Promise<{ slice: SliceRecord; login: { provider: string; login_kind: string; auth_url?: string | null; verification_url?: string | null; user_code?: string | null; status: string; message: string } }> {
  const response = await client.send<Record<string, unknown>>(startSliceProviderLoginRequest(sliceRef, provider, accountProfile))
  return expectVariant<{ slice: SliceRecord; login: { provider: string; login_kind: string; auth_url?: string | null; verification_url?: string | null; user_code?: string | null; status: string; message: string } }>(response, "SliceProviderLoginStarted")
}

export async function getSliceDisplayEndpoint(client: LocalIpcClient, sliceRef: string): Promise<SliceDisplayEndpoint> {
  const response = await client.send<Record<string, unknown>>(getSliceDisplayEndpointRequest(sliceRef))
  return expectVariant<{ endpoint: SliceDisplayEndpoint }>(response, "SliceDisplayEndpoint").endpoint
}

export async function getSliceLogs(
  client: LocalIpcClient,
  sliceRef: string,
  tailLines?: number | null,
): Promise<{ slice: SliceRecord; entries: SliceLogEntry[] }> {
  const response = await client.send<Record<string, unknown>>(getSliceLogsRequest(sliceRef, tailLines))
  return expectVariant<{ slice: SliceRecord; entries: SliceLogEntry[] }>(response, "SliceLogs")
}

export async function listSliceAudit(
  client: LocalIpcClient,
  sliceRef: string,
  limit?: number | null,
): Promise<Record<string, unknown>[]> {
  const response = await client.send<Record<string, unknown>>(listSliceAuditRequest(sliceRef, limit))
  return expectVariant<{ events: Record<string, unknown>[] }>(response, "SliceAuditListed").events
}

export async function saveSliceState(
  client: LocalIpcClient,
  sliceRef: string,
  mode?: "restart_agents" | "shutdown" | null,
  scope?: "this_slice" | "future_slices" | null,
): Promise<{ slice: SliceRecord; state: SliceSavedStateRecord }> {
  const response = await client.send<Record<string, unknown>>(saveSliceStateRequest(sliceRef, mode, scope))
  return expectVariant<{ slice: SliceRecord; state: SliceSavedStateRecord }>(response, "SliceStateSaved")
}

export async function getSliceStateStatus(
  client: LocalIpcClient,
  sliceRef: string,
): Promise<{ slice: SliceRecord; state: SliceSavedStateRecord | null }> {
  const response = await client.send<Record<string, unknown>>(getSliceStateStatusRequest(sliceRef))
  return expectVariant<{ slice: SliceRecord; state: SliceSavedStateRecord | null }>(response, "SliceStateStatus")
}

export async function resetSliceState(
  client: LocalIpcClient,
  sliceRef: string,
): Promise<{ slice: SliceRecord; removed_state: SliceSavedStateRecord | null }> {
  const response = await client.send<Record<string, unknown>>(resetSliceStateRequest(sliceRef))
  return expectVariant<{ slice: SliceRecord; removed_state: SliceSavedStateRecord | null }>(response, "SliceStateReset")
}

export async function createSliceBackup(
  client: LocalIpcClient,
  sliceRef: string,
  name?: string | null,
): Promise<{ slice: SliceRecord; backup: SliceBackupRecord; instructions: string }> {
  const response = await client.send<Record<string, unknown>>(createSliceBackupRequest(sliceRef, name))
  return expectVariant<{ slice: SliceRecord; backup: SliceBackupRecord; instructions: string }>(response, "SliceBackupCreated")
}
