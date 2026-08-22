import type { ManagedEnvironmentDevelopmentSetup } from "./ipc-managed-environment-requests.js"

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
  developmentSetup?: ManagedEnvironmentDevelopmentSetup | null
  workerKernelRef?: string | null
  displayUrl?: string | null
  providerAuth?: unknown[]
  fromSavedState?: string | null
  base?: "default" | "clean" | null
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
      ...(options.developmentSetup
        ? { development: serializeSliceDevelopment(options.developmentSetup) }
        : {}),
      worker_kernel_ref: options.workerKernelRef ?? null,
      display_url: options.displayUrl ?? null,
      provider_auth: options.providerAuth ?? [],
      from_saved_state: options.fromSavedState ?? null,
      base: options.base ?? null,
    },
  }
}

function serializeSliceDevelopment(
  development: ManagedEnvironmentDevelopmentSetup,
) {
  if (development.kind === "empty") return { kind: "empty" } as const
  return {
    kind: "source_project" as const,
    project_id: development.projectId,
    repositories: development.repositories.map((repository) => ({
      role: repository.role,
      workspaceId: repository.workspaceId,
      worktreeId: repository.worktreeId,
    })),
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

export function importSliceProviderAuthRequest(sliceRef: string, provider: string, accountProfile: string) {
  return {
    ImportSliceProviderAuth: {
      slice_ref: sliceRef,
      provider,
      account_profile: accountProfile,
    },
  }
}

export function removeSliceProviderAuthRequest(sliceRef: string, provider: string, accountProfile: string) {
  return {
    RemoveSliceProviderAuth: {
      slice_ref: sliceRef,
      provider,
      account_profile: accountProfile,
    },
  }
}

export function startSliceProviderLoginRequest(sliceRef: string, provider: string, accountProfile: string) {
  return {
    StartSliceProviderLogin: {
      slice_ref: sliceRef,
      provider,
      account_profile: accountProfile,
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

export type SliceStateSaveMode = "restart_agents" | "shutdown"
export type SliceStateSaveScope = "this_slice" | "future_slices"

export function saveSliceStateRequest(
  sliceRef: string,
  mode?: SliceStateSaveMode | null,
  scope?: SliceStateSaveScope | null,
) {
  return { SaveSliceState: { slice_ref: sliceRef, ...(mode == null ? {} : { mode }), ...(scope == null ? {} : { scope }) } }
}

export function getSliceStateStatusRequest(sliceRef: string) {
  return { GetSliceStateStatus: { slice_ref: sliceRef } }
}

export function resetSliceStateRequest(sliceRef: string) {
  return { ResetSliceState: { slice_ref: sliceRef } }
}

export function createSliceBackupRequest(sliceRef: string, name?: string | null) {
  return {
    CreateSliceBackup: {
      slice_ref: sliceRef,
      name: name ?? null,
    },
  }
}
