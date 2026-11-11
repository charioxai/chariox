import type { ManagedEnvironmentShutdownPolicy } from "./managed-environment-api.js"
import type {
  ManagedEnvironmentCatalog,
  ManagedEnvironmentContextPlanInput,
  ManagedEnvironmentContextSourceOption,
  ManagedEnvironmentSummary,
} from "@chariox/kernel-client/ipc-managed-environment-requests"
import type { WaitingRoomProjectSummary } from "./waiting-room-projects.js"
import { projectSelectionFromId } from "./waiting-room-projects.js"
import type {
  WaitingRoomManagedProviderAccountSelection,
  WaitingRoomRemoteState,
  WaitingRoomState,
} from "./waiting-room-types.js"

export const DEFAULT_MANAGED_REPOSITORY_ROOT = "/home/chariox"

export {
  NEW_MANAGED_MACHINE_REF,
  managedEnvironmentIdFromMachineRef,
  managedEnvironmentMachineRef,
} from "@chariox/kernel-client/waiting-room-runtime-placement"
import {
  NEW_MANAGED_MACHINE_REF,
  managedEnvironmentIdFromMachineRef,
} from "@chariox/kernel-client/waiting-room-runtime-placement"

/** The catalog without a Cloud sign-in: renting stays offered and says what to do first. */
export const SIGNED_OUT_MANAGED_CATALOG: ManagedEnvironmentCatalog = {
  computeClasses: [],
  contextSources: [],
  environments: [],
  rentUnavailable: "Sign in with /cloud login to rent a machine.",
}

export function waitingRoomManagedCatalogRemoteState(
  catalog: ManagedEnvironmentCatalog | undefined,
): WaitingRoomRemoteState {
  return catalog
    ? {
        managedComputeClasses: catalog.computeClasses,
        managedContextSources: catalog.contextSources,
        managedEnvironments: catalog.environments,
        managedRentUnavailable: catalog.rentUnavailable,
      }
    : {}
}

export function waitingRoomUsesManagedMachine(machineRef: string | null | undefined): boolean {
  return machineRef === NEW_MANAGED_MACHINE_REF || managedEnvironmentIdFromMachineRef(machineRef) !== null
}

export function waitingRoomConfiguresNewManagedMachine(machineRef: string | null | undefined): boolean {
  return machineRef === NEW_MANAGED_MACHINE_REF
}

export function selectedManagedEnvironment(
  state: Pick<WaitingRoomState, "selectedMachineRef">,
  remote: WaitingRoomRemoteState,
): ManagedEnvironmentSummary | null {
  const environmentId = managedEnvironmentIdFromMachineRef(state.selectedMachineRef)
  return environmentId
    ? remote.managedEnvironments?.find((environment) => environment.environmentId === environmentId) ?? null
    : null
}

/** After a rent Cloud deleted, the Machine row offers a new machine with the person's draft, so only the region needs changing. */
export function waitingRoomStateAfterFailedRent(
  state: WaitingRoomState,
  remote: WaitingRoomRemoteState,
): WaitingRoomState {
  return selectedManagedEnvironment(state, remote)?.desiredState === "deleted"
    ? { ...state, selectedMachineRef: NEW_MANAGED_MACHINE_REF, selectedKernelRef: "" }
    : state
}

export function managedEnvironmentIsLaunchReady(environment: ManagedEnvironmentSummary): boolean {
  return environment.desiredState === "running"
    && environment.observedState === "ready"
    && environment.observedRevision === environment.desiredRevision
    && Boolean(environment.runtimeMachineId)
    && Boolean(environment.runtimeKernelId)
    && Boolean(environment.contextManifestDigest)
}

export function normalizeWaitingRoomManagedDraft(
  state: WaitingRoomState,
  remote: WaitingRoomRemoteState,
): WaitingRoomState {
  const computeClasses = remote.managedComputeClasses ?? []
  const computeClass = state.managedComputeClass ?? computeClasses[0]?.computeClass
  const regions = computeClasses.find((option) => option.computeClass === computeClass)?.regions ?? []
  const region = state.managedRegion ?? regions[0]
  const sources = remote.managedContextSources ?? []
  const selectedSource = state.managedContextSourceTargetId
    ?? preferredManagedContextSource(sources, remote)?.sourceTargetId
  const providerAccounts = remote.providerAccounts ?? []
  const repositoryOptions = waitingRoomProjectRepositoryOptions(state, remote)
  const primaryWorkspaceId = repositoryOptions[0]?.workspaceId
  const project = selectedManagedProject(state, remote)
  const existingSelection = state.managedRepositorySelection
  const supportingWorkspaceIds = repositoryOptions.slice(1).map((option) => option.workspaceId)
  const selectedSupportingWorkspaceIds = existingSelection
    && project
    && existingSelection.projectId === project.id
    && existingSelection.primaryWorkspaceId === primaryWorkspaceId
      ? supportingWorkspaceIds.filter((workspaceId) => (
          existingSelection.supportingWorkspaceIds.includes(workspaceId)
        ))
      : supportingWorkspaceIds
  const managedRepositorySelection = project && primaryWorkspaceId
    ? {
        projectId: project.id,
        primaryWorkspaceId,
        supportingWorkspaceIds: selectedSupportingWorkspaceIds,
      }
    : undefined
  const normalized: WaitingRoomState = {
    ...state,
    managedRepositoryRoot: state.managedRepositoryRoot ?? DEFAULT_MANAGED_REPOSITORY_ROOT,
    managedKernelContext: state.managedKernelContext ?? "empty",
    managedDevelopmentMode: state.managedDevelopmentMode ?? "empty",
    managedRepositoryIndex: supportingWorkspaceIds.length === 0
      ? 0
      : modulo(state.managedRepositoryIndex ?? 0, supportingWorkspaceIds.length),
    managedProviderAccountSource: state.managedProviderAccountSource ?? "none",
    managedProviderAccountIndex: providerAccounts.length === 0
      ? 0
      : modulo(state.managedProviderAccountIndex ?? 0, providerAccounts.length),
    managedGitCredentialSource: state.managedGitCredentialSource ?? "none",
    managedAutoStopPreset: state.managedAutoStopPreset ?? "agents_done",
    ...(computeClass ? { managedComputeClass: computeClass } : {}),
    ...(region ? { managedRegion: region } : {}),
    ...(selectedSource ? { managedContextSourceTargetId: selectedSource } : {}),
  }
  if (managedRepositorySelection) normalized.managedRepositorySelection = managedRepositorySelection
  else delete normalized.managedRepositorySelection
  return normalized
}

export function preferredManagedContextSource(
  sources: readonly ManagedEnvironmentContextSourceOption[],
  remote: WaitingRoomRemoteState,
): ManagedEnvironmentContextSourceOption | null {
  const currentKernelId = remote.relay?.daemon_id?.trim()
  const currentMachineId = remote.relay?.machine_id?.trim()
  return sources.find((source) => (
    source.kernelId === currentKernelId && source.machineId === currentMachineId
  )) ?? null
}

export function cycleManagedComputeClass(
  state: WaitingRoomState,
  remote: WaitingRoomRemoteState,
  delta: number,
): WaitingRoomState {
  const options = remote.managedComputeClasses ?? []
  if (options.length === 0) return state
  const index = Math.max(0, options.findIndex((option) => option.computeClass === state.managedComputeClass))
  const next = options[modulo(index + delta, options.length)] ?? options[0]!
  return {
    ...state,
    managedComputeClass: next.computeClass,
    managedRegion: next.regions[0] ?? "",
  }
}

export function cycleManagedRegion(
  state: WaitingRoomState,
  remote: WaitingRoomRemoteState,
  delta: number,
): WaitingRoomState {
  const regions = remote.managedComputeClasses
    ?.find((option) => option.computeClass === state.managedComputeClass)?.regions ?? []
  if (regions.length === 0) return state
  const index = Math.max(0, regions.indexOf(state.managedRegion ?? ""))
  return { ...state, managedRegion: regions[modulo(index + delta, regions.length)]! }
}

export function cycleManagedKernelContext(
  state: WaitingRoomState,
  remote: WaitingRoomRemoteState,
  delta: number,
): WaitingRoomState {
  const options = [
    { mode: "empty" as const, sourceTargetId: state.managedContextSourceTargetId },
    ...(remote.managedContextSources ?? []).map((source) => ({
      mode: "source_kernel" as const,
      sourceTargetId: source.sourceTargetId,
    })),
  ]
  const index = Math.max(0, options.findIndex((option) => (
    option.mode === state.managedKernelContext
    && (option.mode === "empty" || option.sourceTargetId === state.managedContextSourceTargetId)
  )))
  const next = options[modulo(index + delta, options.length)] ?? options[0]!
  return {
    ...state,
    managedKernelContext: next.mode,
    ...(next.sourceTargetId ? { managedContextSourceTargetId: next.sourceTargetId } : {}),
  }
}

export function cycleManagedDevelopment(state: WaitingRoomState): WaitingRoomState {
  return {
    ...state,
    managedDevelopmentMode: state.managedDevelopmentMode === "empty" ? "current_project" : "empty",
  }
}

export function toggleManagedRepositorySelection(
  state: WaitingRoomState,
  remote: WaitingRoomRemoteState,
): WaitingRoomState {
  const normalized = normalizeWaitingRoomManagedDraft(state, remote)
  const selection = normalized.managedRepositorySelection
  const option = waitingRoomProjectRepositoryOptions(normalized, remote)
    .slice(1)[normalized.managedRepositoryIndex ?? 0]
  if (!selection || !option) return normalized
  const selected = new Set(selection.supportingWorkspaceIds)
  if (selected.has(option.workspaceId)) selected.delete(option.workspaceId)
  else selected.add(option.workspaceId)
  const supportingWorkspaceIds = waitingRoomProjectRepositoryOptions(normalized, remote)
    .slice(1)
    .map((candidate) => candidate.workspaceId)
    .filter((workspaceId) => selected.has(workspaceId))
  return {
    ...normalized,
    managedRepositorySelection: { ...selection, supportingWorkspaceIds },
  }
}

export function cycleManagedProviderAccounts(
  state: WaitingRoomState,
  _remote: WaitingRoomRemoteState,
): WaitingRoomState {
  if (state.managedProviderAccountSource === "selected_account") {
    return {
      ...state,
      managedProviderAccountSource: "none",
      managedProviderAccountSelection: [],
    }
  }
  const next: WaitingRoomState = {
    ...state,
    managedProviderAccountSource: "selected_account",
  }
  delete next.managedProviderAccountSelection
  return next
}

export function toggleManagedProviderAccountSelection(
  state: WaitingRoomState,
  remote: WaitingRoomRemoteState,
): WaitingRoomState {
  const normalized = normalizeWaitingRoomManagedDraft(state, remote)
  const option = remote.providerAccounts?.[normalized.managedProviderAccountIndex ?? 0]
  if (!option) return normalized
  const selected = new Map(managedProviderAccountSelections(normalized, remote).map((account) => (
    [managedProviderAccountSelectionKey(account), account]
  )))
  const key = managedProviderAccountSelectionKey({
    provider: option.provider,
    accountProfile: option.profile_id,
  })
  if (selected.has(key)) selected.delete(key)
  else if (managedProviderAccountIsTransferable(option)) {
    selected.set(key, {
      provider: option.provider,
      accountProfile: option.profile_id,
    })
  }
  const managedProviderAccountSelection = [
    ...(remote.providerAccounts ?? []).flatMap((profile) => {
      const selection = selected.get(managedProviderAccountSelectionKey({
        provider: profile.provider,
        accountProfile: profile.profile_id,
      }))
      return selection ? [selection] : []
    }),
    ...[...selected.values()].filter((selection) => !(remote.providerAccounts ?? []).some((profile) => (
      profile.provider === selection.provider && profile.profile_id === selection.accountProfile
    ))),
  ]
  return {
    ...normalized,
    managedProviderAccountSource: managedProviderAccountSelection.length > 0
      ? "selected_account"
      : "none",
    managedProviderAccountSelection,
  }
}

export function managedProviderAccountSelections(
  state: Pick<WaitingRoomState,
    | "managedProviderAccountSelection"
    | "managedProviderAccountSource"
    | "providerId"
    | "accountProfileId"
  >,
  remote: WaitingRoomRemoteState,
): WaitingRoomManagedProviderAccountSelection[] {
  if (state.managedProviderAccountSource !== "selected_account") return []
  if (state.managedProviderAccountSelection !== undefined) {
    return [...new Map(state.managedProviderAccountSelection.map((account) => (
      [managedProviderAccountSelectionKey(account), account]
    ))).values()]
  }
  return (remote.providerAccounts ?? [])
    .filter(managedProviderAccountIsTransferable)
    .map((account) => ({
      provider: account.provider,
      accountProfile: account.profile_id,
    }))
}

export function cycleManagedGitCredentials(state: WaitingRoomState): WaitingRoomState {
  return {
    ...state,
    managedGitCredentialSource: state.managedGitCredentialSource === "none" ? "selected" : "none",
  }
}

export function cycleManagedAutoStopPreset(state: WaitingRoomState, delta: number): WaitingRoomState {
  const options: readonly NonNullable<WaitingRoomState["managedAutoStopPreset"]>[] = [
    "agents_done",
    "time_3h",
    "manual",
  ]
  const index = Math.max(0, options.indexOf(state.managedAutoStopPreset ?? "agents_done"))
  return { ...state, managedAutoStopPreset: options[modulo(index + delta, options.length)] ?? "agents_done" }
}

export function managedKernelContextLabel(state: WaitingRoomState, remote: WaitingRoomRemoteState): string {
  if (state.managedKernelContext === "empty") return "Empty"
  return remote.managedContextSources
    ?.find((source) => source.sourceTargetId === state.managedContextSourceTargetId)?.label
    ?? "Source unavailable"
}

export function managedAutoStopLabel(state: WaitingRoomState): string {
  switch (state.managedAutoStopPreset ?? "agents_done") {
    case "agents_done": return "30 s after last work finishes"
    case "time_3h": return "stop after 3 h, including active work"
    case "manual": return "manual"
  }
}

export function managedEnvironmentShutdownPolicy(state: WaitingRoomState): ManagedEnvironmentShutdownPolicy {
  switch (state.managedAutoStopPreset ?? "agents_done") {
    case "agents_done": return { kind: "last_work_finished", warningSeconds: 30 }
    case "time_3h": return { kind: "time_limit", durationSeconds: 10800 }
    case "manual": return { kind: "manual" }
  }
}

export function managedEnvironmentDraftBlockReason(
  state: WaitingRoomState,
  remote: WaitingRoomRemoteState,
): string | null {
  if (remote.managedRentUnavailable) return remote.managedRentUnavailable
  const computeClass = remote.managedComputeClasses
    ?.find((candidate) => candidate.computeClass === state.managedComputeClass)
  if (!computeClass) return "The selected managed compute class is unavailable."
  if (!state.managedRegion || !computeClass.regions.includes(state.managedRegion)) {
    return "The selected managed region is unavailable for this compute class."
  }
  const selectedProviderAccounts = managedProviderAccountSelections(state, remote)
  const sourceRequired = state.managedKernelContext === "source_kernel"
    || state.managedDevelopmentMode === "current_project"
    || selectedProviderAccounts.length > 0
    || state.managedGitCredentialSource === "selected"
  const source = remote.managedContextSources
    ?.find((candidate) => candidate.sourceTargetId === state.managedContextSourceTargetId)
  if (sourceRequired) {
    if (!source) return "The selected managed-context source is unavailable."
    if (
      !remote.relay?.connected
      || source.kernelId !== remote.relay.daemon_id?.trim()
      || source.machineId !== remote.relay.machine_id?.trim()
    ) {
      return "The selected managed-context source must be the connected kernel on this Machine."
    }
  }
  if (state.managedDevelopmentMode === "current_project") {
    if (!selectedManagedProject(state, remote)) return "Choose an existing Project before transferring it."
    if (!remote.workspaceId) return "The connected source kernel returned no primary Workspace."
  }
  if ((state.managedProviderAccountSource ?? "none") === "selected_account") {
    const accounts = selectedProviderAccounts
    if (accounts.length === 0 && state.managedProviderAccountSelection !== undefined) {
      return "Choose at least one available provider account before transferring it."
    }
    if (accounts.length > 16) return "Choose no more than 16 provider accounts."
    if (accounts.some((account) => !(remote.providerAccounts ?? []).some((profile) => (
      profile.provider === account.provider
      && profile.profile_id === account.accountProfile
      && managedProviderAccountIsTransferable(profile)
    )))) {
      return "One or more selected provider accounts are unavailable or not authenticated."
    }
  }
  if ((state.managedGitCredentialSource ?? "none") === "selected"
    && !remote.gitCredentials?.some((credential) => credential.credentialId === "github")) {
    return "The connected source kernel has no transferable GitHub credential."
  }
  return null
}

export function managedEnvironmentContextPlanInput(
  state: WaitingRoomState,
  remote: WaitingRoomRemoteState,
): ManagedEnvironmentContextPlanInput {
  const blockReason = managedEnvironmentDraftBlockReason(state, remote)
  if (blockReason) throw new Error(blockReason)
  const providerAccounts = managedProviderAccountSelections(state, remote)
  const sourceRequired = state.managedKernelContext === "source_kernel"
    || state.managedDevelopmentMode === "current_project"
    || providerAccounts.length > 0
    || state.managedGitCredentialSource === "selected"
  const project = selectedManagedProject(state, remote)
  const developmentSetup = waitingRoomProjectDevelopmentSetup(state, remote)
  return {
    sourceTargetId: sourceRequired ? state.managedContextSourceTargetId ?? null : null,
    kernelContext: state.managedKernelContext ?? "empty",
    developmentSetup: state.managedDevelopmentMode === "current_project" && project
      ? developmentSetup ?? { kind: "empty" }
      : { kind: "empty" },
    providerAccounts: providerAccounts.length > 0
      ? {
          kind: "selected",
          accounts: providerAccounts,
        }
      : { kind: "none" },
    gitCredentials: state.managedGitCredentialSource === "selected"
      ? { kind: "selected", credentialIds: ["github"] }
      : { kind: "none" },
  }
}

export function waitingRoomProjectDevelopmentSetup(
  state: Pick<WaitingRoomState,
    | "projectSelectionId"
    | "managedDevelopmentMode"
    | "managedRepositorySelection"
  >,
  remote: WaitingRoomRemoteState,
): ManagedEnvironmentContextPlanInput["developmentSetup"] | null {
  if (state.managedDevelopmentMode === "empty") return { kind: "empty" }
  if (state.managedDevelopmentMode !== "current_project") return null
  if (!remote.workspaceId) return null
  const project = selectedManagedProject(state, remote)
  if (!project) return null
  const options = waitingRoomProjectRepositoryOptions(state, remote)
  const primaryWorkspaceId = options[0]?.workspaceId ?? ""
  const selection = state.managedRepositorySelection
  const selectedSupporting = selection
    && selection.projectId === project.id
    && selection.primaryWorkspaceId === primaryWorkspaceId
      ? new Set(selection.supportingWorkspaceIds)
      : new Set(options.slice(1).map((option) => option.workspaceId))
  const repositories = options
    .filter((option) => option.primary || selectedSupporting.has(option.workspaceId))
    .map((option) => ({
      role: option.primary ? "primary" as const : "supporting" as const,
      workspaceId: option.workspaceId,
      worktreeId: option.primary ? remote.worktreeId ?? null : null,
    }))
  return { kind: "source_project", projectId: project.id, repositories }
}

export function waitingRoomProjectRepositoryOptions(
  state: Pick<WaitingRoomState, "projectSelectionId">,
  remote: WaitingRoomRemoteState,
): Array<{ workspaceId: string; primary: boolean }> {
  const project = selectedManagedProject(state, remote)
  if (!project) return []
  const primaryWorkspaceId = remote.workspaceId ?? ""
  const workspaceIds = project.workspace_ids?.length ? project.workspace_ids : [project.workspace_id]
  return [primaryWorkspaceId, ...workspaceIds.filter((workspaceId) => workspaceId !== primaryWorkspaceId)]
    .filter((workspaceId, index, values) => workspaceId && values.indexOf(workspaceId) === index)
    .map((workspaceId, index) => ({ workspaceId, primary: index === 0 }))
}

function selectedManagedProject(
  state: Pick<WaitingRoomState, "projectSelectionId">,
  remote: WaitingRoomRemoteState,
): WaitingRoomProjectSummary | null {
  const selection = projectSelectionFromId(state.projectSelectionId ?? "default")
  if (selection.kind === "existing") {
    return remote.projects?.find((project) => project.id === selection.project_id) ?? null
  }
  if (selection.kind === "default") {
    return remote.projects?.find((project) => (
      project.kind === "default"
      && (project.workspace_ids?.includes(remote.workspaceId ?? "")
        || project.workspace_id === remote.workspaceId)
    )) ?? null
  }
  return null
}

function modulo(value: number, size: number): number {
  if (size <= 0) return 0
  return ((value % size) + size) % size
}

function managedProviderAccountSelectionKey(
  account: WaitingRoomManagedProviderAccountSelection,
): string {
  return `${account.provider}\u0000${account.accountProfile}`
}

export function managedProviderAccountIsTransferable(
  profile: NonNullable<WaitingRoomRemoteState["providerAccounts"]>[number],
): boolean {
  return profile.auth_state === "authenticated" && profile.context_transfer !== "unavailable"
}

/** The short label a Machine setup row shows for a login whose Vault cannot be sealed for the new Machine. */
export function managedProviderAccountUnavailableLabel(
  profile: NonNullable<WaitingRoomRemoteState["providerAccounts"]>[number],
): string {
  return profile.context_transfer === "unavailable" ? "Vault token" : profile.auth_state
}
