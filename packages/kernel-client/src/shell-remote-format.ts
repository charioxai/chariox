import type {
  PairedClientRecord,
  PairingInviteRecord,
  PairingJoinRecord,
  ProviderAccountSummary,
  RelayKernelPresence,
  RelayStatus,
  RemoteMachineRecord,
} from "./kernel-types.js"
import { formatSliceProviderAuth } from "./slice-format.js"

export function formatRemoteMachines(machines: RemoteMachineRecord[]): string {
  if (machines.length === 0) {
    return "no remote machines"
  }
  return machines.map((machine) => {
    const name = formatRemoteMachineLabel(machine)
    const providers = (machine.available_providers ?? []).join(",") || "-"
    const accounts = formatProviderAccounts(machine.provider_accounts)
    const offline = machine.online ? "" : ",offline"
    const next = remoteMachineNextAction(machine)
    return `${name} id=${machine.machine_id} status=${machine.trust_status}${offline} kernels=${machine.kernel_count} providers=${providers} accounts=${accounts}${next ? ` next: ${next}` : ""}`
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
      const accounts = formatRemoteKernelProviderAccounts(kernel)
      const next = remoteKernelNextAction(kernel)
      const readiness = remoteKernelReadiness(kernel)
      return `${name} id=${kernel.kernel_id} machine=${kernel.machine_alias ?? kernel.machine_id} readiness=${readiness} providers=${providers} accounts=${accounts} accepting_remote_leases=${formatAcceptingRemoteLeases(kernel.accepting_remote_leases)} leased_agents=${kernel.leased_agent_count ?? 0} local_sessions=${kernel.local_session_count ?? 0}${next ? ` next: ${next}` : ""}`
    }),
  ].join("\n")
}

function formatRemoteKernelProviderAccounts(kernel: RelayKernelPresence): string {
  return formatProviderAccounts(kernel.provider_accounts)
}

function formatProviderAccounts(accountsInput: readonly ProviderAccountSummary[] | null | undefined): string {
  const accounts = accountsInput ?? []
  if (accounts.length === 0) {
    return "none"
  }
  return accounts.map((entry) => formatSliceProviderAuth(entry, {
    separator: "=",
    includeOrgPlan: false,
  })).join(",")
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
  const accountRecovery = remoteMachineProviderAccountNextAction(machine)
  if (accountRecovery) {
    return accountRecovery
  }
  return ""
}

function remoteMachineProviderAccountNextAction(machine: RemoteMachineRecord): string {
  const providers = machine.available_providers ?? []
  if (providers.length === 0) {
    return ""
  }
  const accounts = machine.provider_accounts ?? []
  if (!providerAccountsNeedRecovery(providers, accounts)) {
    return ""
  }
  const machineLabel = formatRemoteMachineLabel(machine)
  return `run /machine kernels ${machineLabel}; configure/import or refresh provider accounts before spawning remote agents`
}

function providerAccountsNeedRecovery(
  providers: readonly string[],
  accounts: readonly ProviderAccountSummary[],
): boolean {
  return providers.some((provider) => {
    const account = accounts.find((entry) => entry.provider === provider)
    return !account || providerAccountNeedsAttention(account)
  })
}

function providerAccountNeedsAttention(account: ProviderAccountSummary): boolean {
  return account.state !== "authenticated"
}

function remoteKernelNextAction(kernel: RelayKernelPresence): string {
  const kernelLabel = remoteKernelLabel(kernel)
  const machine = kernel.machine_alias ?? kernel.machine_id
  const inspect = machine ? `run /machine kernels ${machine}; ` : ""
  if (kernel.accepting_remote_leases === false) {
    return `${inspect}enable remote leases on ${kernelLabel} or choose another worker`
  }
  if (kernel.accepting_remote_leases === undefined) {
    return `${inspect}refresh ${kernelLabel} readiness or reconnect that worker before launching remote agents`
  }
  if ((kernel.available_providers ?? []).length === 0) {
    return `${inspect}configure provider CLIs on ${kernelLabel}`
  }
  const accountRecovery = remoteKernelProviderAccountNextAction(kernel)
  if (accountRecovery) {
    return accountRecovery
  }
  return ""
}

function remoteKernelProviderAccountNextAction(kernel: RelayKernelPresence): string {
  const providers = kernel.available_providers ?? []
  if (providers.length === 0) {
    return ""
  }
  const accounts = kernel.provider_accounts ?? []
  if (!providerAccountsNeedRecovery(providers, accounts)) {
    return ""
  }
  return `configure/import or refresh provider accounts on ${remoteKernelLabel(kernel)} before spawning remote agents`
}

export type RemoteKernelReadiness = "ready" | "blocked" | "needs-provider" | "needs-account" | "unknown"

function formatRemoteKernelSummary(kernels: RelayKernelPresence[], kernelRef: string): string {
  const counts = remoteKernelReadinessCounts(kernels)
  const total = kernels.length
  const parts = [`${counts.ready}/${total} ready`]
  if (counts["needs-provider"] > 0) parts.push(`${counts["needs-provider"]} needs provider`)
  if (counts["needs-account"] > 0) parts.push(`${counts["needs-account"]} needs account`)
  if (counts.blocked > 0) parts.push(`${counts.blocked} blocked`)
  if (counts.unknown > 0) parts.push(`${counts.unknown} unknown`)
  const next = counts.ready > 0
    ? "spawn a remote agent with a ready worker kernel"
    : "fix the listed kernel readiness issue or choose another machine"
  return `machine ${kernelRef} worker readiness: ${parts.join(", ")}; next: ${next}`
}

export function remoteKernelReadiness(kernel: RelayKernelPresence): RemoteKernelReadiness {
  if (kernel.accepting_remote_leases === false) return "blocked"
  if ((kernel.available_providers ?? []).length === 0) {
    return kernel.accepting_remote_leases === undefined ? "unknown" : "needs-provider"
  }
  if (kernel.accepting_remote_leases === undefined) return "unknown"
  if (
    "provider_accounts" in kernel
    && providerAccountsNeedRecovery(kernel.available_providers ?? [], kernel.provider_accounts ?? [])
  ) {
    return "needs-account"
  }
  return "ready"
}

export function remoteKernelReadinessCounts(
  kernels: readonly RelayKernelPresence[],
): Record<RemoteKernelReadiness, number> {
  return kernels.reduce<Record<RemoteKernelReadiness, number>>((acc, kernel) => {
    const readiness = remoteKernelReadiness(kernel)
    acc[readiness] += 1
    return acc
  }, { ready: 0, blocked: 0, "needs-provider": 0, "needs-account": 0, unknown: 0 })
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
