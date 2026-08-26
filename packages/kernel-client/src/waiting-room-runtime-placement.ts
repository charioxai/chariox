export type WaitingRoomLaunchMachineInput = {
  machine_id: string
  machine_alias?: string | null
  registry_alias?: string | null
  display_name?: string | null
}

export type WaitingRoomLaunchKernelInput = {
  kernel_id: string
  machine_id: string
  kernel_alias?: string | null
  relay_alias?: string | null
}

export type WaitingRoomLaunchRemoteState = {
  machines?: readonly WaitingRoomLaunchMachineInput[]
  kernels?: readonly WaitingRoomLaunchKernelInput[]
  managedEnvironments?: readonly WaitingRoomLaunchManagedEnvironmentInput[]
}

export type WaitingRoomLaunchManagedEnvironmentInput = {
  environmentId: string
  name: string
  desiredState: "running" | "stopped" | "deleted"
  observedState: string
  desiredRevision: number
  observedRevision: number
  runtimeMachineId: string | null
  runtimeKernelId: string | null
}

export const NEW_MANAGED_MACHINE_REF = "managed:new"
const managedEnvironmentRefPrefix = "managed:environment:"

export type WaitingRoomLaunchPlacementState = {
  selectedMachineRef?: string | null
  selectedKernelRef?: string | null
}

export type WaitingRoomLaunchMachineOption<TMachine extends WaitingRoomLaunchMachineInput = WaitingRoomLaunchMachineInput> = {
  id: string
  label: string
  machine: TMachine | null
  managedEnvironment: WaitingRoomLaunchManagedEnvironmentInput | null
}

export type WaitingRoomLaunchKernelOption<TKernel extends WaitingRoomLaunchKernelInput = WaitingRoomLaunchKernelInput> = {
  id: string
  label: string
  machineId: string
  kernel: TKernel | null
}

export function waitingRoomLaunchMachineOptions<TMachine extends WaitingRoomLaunchMachineInput>(
  remote: {
    machines?: readonly TMachine[]
    managedEnvironments?: readonly WaitingRoomLaunchManagedEnvironmentInput[]
  } = {},
): WaitingRoomLaunchMachineOption<TMachine>[] {
  const managedMachineIds = new Set(
    (remote.managedEnvironments ?? []).flatMap((environment) => (
      environment.runtimeMachineId ? [environment.runtimeMachineId] : []
    )),
  )
  return [
    { id: "local", label: "local", machine: null, managedEnvironment: null },
    ...(remote.machines ?? []).filter((machine) => !managedMachineIds.has(machine.machine_id)).map((machine) => ({
      id: machine.machine_id,
      label: machine.registry_alias ?? machine.machine_alias ?? machine.display_name ?? machine.machine_id,
      machine,
      managedEnvironment: null,
    })),
    ...(remote.managedEnvironments ?? [])
      .filter((environment) => environment.desiredState !== "deleted")
      .map((environment) => ({
        id: managedEnvironmentMachineRef(environment.environmentId),
        label: `${environment.name} · ${managedEnvironmentStatusLabel(environment)}`,
        machine: null,
        managedEnvironment: environment,
      })),
    ...(remote.managedEnvironments !== undefined
      ? [{
          id: NEW_MANAGED_MACHINE_REF,
          label: "+ New Chariox-managed machine...",
          machine: null,
          managedEnvironment: null,
        }]
      : []),
  ]
}

export function waitingRoomSelectedLaunchMachineRef(
  state: Pick<WaitingRoomLaunchPlacementState, "selectedMachineRef">,
  remote: WaitingRoomLaunchRemoteState = {},
): string {
  const options = waitingRoomLaunchMachineOptions(remote)
  const selected = state.selectedMachineRef?.trim() || "local"
  if (selected === NEW_MANAGED_MACHINE_REF || managedEnvironmentIdFromMachineRef(selected)) {
    return selected
  }
  return options.some((option) => option.id === selected) ? selected : options[0]?.id ?? "local"
}

export function waitingRoomLaunchKernelOptions<TKernel extends WaitingRoomLaunchKernelInput>(
  remote: {
    kernels?: readonly TKernel[]
    managedEnvironments?: readonly WaitingRoomLaunchManagedEnvironmentInput[]
  } = {},
  machineRef = "local",
): WaitingRoomLaunchKernelOption<TKernel>[] {
  if (machineRef === "local") {
    return [{ id: "local", label: "local", machineId: "local", kernel: null }]
  }
  const managedEnvironmentId = managedEnvironmentIdFromMachineRef(machineRef)
  if (managedEnvironmentId) {
    const environment = remote.managedEnvironments
      ?.find((candidate) => candidate.environmentId === managedEnvironmentId)
    if (!environment || !managedEnvironmentIsReady(environment)) return []
    const kernel = (remote.kernels ?? []).find((candidate) => (
      candidate.kernel_id === environment.runtimeKernelId
      && candidate.machine_id === environment.runtimeMachineId
    ))
    return kernel
      ? [{
          id: kernel.kernel_id,
          label: kernel.relay_alias ?? kernel.kernel_alias ?? kernel.kernel_id,
          machineId: kernel.machine_id,
          kernel,
        }]
      : []
  }
  if (machineRef === NEW_MANAGED_MACHINE_REF) return []
  return (remote.kernels ?? [])
    .filter((kernel) => kernel.machine_id === machineRef)
    .map((kernel) => ({
      id: kernel.kernel_id,
      label: kernel.relay_alias ?? kernel.kernel_alias ?? kernel.kernel_id,
      machineId: kernel.machine_id,
      kernel,
    }))
}

export function waitingRoomSelectedLaunchKernelRef(
  state: Pick<WaitingRoomLaunchPlacementState, "selectedMachineRef" | "selectedKernelRef">,
  remote: WaitingRoomLaunchRemoteState = {},
): string {
  const machineRef = waitingRoomSelectedLaunchMachineRef(state, remote)
  const options = waitingRoomLaunchKernelOptions(remote, machineRef)
  const selected = state.selectedKernelRef?.trim() || (machineRef === "local" ? "local" : "")
  return options.some((option) => option.id === selected) ? selected : options[0]?.id ?? ""
}

export function waitingRoomLaunchPlacement(
  state: Pick<WaitingRoomLaunchPlacementState, "selectedMachineRef" | "selectedKernelRef">,
  remote: WaitingRoomLaunchRemoteState = {},
): {
  machineRef: string
  kernelRef: string
  workerKernelRef: string | null
  managedEnvironmentId: string | null
  newManagedEnvironment: boolean
} {
  const machineRef = waitingRoomSelectedLaunchMachineRef(state, remote)
  const kernelRef = waitingRoomSelectedLaunchKernelRef(state, remote)
  return {
    machineRef,
    kernelRef,
    workerKernelRef: null,
    managedEnvironmentId: managedEnvironmentIdFromMachineRef(machineRef),
    newManagedEnvironment: machineRef === NEW_MANAGED_MACHINE_REF,
  }
}

export function formatWaitingRoomLaunchMachineValue(
  state: Pick<WaitingRoomLaunchPlacementState, "selectedMachineRef">,
  remote: WaitingRoomLaunchRemoteState = {},
): string {
  const selected = waitingRoomSelectedLaunchMachineRef(state, remote)
  return waitingRoomLaunchMachineOptions(remote).find((option) => option.id === selected)?.label ?? selected
}

export function formatWaitingRoomLaunchKernelValue(
  state: Pick<WaitingRoomLaunchPlacementState, "selectedMachineRef" | "selectedKernelRef">,
  remote: WaitingRoomLaunchRemoteState = {},
): string {
  const machineRef = waitingRoomSelectedLaunchMachineRef(state, remote)
  const selected = waitingRoomSelectedLaunchKernelRef(state, remote)
  if (!selected) {
    if (machineRef === NEW_MANAGED_MACHINE_REF) return "created during launch"
    const environmentId = managedEnvironmentIdFromMachineRef(machineRef)
    const environment = remote.managedEnvironments
      ?.find((candidate) => candidate.environmentId === environmentId)
    return environment ? managedEnvironmentStatusLabel(environment) : "none available"
  }
  return waitingRoomLaunchKernelOptions(remote, machineRef).find((option) => option.id === selected)?.label ?? selected
}

export function normalizeWaitingRoomLaunchPlacement(
  state: WaitingRoomLaunchPlacementState,
  remote: WaitingRoomLaunchRemoteState = {},
): { selectedMachineRef: string; selectedKernelRef: string } {
  const selectedMachineRef = waitingRoomSelectedLaunchMachineRef(state, remote)
  const selectedKernelRef = waitingRoomSelectedLaunchKernelRef({ ...state, selectedMachineRef }, remote)
  return { selectedMachineRef, selectedKernelRef }
}

export function cycleWaitingRoomLaunchMachine<TState extends WaitingRoomLaunchPlacementState>(
  state: TState,
  remote: WaitingRoomLaunchRemoteState = {},
  delta: number,
): TState {
  const options = waitingRoomLaunchMachineOptions(remote)
  const current = waitingRoomSelectedLaunchMachineRef(state, remote)
  const index = Math.max(0, options.findIndex((option) => option.id === current))
  const nextMachineRef = options[modulo(index + delta, options.length)]?.id ?? "local"
  const nextKernelRef = waitingRoomLaunchKernelOptions(remote, nextMachineRef)[0]?.id ?? ""
  return {
    ...state,
    selectedMachineRef: nextMachineRef,
    selectedKernelRef: nextKernelRef,
  }
}

export function cycleWaitingRoomLaunchKernel<TState extends WaitingRoomLaunchPlacementState>(
  state: TState,
  remote: WaitingRoomLaunchRemoteState = {},
  delta: number,
): TState {
  const machineRef = waitingRoomSelectedLaunchMachineRef(state, remote)
  const options = waitingRoomLaunchKernelOptions(remote, machineRef)
  if (options.length === 0) {
    return { ...state, selectedKernelRef: "" }
  }
  const current = waitingRoomSelectedLaunchKernelRef(state, remote)
  const index = Math.max(0, options.findIndex((option) => option.id === current))
  const next = options[modulo(index + delta, options.length)] ?? options[0]
  return {
    ...state,
    selectedMachineRef: managedEnvironmentIdFromMachineRef(machineRef) ? machineRef : next?.machineId ?? machineRef,
    selectedKernelRef: next?.id ?? "",
  }
}

function modulo(value: number, size: number) {
  if (size <= 0) {
    return 0
  }
  return ((value % size) + size) % size
}

export function managedEnvironmentMachineRef(environmentId: string): string {
  return `${managedEnvironmentRefPrefix}${environmentId}`
}

export function managedEnvironmentIdFromMachineRef(machineRef: string | null | undefined): string | null {
  const value = machineRef?.trim() ?? ""
  return value.startsWith(managedEnvironmentRefPrefix)
    ? value.slice(managedEnvironmentRefPrefix.length) || null
    : null
}

function managedEnvironmentIsReady(environment: WaitingRoomLaunchManagedEnvironmentInput): boolean {
  return environment.desiredState === "running"
    && environment.observedState === "ready"
    && environment.desiredRevision === environment.observedRevision
    && Boolean(environment.runtimeMachineId)
    && Boolean(environment.runtimeKernelId)
}

function managedEnvironmentStatusLabel(environment: WaitingRoomLaunchManagedEnvironmentInput): string {
  if (managedEnvironmentIsReady(environment)) return "ready"
  return environment.observedState.replaceAll("_", " ")
}
