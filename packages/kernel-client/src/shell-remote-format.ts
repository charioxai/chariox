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
    return `${name} id=${machine.machine_id} status=${machine.trust_status}${offline} kernels=${machine.kernel_count} providers=${providers}`
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
    return `no live kernels found for machine ${kernelRef}`
  }
  return kernels.map((kernel) => {
    const name = kernel.relay_alias ?? kernel.kernel_alias ?? kernel.kernel_id
    const providers = (kernel.available_providers ?? []).join(",") || "-"
    return `${name} id=${kernel.kernel_id} machine=${kernel.machine_alias ?? kernel.machine_id} providers=${providers} accepting_remote_leases=${String(kernel.accepting_remote_leases ?? false)} leased_agents=${kernel.leased_agent_count ?? 0} local_sessions=${kernel.local_session_count ?? 0}`
  }).join("\n")
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
