import type {
  WaitingRoomRemoteKernel,
  WaitingRoomRemoteMachine,
  WaitingRoomRemoteState,
  WaitingRoomRow,
  WaitingRoomState,
} from "./waiting-room-types.js"

export function waitingRoomRemoteRows(
  state: Pick<WaitingRoomState, "focus" | "machineIndex" | "remoteKernelIndex">,
  remote: WaitingRoomRemoteState,
  titleWidth: number,
): WaitingRoomRow[] {
  const inventoryLoading = remote.inventoryStatus === "loading"
  const loadingText = waitingRoomLoadingText(remote.loadingFrame)
  const relay = remote.relay ?? null
  const machines = remote.machines ?? []
  const pendingCount = machines.filter((machine) => machine.pending).length
  const onlineMachines = machines.filter((machine) => machine.online !== false)
  const relayStatus = !relay || !relay.configured
    ? inventoryLoading
      ? loadingText
      : "not configured"
    : relay.connected
      ? `connected ${relay.relay_url ?? ""}`.trim()
      : `connecting ${relay.relay_url ?? ""}`.trim()
  const rows: WaitingRoomRow[] = [
    {
      id: "relay-header",
      title: "Relay",
      value: relayStatus,
      titleWidth,
      indent: 0,
      focused: false,
      selectable: false,
      scrollbar: "",
    },
    {
      id: "relay-configure",
      title: "Cloud",
      value: "/cloud",
      titleWidth,
      indent: 1,
      focused: state.focus === "relay",
      selectable: true,
      scrollbar: "",
    },
    {
      id: "machines-header",
      title: "Machines",
      value: inventoryLoading && machines.length === 0
        ? loadingText
        : `${onlineMachines.length} online${pendingCount > 0 ? ` (${pendingCount} pending)` : ""}`,
      titleWidth,
      indent: 0,
      focused: false,
      selectable: false,
      scrollbar: "",
    },
  ]
  const cloudNoticeLines = (remote.cloudNotice ?? "")
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
  for (const [index, line] of cloudNoticeLines.entries()) {
    rows.push({
      id: `cloud-notice:${index}`,
      title: index === 0 ? "Cloud status" : "",
      value: line,
      titleWidth,
      indent: 2,
      focused: false,
      selectable: false,
      scrollbar: "",
    })
  }

  if (inventoryLoading && machines.length === 0 && waitingRoomRemoteKernels(remote).length === 0) {
    rows.push({
      id: "machines-loading",
      title: "Remote Machines",
      value: loadingText,
      titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    })
    return rows
  }

  if (!relay?.configured && machines.length === 0 && waitingRoomRemoteKernels(remote).length === 0) {
    rows.push({
      id: "machines-unavailable",
      title: "Remote Machines",
      value: "unavailable until relay is configured",
      titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    })
    return rows
  }

  if (machines.length === 0) {
    rows.push({
      id: "machines-none",
      title: "Remote Machines",
      value: relay?.connected ? "none online" : "waiting for relay connection",
      titleWidth,
      indent: 1,
      focused: false,
      selectable: false,
      scrollbar: "",
    })
    return rows
  }

  const kernels = waitingRoomRemoteKernels(remote)
  for (const [index, machine] of machines.entries()) {
    const label = machine.display_name ?? machine.registry_alias ?? machine.machine_alias ?? machine.machine_id
    const providers = (machine.available_providers ?? []).join(",") || "no providers"
    const status = machine.online === false ? "offline" : machine.pending ? "pending" : "approved"
    const machineKernels = kernels.filter((kernel) => kernel.machine_id === machine.machine_id)
    const leaseSummary = waitingRoomMachineLeaseSummary(machine, machineKernels)
    const next = waitingRoomRemoteMachineNextAction(machine, machineKernels)
    rows.push({
      id: `machine:${machine.machine_id}`,
      title: `${label}${status !== "approved" ? ` (${status})` : ""}`,
      value: `${machine.kernel_count} kernel${machine.kernel_count === 1 ? "" : "s"} ${providers}${leaseSummary}${next ? ` · next: ${next}` : ""}`,
      titleWidth,
      indent: 1,
      focused: state.focus === "machine" && state.machineIndex === index,
      selectable: true,
      scrollbar: "",
    })
  }
  if (kernels.length > 0) {
    rows.push({
      id: "remote-kernels-header",
      title: "Remote Kernels",
      value: "",
      titleWidth,
      indent: 0,
      focused: false,
      selectable: false,
      scrollbar: "",
    })
    for (const [index, kernel] of kernels.entries()) {
      const label = kernel.relay_alias ?? kernel.kernel_alias ?? kernel.kernel_id
      const machine = kernel.machine_alias ?? kernel.machine_id
      const providers = (kernel.available_providers ?? []).join(",") || "no providers"
      const status = waitingRoomRemoteKernelIsAttachable(kernel) ? "ready" : "inactive"
      const next = waitingRoomRemoteKernelNextAction(kernel)
      rows.push({
        id: `remote-kernel:${kernel.kernel_id}`,
        title: `${label} @ ${machine}`,
        value: `${status} ${providers}${next ? ` · next: ${next}` : ""}`,
        titleWidth,
        indent: 1,
        focused: state.focus === "remote-kernel" && state.remoteKernelIndex === index,
        selectable: true,
        scrollbar: "",
      })
    }
  }
  return rows
}

export function waitingRoomRemoteMachines(remote: WaitingRoomRemoteState) {
  return remote.machines ?? []
}

export function waitingRoomRemoteKernels(remote: WaitingRoomRemoteState) {
  return remote.kernels ?? []
}

export function waitingRoomRemoteMachineCanDelete(machine: WaitingRoomRemoteMachine) {
  return machine.online === false
    || machine.pending === true
    || machine.trust_status === "forgotten"
    || machine.kernel_count === 0
}

export function waitingRoomRemoteKernelIsAttachable(kernel: WaitingRoomRemoteKernel) {
  return kernel.accepting_remote_leases !== false
}

export function waitingRoomRemoteKernelCanDelete(kernel: WaitingRoomRemoteKernel) {
  return kernel.accepting_remote_leases === false
    && (kernel.leased_agent_count ?? 0) === 0
    && (kernel.local_session_count ?? 0) === 0
}

function waitingRoomMachineLeaseSummary(machine: WaitingRoomRemoteMachine, kernels: readonly WaitingRoomRemoteKernel[]): string {
  if (machine.online === false || machine.pending || machine.kernel_count === 0 || kernels.length === 0) {
    return ""
  }
  const accepting = kernels.filter((kernel) => kernel.accepting_remote_leases !== false).length
  const leased = kernels.reduce((sum, kernel) => sum + (kernel.leased_agent_count ?? 0), 0)
  return ` · accepting=${accepting}/${kernels.length} leased=${leased}`
}

function waitingRoomRemoteMachineNextAction(machine: WaitingRoomRemoteMachine, kernels: readonly WaitingRoomRemoteKernel[] = []): string {
  const machineLabel = waitingRoomRemoteMachineLabel(machine)
  if (machine.online === false) {
    return "connect or restart the remote kernel"
  }
  if (machine.trust_status !== "approved" || machine.pending) {
    return `approve ${machine.machine_id}`
  }
  if (machine.kernel_count === 0) {
    return "start a kernel on this machine"
  }
  if ((machine.available_providers ?? []).length === 0) {
    return `configure provider CLIs on ${machineLabel}`
  }
  if (kernels.length > 0 && kernels.every((kernel) => kernel.accepting_remote_leases === false)) {
    const [firstKernel] = kernels
    const kernelLabel = kernels.length === 1 && firstKernel
      ? waitingRoomRemoteKernelLabel(firstKernel)
      : "one of this machine's kernels"
    return `enable remote leases on ${kernelLabel} or choose another worker`
  }
  return ""
}

function waitingRoomRemoteKernelNextAction(kernel: WaitingRoomRemoteKernel): string {
  const kernelLabel = waitingRoomRemoteKernelLabel(kernel)
  if (kernel.accepting_remote_leases === false) {
    return `enable remote leases on ${kernelLabel} or choose another worker`
  }
  if ((kernel.available_providers ?? []).length === 0) {
    return `configure provider CLIs on ${kernelLabel}`
  }
  return ""
}

function waitingRoomRemoteMachineLabel(machine: WaitingRoomRemoteMachine): string {
  return machine.display_name ?? machine.registry_alias ?? machine.machine_alias ?? machine.machine_id
}

function waitingRoomRemoteKernelLabel(kernel: WaitingRoomRemoteKernel): string {
  return kernel.relay_alias ?? kernel.kernel_alias ?? kernel.kernel_id
}

function waitingRoomLoadingText(frame = 0) {
  return `loading${".".repeat(Math.abs(frame) % 4)}`
}
