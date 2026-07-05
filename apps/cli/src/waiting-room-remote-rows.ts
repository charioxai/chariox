import type {
  WaitingRoomRemoteKernel,
  WaitingRoomRemoteMachine,
  WaitingRoomRemoteState,
  WaitingRoomRow,
  WaitingRoomState,
} from "./waiting-room-types.js"
import {
  remoteKernelReadiness,
  remoteKernelReadinessCounts,
} from "@arroba/kernel-client/shell-remote-format"

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
      title: "Machines",
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
      title: "Machines",
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
      title: "Machines",
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
    const label = waitingRoomRemoteMachineLabel(machine)
    const providers = (machine.available_providers ?? []).join(",") || "no providers"
    const status = machine.online === false ? "offline" : machine.pending ? "pending" : "approved"
    const machineKernels = kernels.filter((kernel) => kernel.machine_id === machine.machine_id)
    const readinessSummary = waitingRoomMachineReadinessSummary(machine, machineKernels)
    const next = waitingRoomRemoteMachineNextAction(machine, machineKernels)
    rows.push({
      id: `machine:${machine.machine_id}`,
      title: `${label}${status !== "approved" ? ` (${status})` : ""}`,
      value: `${machine.kernel_count} kernel${machine.kernel_count === 1 ? "" : "s"} ${providers}${readinessSummary}${next ? ` · next: ${next}` : ""}`,
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
      title: "Kernels",
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
      const status = remoteKernelReadiness(kernel)
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
  return remoteKernelReadiness(kernel) === "ready"
}

export function waitingRoomRemoteKernelCanDelete(kernel: WaitingRoomRemoteKernel) {
  return kernel.accepting_remote_leases === false
    && (kernel.leased_agent_count ?? 0) === 0
    && (kernel.local_session_count ?? 0) === 0
}

function waitingRoomMachineReadinessSummary(machine: WaitingRoomRemoteMachine, kernels: readonly WaitingRoomRemoteKernel[]): string {
  if (machine.online === false || machine.pending || machine.kernel_count === 0 || kernels.length === 0) {
    return ""
  }
  const counts = remoteKernelReadinessCounts(kernels)
  const leased = kernels.reduce((sum, kernel) => sum + (kernel.leased_agent_count ?? 0), 0)
  const parts = [`ready=${counts.ready}/${kernels.length}`]
  if (counts["needs-provider"] > 0) parts.push(`needs-provider=${counts["needs-provider"]}`)
  if (counts["needs-account"] > 0) parts.push(`needs-account=${counts["needs-account"]}`)
  if (counts.blocked > 0) parts.push(`blocked=${counts.blocked}`)
  if (counts.unknown > 0) parts.push(`unknown=${counts.unknown}`)
  parts.push(`leased=${leased}`)
  return ` · ${parts.join(" ")}`
}

function waitingRoomRemoteMachineNextAction(machine: WaitingRoomRemoteMachine, kernels: readonly WaitingRoomRemoteKernel[] = []): string {
  const machineLabel = waitingRoomRemoteMachineLabel(machine)
  if (machine.online === false) {
    return "connect or restart the kernel"
  }
  if (machine.trust_status !== "approved" || machine.pending) {
    return `approve ${machine.machine_id}`
  }
  if (machine.kernel_count === 0) {
    return "start a kernel on this machine"
  }
  if (kernels.length > 0 && kernels.every((kernel) => remoteKernelReadiness(kernel) === "blocked")) {
    const [firstKernel] = kernels
    const kernelLabel = kernels.length === 1 && firstKernel
      ? waitingRoomRemoteKernelLabel(firstKernel)
      : "one of this machine's kernels"
    return `run /machine kernels ${machineLabel}; enable remote access on ${kernelLabel} or choose another kernel`
  }
  if (kernels.length > 0 && kernels.every((kernel) => remoteKernelReadiness(kernel) === "needs-provider")) {
    return `run /machine kernels ${machineLabel}; configure provider CLIs on ${machineLabel}`
  }
  if (kernels.length > 0 && kernels.every((kernel) => remoteKernelReadiness(kernel) === "needs-account")) {
    return `run /machine kernels ${machineLabel}; configure/import or refresh provider accounts on ${machineLabel}`
  }
  if (kernels.length > 0 && kernels.every((kernel) => remoteKernelReadiness(kernel) === "unknown")) {
    return `refresh ${machineLabel} or run /machine kernels ${machineLabel}`
  }
  if (kernels.length > 0 && !kernels.some((kernel) => remoteKernelReadiness(kernel) === "ready")) {
    return `fix listed kernel readiness issues on ${machineLabel} or choose another kernel`
  }
  if ((machine.available_providers ?? []).length === 0) {
    return `configure provider CLIs on ${machineLabel}`
  }
  return ""
}

function waitingRoomRemoteKernelNextAction(kernel: WaitingRoomRemoteKernel): string {
  const kernelLabel = waitingRoomRemoteKernelLabel(kernel)
  const machineLabel = kernel.machine_alias ?? kernel.machine_id
  const inspect = machineLabel ? `run /machine kernels ${machineLabel}; ` : ""
  const readiness = remoteKernelReadiness(kernel)
  if (readiness === "blocked") {
    return `${inspect}enable remote access on ${kernelLabel} or choose another kernel`
  }
  if (readiness === "unknown") {
    return `${inspect}refresh ${kernelLabel} readiness or reconnect that kernel before launching agents`
  }
  if (readiness === "needs-provider") {
    return `${inspect}configure provider CLIs on ${kernelLabel}`
  }
  if (readiness === "needs-account") {
    return `${inspect}configure/import or refresh provider accounts on ${kernelLabel}`
  }
  return ""
}

function waitingRoomRemoteMachineLabel(machine: WaitingRoomRemoteMachine): string {
  return machine.registry_alias ?? machine.machine_alias ?? machine.display_name ?? machine.machine_id
}

function waitingRoomRemoteKernelLabel(kernel: WaitingRoomRemoteKernel): string {
  return kernel.relay_alias ?? kernel.kernel_alias ?? kernel.kernel_id
}

function waitingRoomLoadingText(frame = 0) {
  return `loading${".".repeat(Math.abs(frame) % 4)}`
}
