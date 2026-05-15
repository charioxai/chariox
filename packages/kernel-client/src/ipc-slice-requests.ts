export function listSlicesRequest() {
  return { ListSlices: null }
}

export function createSliceRequest(options: {
  name: string
  backend?: "local_docker" | "ssh_docker"
  os?: string
  workspaceMount?: string | null
  workerKernelRef?: string | null
  displayUrl?: string | null
}) {
  return {
    CreateSlice: {
      name: options.name,
      backend: options.backend ?? "local_docker",
      os: options.os ?? "linux",
      workspace_mount: options.workspaceMount ?? null,
      worker_kernel_ref: options.workerKernelRef ?? null,
      display_url: options.displayUrl ?? null,
    },
  }
}

export function getSliceRequest(sliceRef: string) {
  return { GetSlice: { slice_ref: sliceRef } }
}

export function startSliceRequest(sliceRef: string) {
  return { StartSlice: { slice_ref: sliceRef } }
}

export function stopSliceRequest(sliceRef: string) {
  return { StopSlice: { slice_ref: sliceRef } }
}

export function deleteSliceRequest(sliceRef: string) {
  return { DeleteSlice: { slice_ref: sliceRef } }
}

export function importSliceProviderAuthRequest(sliceRef: string, provider: string) {
  return {
    ImportSliceProviderAuth: {
      slice_ref: sliceRef,
      provider,
    },
  }
}

export function getSliceDisplayEndpointRequest(sliceRef: string) {
  return { GetSliceDisplayEndpoint: { slice_ref: sliceRef } }
}
