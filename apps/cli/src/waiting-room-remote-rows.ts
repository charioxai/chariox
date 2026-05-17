import type {
  WaitingRoomRemoteKernel,
  WaitingRoomRemoteMachine,
  WaitingRoomRemoteState,
  WaitingRoomRow,
  WaitingRoomState,
} from "./waiting-room.js"

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

  for (const [index, machine] of machines.entries()) {
    const label = machine.display_name ?? machine.registry_alias ?? machine.machine_alias ?? machine.machine_id
    const providers = (machine.available_providers ?? []).join(",") || "no providers"
    const status = machine.online === false ? "offline" : machine.pending ? "pending" : "approved"
    rows.push({
      id: `machine:${machine.machine_id}`,
      title: `${label}${status !== "approved" ? ` (${status})` : ""}`,
      value: `${machine.kernel_count} kernel${machine.kernel_count === 1 ? "" : "s"} ${providers}`,
      titleWidth,
      indent: 1,
      focused: state.focus === "machine" && state.machineIndex === index,
      selectable: true,
      scrollbar: "",
    })
  }
  const kernels = waitingRoomRemoteKernels(remote)
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
      rows.push({
        id: `remote-kernel:${kernel.kernel_id}`,
        title: `${label} @ ${machine}`,
        value: `${status} ${providers}`,
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

function waitingRoomLoadingText(frame = 0) {
  return `loading${".".repeat(Math.abs(frame) % 4)}`
}
