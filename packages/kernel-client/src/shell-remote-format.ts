import type {
  PairedClientRecord,
  PairingInviteRecord,
  PairingJoinRecord,
  RelayKernelPresence,
  RelayStatus,
  RemoteMachineRecord,
} from "./kernel-types.js"

export function formatRemoteMachines(machines: RemoteMachineRecord[]): string {
  if (machines.length === 0) {
    return "no remote machines"
  }
  return machines.map((machine) => {
    const name = formatRemoteMachineLabel(machine)
    const providers = (machine.available_providers ?? []).join(",") || "-"
    const offline = machine.online ? "" : ",offline"
    const next = remoteMachineNextAction(machine)
    return `${name} id=${machine.machine_id} status=${machine.trust_status}${offline} kernels=${machine.kernel_count} providers=${providers}${next ? ` next: ${next}` : ""}`
  }).join("\n")
}

export function formatRemoteMachineLabel(machine: RemoteMachineRecord): string {
  return machine.display_name || machine.machine_alias || machine.registry_alias || machine.machine_id
}

export function formatPairedClients(clients: PairedClientRecord[]): string {
  if (clients.length === 0) {
    return "no paired clients"
  }
  return clients.map((client) => {
    const label = formatPairedClientLabel(client)
    const revoked = client.revoked ? " revoked=true" : ""
    return `${label} thumbprint=${client.public_key_thumbprint} paired_at_ms=${client.paired_at_ms}${revoked}`
  }).join("\n")
}

export function formatPairedClientLabel(client: PairedClientRecord): string {
  return client.alias ? `${client.alias} id=${client.client_id}` : client.client_id
}

export function formatPairingInvite(invite: PairingInviteRecord): string {
  return [
    `${invite.intent} invite ${invite.invite_id}`,
    `target=${invite.target_daemon_alias ?? invite.target_daemon_id}`,
    `relay=${invite.relay_url}`,
    `expires_at_ms=${invite.expires_at_ms}`,
    `token=${invite.invite_token}`,
  ].join("\n")
}

export function formatPairingJoin(pairing: PairingJoinRecord): string {
  const alias = pairing.alias ? ` alias=${pairing.alias}` : ""
  return `joined ${pairing.intent} ${pairing.subject_id}${alias} target=${pairing.target_daemon_id} thumbprint=${pairing.public_key_thumbprint}`
}

export function formatRemoteKernels(kernels: RelayKernelPresence[], kernelRef: string): string {
  if (kernels.length === 0) {
    return `no live kernels found for machine ${kernelRef}; next: reconnect that machine or choose another worker`
  }
  return [
    formatRemoteKernelSummary(kernels, kernelRef),
    ...kernels.map((kernel) => {
      const name = kernel.relay_alias ?? kernel.kernel_alias ?? kernel.kernel_id
      const providers = (kernel.available_providers ?? []).join(",") || "-"
      const next = remoteKernelNextAction(kernel)
      const readiness = remoteKernelReadiness(kernel)
      return `${name} id=${kernel.kernel_id} machine=${kernel.machine_alias ?? kernel.machine_id} readiness=${readiness} providers=${providers} accepting_remote_leases=${formatAcceptingRemoteLeases(kernel.accepting_remote_leases)} leased_agents=${kernel.leased_agent_count ?? 0} local_sessions=${kernel.local_session_count ?? 0}${next ? ` next: ${next}` : ""}`
    }),
  ].join("\n")
}

function formatAcceptingRemoteLeases(value: boolean | undefined): string {
  return value === undefined ? "unknown" : String(value)
}

function remoteMachineNextAction(machine: RemoteMachineRecord): string {
  const machineLabel = formatRemoteMachineLabel(machine)
  if (machine.online === false) {
    return "connect or restart the remote kernel on this machine"
  }
  if (machine.trust_status !== "approved" || machine.pending) {
    return `approve with machine approve ${machine.machine_id}`
  }
  if (machine.kernel_count === 0) {
    return "start a kernel on this machine"
  }
  if ((machine.available_providers ?? []).length === 0) {
    return `configure provider CLIs on ${machineLabel}`
  }
  return ""
}

function remoteKernelNextAction(kernel: RelayKernelPresence): string {
  const kernelLabel = remoteKernelLabel(kernel)
  if (kernel.accepting_remote_leases === false) {
    return `enable remote leases on ${kernelLabel} or choose another worker`
  }
  if (kernel.accepting_remote_leases === undefined) {
    return `refresh ${kernelLabel} readiness or reconnect that worker before launching remote agents`
  }
  if ((kernel.available_providers ?? []).length === 0) {
    return `configure provider CLIs on ${kernelLabel}`
  }
  return ""
}

function formatRemoteKernelSummary(kernels: RelayKernelPresence[], kernelRef: string): string {
  const counts = kernels.reduce<Record<ReturnType<typeof remoteKernelReadiness>, number>>((acc, kernel) => {
    const readiness = remoteKernelReadiness(kernel)
    acc[readiness] += 1
    return acc
  }, { ready: 0, blocked: 0, "needs-provider": 0, unknown: 0 })
  const total = kernels.length
  const parts = [`${counts.ready}/${total} ready`]
  if (counts["needs-provider"] > 0) parts.push(`${counts["needs-provider"]} needs provider`)
  if (counts.blocked > 0) parts.push(`${counts.blocked} blocked`)
  if (counts.unknown > 0) parts.push(`${counts.unknown} unknown`)
  const next = counts.ready > 0
    ? "spawn a remote agent with a ready worker kernel"
    : "fix the listed kernel readiness issue or choose another machine"
  return `machine ${kernelRef} worker readiness: ${parts.join(", ")}; next: ${next}`
}

function remoteKernelReadiness(kernel: RelayKernelPresence): "ready" | "blocked" | "needs-provider" | "unknown" {
  if (kernel.accepting_remote_leases === false) return "blocked"
  if ((kernel.available_providers ?? []).length === 0) {
    return kernel.accepting_remote_leases === undefined ? "unknown" : "needs-provider"
  }
  if (kernel.accepting_remote_leases === undefined) return "unknown"
  return "ready"
}

function remoteKernelLabel(kernel: RelayKernelPresence): string {
  return kernel.relay_alias ?? kernel.kernel_alias ?? kernel.kernel_id
}

export function formatRelayStatus(status: RelayStatus): string {
  const state = !status.configured ? "not configured" : status.connected ? "connected" : "configured, disconnected"
  return [
    `relay ${state}`,
    `url=${status.relay_url ?? "-"}`,
    `token_configured=${String(status.relay_token_configured)}`,
    `daemon=${status.daemon_id}`,
    `machine=${status.machine_alias ?? status.machine_id}`,
  ].join("\n")
}
