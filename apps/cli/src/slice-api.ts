import type { LocalIpcClient } from "./ipc.js"
import type { SliceDisplayEndpoint, SliceRecord } from "./cli-types.js"
import {
  createSliceRequest,
  deleteSliceRequest,
  getSliceDisplayEndpointRequest,
  getSliceRequest,
  importSliceProviderAuthRequest,
  listSlicesRequest,
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
): Promise<{ slice: SliceRecord; provider: string; status: string }> {
  const response = await client.send<Record<string, unknown>>(importSliceProviderAuthRequest(sliceRef, provider))
  return expectVariant<{ slice: SliceRecord; provider: string; status: string }>(response, "SliceProviderAuthImported")
}

export async function getSliceDisplayEndpoint(client: LocalIpcClient, sliceRef: string): Promise<SliceDisplayEndpoint> {
  const response = await client.send<Record<string, unknown>>(getSliceDisplayEndpointRequest(sliceRef))
  return expectVariant<{ endpoint: SliceDisplayEndpoint }>(response, "SliceDisplayEndpoint").endpoint
}
