export function listSlicesRequest() {
  return { ListSlices: null }
}

export function createSliceRequest(options: {
  name: string
  backend?: "local_docker" | "ssh_docker"
  os?: string
  displayMode?: "headless" | "headed"
  workspaceId?: string | null
  worktreeId?: string | null
  workspaceMount?: string | null
  workerKernelRef?: string | null
  displayUrl?: string | null
  providerAuth?: unknown[]
}) {
  return {
    CreateSlice: {
      name: options.name,
      backend: options.backend ?? "local_docker",
      os: options.os ?? "linux",
      display_mode: options.displayMode ?? "headless",
      workspace_id: options.workspaceId ?? null,
      worktree_id: options.worktreeId ?? null,
      workspace_mount: options.workspaceMount ?? null,
      worker_kernel_ref: options.workerKernelRef ?? null,
      display_url: options.displayUrl ?? null,
      provider_auth: options.providerAuth ?? [],
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

export function removeSliceProviderAuthRequest(sliceRef: string, provider: string) {
  return {
    RemoveSliceProviderAuth: {
      slice_ref: sliceRef,
      provider,
    },
  }
}

export function startSliceProviderLoginRequest(sliceRef: string, provider: string) {
  return {
    StartSliceProviderLogin: {
      slice_ref: sliceRef,
      provider,
    },
  }
}

export function setSliceProviderAuthAliasRequest(sliceRef: string, provider: string, alias: string | null) {
  return {
    SetSliceProviderAuthAlias: {
      slice_ref: sliceRef,
      provider,
      alias,
    },
  }
}

export function getSliceDisplayEndpointRequest(sliceRef: string) {
  return { GetSliceDisplayEndpoint: { slice_ref: sliceRef } }
}

export function getSliceLogsRequest(sliceRef: string, tailLines?: number | null) {
  return {
    GetSliceLogs: {
      slice_ref: sliceRef,
      tail_lines: tailLines ?? null,
    },
  }
}

export function listSliceAuditRequest(sliceRef: string, limit?: number | null) {
  return {
    ListSliceAudit: {
      slice_ref: sliceRef,
      ...(limit == null ? {} : { limit }),
    },
  }
}
