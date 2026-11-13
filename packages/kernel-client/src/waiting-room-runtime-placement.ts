export type WaitingRoomLaunchMachineInput = {
  machine_id: string
  machine_alias?: string | null
  display_name?: string | null
}

export type WaitingRoomLaunchKernelInput = {
  kernel_id: string
  machine_id: string
  kernel_alias?: string | null
  relay_alias?: string | null
}

/** The kernel this client is connected to, as its relay status reports it. */
export type WaitingRoomLaunchConnectedKernelInput = {
  daemon_id?: string | null
  daemon_alias?: string | null
  machine_id?: string | null
}

export type WaitingRoomLaunchRemoteState = {
  machineAllocationOffered?: boolean
  machines?: readonly WaitingRoomLaunchMachineInput[]
  kernels?: readonly WaitingRoomLaunchKernelInput[]
  relay?: WaitingRoomLaunchConnectedKernelInput | null
}

export const NEW_MACHINE_ALLOCATION_REF = "allocation:new"

export type WaitingRoomLaunchPlacementState = {
  selectedMachineRef?: string | null
  selectedKernelRef?: string | null
}

export type WaitingRoomLaunchMachineOption<TMachine extends WaitingRoomLaunchMachineInput = WaitingRoomLaunchMachineInput> = {
  id: string
  label: string
  machine: TMachine | null
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
    machineAllocationOffered?: boolean
  } = {},
): WaitingRoomLaunchMachineOption<TMachine>[] {
  return [
    { id: "local", label: "local", machine: null },
    ...(remote.machines ?? []).map(machine => ({ id: machine.machine_id, label: machine.machine_alias ?? machine.display_name ?? machine.machine_id, machine })),
    ...(remote.machineAllocationOffered ? [{ id: NEW_MACHINE_ALLOCATION_REF, label: "+ New machine...", machine: null }] : []),
  ]
}

export function waitingRoomSelectedLaunchMachineRef(
  state: Pick<WaitingRoomLaunchPlacementState, "selectedMachineRef">,
  remote: WaitingRoomLaunchRemoteState = {},
): string {
  const options = waitingRoomLaunchMachineOptions(remote)
  const selected = state.selectedMachineRef?.trim() || "local"
  if (selected === NEW_MACHINE_ALLOCATION_REF) return selected
  return options.some((option) => option.id === selected) ? selected : options[0]?.id ?? "local"
}

export function waitingRoomLaunchKernelOptions<TKernel extends WaitingRoomLaunchKernelInput>(
  remote: {
    kernels?: readonly TKernel[]
    machineAllocationOffered?: boolean
    relay?: WaitingRoomLaunchConnectedKernelInput | null
  } = {},
  machineRef = "local",
): WaitingRoomLaunchKernelOption<TKernel>[] {
  if (machineRef === "local") {
    return [{ id: "local", label: "local", machineId: "local", kernel: null }]
  }
  if (machineRef === NEW_MACHINE_ALLOCATION_REF) return []
  const options: WaitingRoomLaunchKernelOption<TKernel>[] = (remote.kernels ?? [])
    .filter((kernel) => kernel.machine_id === machineRef)
    .map((kernel) => ({
      id: kernel.kernel_id,
      label: kernel.relay_alias ?? kernel.kernel_alias ?? kernel.kernel_id,
      machineId: kernel.machine_id,
      kernel,
    }))
  // MP-08/MP-11: a kernel leaves itself out of its relay kernel list, so once this client is
  // connected to the chosen machine's kernel that kernel is known only from the relay status.
  const connected = remote.relay
  if (connected?.daemon_id && connected.machine_id === machineRef
    && !options.some((option) => option.id === connected.daemon_id)) {
    options.push({
      id: connected.daemon_id,
      label: connected.daemon_alias ?? connected.daemon_id,
      machineId: machineRef,
      kernel: null,
    })
  }
  return options
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
  newMachineAllocation: boolean
} {
  const machineRef = waitingRoomSelectedLaunchMachineRef(state, remote)
  const kernelRef = waitingRoomSelectedLaunchKernelRef(state, remote)
  return {
    machineRef,
    kernelRef,
    workerKernelRef: null,
    newMachineAllocation: machineRef === NEW_MACHINE_ALLOCATION_REF,
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
    if (machineRef === NEW_MACHINE_ALLOCATION_REF) return "created on acquisition"
    return "none available"
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
    selectedMachineRef: next?.machineId ?? machineRef,
    selectedKernelRef: next?.id ?? "",
  }
}

function modulo(value: number, size: number) {
  if (size <= 0) {
    return 0
  }
  return ((value % size) + size) % size
}
