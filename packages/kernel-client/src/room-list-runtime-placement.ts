export type RoomListLaunchMachineInput = {
  machine_id: string
  machine_alias?: string | null
  display_name?: string | null
}

export type RoomListLaunchKernelInput = {
  kernel_id: string
  machine_id: string
  kernel_alias?: string | null
  relay_alias?: string | null
}

/** The kernel this client is connected to, as its relay status reports it. */
export type RoomListLaunchConnectedKernelInput = {
  daemon_id?: string | null
  daemon_alias?: string | null
  machine_id?: string | null
}

export type RoomListLaunchRemoteState = {
  machineAllocationOffered?: boolean
  machines?: readonly RoomListLaunchMachineInput[]
  kernels?: readonly RoomListLaunchKernelInput[]
  relay?: RoomListLaunchConnectedKernelInput | null
}

export const NEW_MACHINE_ALLOCATION_REF = "allocation:new"

export type RoomListLaunchPlacementState = {
  selectedMachineRef?: string | null
  selectedKernelRef?: string | null
}

export type RoomListLaunchMachineOption<TMachine extends RoomListLaunchMachineInput = RoomListLaunchMachineInput> = {
  id: string
  label: string
  machine: TMachine | null
}

export type RoomListLaunchKernelOption<TKernel extends RoomListLaunchKernelInput = RoomListLaunchKernelInput> = {
  id: string
  label: string
  machineId: string
  kernel: TKernel | null
}

export function roomListLaunchMachineOptions<TMachine extends RoomListLaunchMachineInput>(
  remote: {
    machines?: readonly TMachine[]
    machineAllocationOffered?: boolean
  } = {},
): RoomListLaunchMachineOption<TMachine>[] {
  return [
    { id: "local", label: "local", machine: null },
    ...(remote.machines ?? []).map(machine => ({ id: machine.machine_id, label: machine.machine_alias ?? machine.display_name ?? machine.machine_id, machine })),
    ...(remote.machineAllocationOffered ? [{ id: NEW_MACHINE_ALLOCATION_REF, label: "+ New machine...", machine: null }] : []),
  ]
}

export function roomListSelectedLaunchMachineRef(
  state: Pick<RoomListLaunchPlacementState, "selectedMachineRef">,
  remote: RoomListLaunchRemoteState = {},
): string {
  const options = roomListLaunchMachineOptions(remote)
  const selected = state.selectedMachineRef?.trim() || "local"
  if (selected === NEW_MACHINE_ALLOCATION_REF) return selected
  return options.some((option) => option.id === selected) ? selected : options[0]?.id ?? "local"
}

export function roomListLaunchKernelOptions<TKernel extends RoomListLaunchKernelInput>(
  remote: {
    kernels?: readonly TKernel[]
    machineAllocationOffered?: boolean
    relay?: RoomListLaunchConnectedKernelInput | null
  } = {},
  machineRef = "local",
): RoomListLaunchKernelOption<TKernel>[] {
  if (machineRef === "local") {
    return [{ id: "local", label: "local", machineId: "local", kernel: null }]
  }
  if (machineRef === NEW_MACHINE_ALLOCATION_REF) return []
  const options: RoomListLaunchKernelOption<TKernel>[] = (remote.kernels ?? [])
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

export function roomListSelectedLaunchKernelRef(
  state: Pick<RoomListLaunchPlacementState, "selectedMachineRef" | "selectedKernelRef">,
  remote: RoomListLaunchRemoteState = {},
): string {
  const machineRef = roomListSelectedLaunchMachineRef(state, remote)
  const options = roomListLaunchKernelOptions(remote, machineRef)
  const selected = state.selectedKernelRef?.trim() || (machineRef === "local" ? "local" : "")
  return options.some((option) => option.id === selected) ? selected : options[0]?.id ?? ""
}

export function roomListLaunchPlacement(
  state: Pick<RoomListLaunchPlacementState, "selectedMachineRef" | "selectedKernelRef">,
  remote: RoomListLaunchRemoteState = {},
): {
  machineRef: string
  kernelRef: string
  workerKernelRef: string | null
  newMachineAllocation: boolean
} {
  const machineRef = roomListSelectedLaunchMachineRef(state, remote)
  const kernelRef = roomListSelectedLaunchKernelRef(state, remote)
  return {
    machineRef,
    kernelRef,
    workerKernelRef: null,
    newMachineAllocation: machineRef === NEW_MACHINE_ALLOCATION_REF,
  }
}

export function formatRoomListLaunchMachineValue(
  state: Pick<RoomListLaunchPlacementState, "selectedMachineRef">,
  remote: RoomListLaunchRemoteState = {},
): string {
  const selected = roomListSelectedLaunchMachineRef(state, remote)
  return roomListLaunchMachineOptions(remote).find((option) => option.id === selected)?.label ?? selected
}

export function formatRoomListLaunchKernelValue(
  state: Pick<RoomListLaunchPlacementState, "selectedMachineRef" | "selectedKernelRef">,
  remote: RoomListLaunchRemoteState = {},
): string {
  const machineRef = roomListSelectedLaunchMachineRef(state, remote)
  const selected = roomListSelectedLaunchKernelRef(state, remote)
  if (!selected) {
    if (machineRef === NEW_MACHINE_ALLOCATION_REF) return "created on acquisition"
    return "none available"
  }
  return roomListLaunchKernelOptions(remote, machineRef).find((option) => option.id === selected)?.label ?? selected
}

export function normalizeRoomListLaunchPlacement(
  state: RoomListLaunchPlacementState,
  remote: RoomListLaunchRemoteState = {},
): { selectedMachineRef: string; selectedKernelRef: string } {
  const selectedMachineRef = roomListSelectedLaunchMachineRef(state, remote)
  const selectedKernelRef = roomListSelectedLaunchKernelRef({ ...state, selectedMachineRef }, remote)
  return { selectedMachineRef, selectedKernelRef }
}

export function cycleRoomListLaunchMachine<TState extends RoomListLaunchPlacementState>(
  state: TState,
  remote: RoomListLaunchRemoteState = {},
  delta: number,
): TState {
  const options = roomListLaunchMachineOptions(remote)
  const current = roomListSelectedLaunchMachineRef(state, remote)
  const index = Math.max(0, options.findIndex((option) => option.id === current))
  const nextMachineRef = options[modulo(index + delta, options.length)]?.id ?? "local"
  const nextKernelRef = roomListLaunchKernelOptions(remote, nextMachineRef)[0]?.id ?? ""
  return {
    ...state,
    selectedMachineRef: nextMachineRef,
    selectedKernelRef: nextKernelRef,
  }
}

export function cycleRoomListLaunchKernel<TState extends RoomListLaunchPlacementState>(
  state: TState,
  remote: RoomListLaunchRemoteState = {},
  delta: number,
): TState {
  const machineRef = roomListSelectedLaunchMachineRef(state, remote)
  const options = roomListLaunchKernelOptions(remote, machineRef)
  if (options.length === 0) {
    return { ...state, selectedKernelRef: "" }
  }
  const current = roomListSelectedLaunchKernelRef(state, remote)
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
