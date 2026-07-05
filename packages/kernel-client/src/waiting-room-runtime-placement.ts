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
}

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
  remote: { machines?: readonly TMachine[] } = {},
): WaitingRoomLaunchMachineOption<TMachine>[] {
  return [
    { id: "local", label: "local", machine: null },
    ...(remote.machines ?? []).map((machine) => ({
      id: machine.machine_id,
      label: machine.registry_alias ?? machine.machine_alias ?? machine.display_name ?? machine.machine_id,
      machine,
    })),
  ]
}

export function waitingRoomSelectedLaunchMachineRef(
  state: Pick<WaitingRoomLaunchPlacementState, "selectedMachineRef">,
  remote: WaitingRoomLaunchRemoteState = {},
): string {
  const options = waitingRoomLaunchMachineOptions(remote)
  const selected = state.selectedMachineRef?.trim() || "local"
  return options.some((option) => option.id === selected) ? selected : options[0]?.id ?? "local"
}

export function waitingRoomLaunchKernelOptions<TKernel extends WaitingRoomLaunchKernelInput>(
  remote: { kernels?: readonly TKernel[] } = {},
  machineRef = "local",
): WaitingRoomLaunchKernelOption<TKernel>[] {
  if (machineRef === "local") {
    return [{ id: "local", label: "local", machineId: "local", kernel: null }]
  }
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
): { machineRef: string; kernelRef: string; workerKernelRef: string | null } {
  const machineRef = waitingRoomSelectedLaunchMachineRef(state, remote)
  const kernelRef = waitingRoomSelectedLaunchKernelRef(state, remote)
  return {
    machineRef,
    kernelRef,
    workerKernelRef: null,
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
  if (!selected) return "none available"
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
