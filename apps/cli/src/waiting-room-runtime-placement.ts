import type {
  WaitingRoomRemoteKernel,
  WaitingRoomRemoteMachine,
  WaitingRoomRemoteState,
  WaitingRoomState,
} from "./waiting-room-types.js"

export type WaitingRoomLaunchMachineOption = {
  id: string
  label: string
  machine: WaitingRoomRemoteMachine | null
}

export type WaitingRoomLaunchKernelOption = {
  id: string
  label: string
  machineId: string
  kernel: WaitingRoomRemoteKernel | null
}

export function waitingRoomLaunchMachineOptions(remote: WaitingRoomRemoteState = {}): WaitingRoomLaunchMachineOption[] {
  return [
    { id: "local", label: "local", machine: null },
    ...(remote.machines ?? []).map((machine) => ({
      id: machine.machine_id,
      label: machine.display_name ?? machine.registry_alias ?? machine.machine_alias ?? machine.machine_id,
      machine,
    })),
  ]
}

export function waitingRoomSelectedLaunchMachineRef(
  state: Pick<WaitingRoomState, "selectedMachineRef">,
  remote: WaitingRoomRemoteState = {},
): string {
  const options = waitingRoomLaunchMachineOptions(remote)
  const selected = state.selectedMachineRef?.trim() || "local"
  return options.some((option) => option.id === selected) ? selected : options[0]?.id ?? "local"
}

export function waitingRoomLaunchKernelOptions(
  remote: WaitingRoomRemoteState = {},
  machineRef = "local",
): WaitingRoomLaunchKernelOption[] {
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
  state: Pick<WaitingRoomState, "selectedMachineRef" | "selectedKernelRef">,
  remote: WaitingRoomRemoteState = {},
): string {
  const machineRef = waitingRoomSelectedLaunchMachineRef(state, remote)
  const options = waitingRoomLaunchKernelOptions(remote, machineRef)
  const selected = state.selectedKernelRef?.trim() || (machineRef === "local" ? "local" : "")
  return options.some((option) => option.id === selected) ? selected : options[0]?.id ?? ""
}

export function waitingRoomLaunchPlacement(
  state: Pick<WaitingRoomState, "selectedMachineRef" | "selectedKernelRef">,
  remote: WaitingRoomRemoteState = {},
): { machineRef: string; kernelRef: string; workerKernelRef: string | null } {
  const machineRef = waitingRoomSelectedLaunchMachineRef(state, remote)
  const kernelRef = waitingRoomSelectedLaunchKernelRef(state, remote)
  return {
    machineRef,
    kernelRef,
    workerKernelRef: kernelRef && kernelRef !== "local" ? kernelRef : null,
  }
}

export function formatWaitingRoomLaunchMachineValue(
  state: Pick<WaitingRoomState, "selectedMachineRef">,
  remote: WaitingRoomRemoteState = {},
): string {
  const selected = waitingRoomSelectedLaunchMachineRef(state, remote)
  return waitingRoomLaunchMachineOptions(remote).find((option) => option.id === selected)?.label ?? selected
}

export function formatWaitingRoomLaunchKernelValue(
  state: Pick<WaitingRoomState, "selectedMachineRef" | "selectedKernelRef">,
  remote: WaitingRoomRemoteState = {},
): string {
  const machineRef = waitingRoomSelectedLaunchMachineRef(state, remote)
  const selected = waitingRoomSelectedLaunchKernelRef(state, remote)
  if (!selected) return "none available"
  return waitingRoomLaunchKernelOptions(remote, machineRef).find((option) => option.id === selected)?.label ?? selected
}

export function normalizeWaitingRoomLaunchPlacement(
  state: WaitingRoomState,
  remote: WaitingRoomRemoteState = {},
): { selectedMachineRef: string; selectedKernelRef: string } {
  const selectedMachineRef = waitingRoomSelectedLaunchMachineRef(state, remote)
  const selectedKernelRef = waitingRoomSelectedLaunchKernelRef({ ...state, selectedMachineRef }, remote)
  return { selectedMachineRef, selectedKernelRef }
}

export function cycleWaitingRoomLaunchMachine(
  state: WaitingRoomState,
  remote: WaitingRoomRemoteState = {},
  delta: number,
): WaitingRoomState {
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

export function cycleWaitingRoomLaunchKernel(
  state: WaitingRoomState,
  remote: WaitingRoomRemoteState = {},
  delta: number,
): WaitingRoomState {
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
