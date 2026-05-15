import type {
  PairedClientRecord,
  PairingInviteRecord,
  PairingJoinRecord,
  RelayKernelPresence,
  RelayStatus,
  RemoteMachineRecord,
} from "./kernel-types.js"
import {
  approveRemoteMachineRequest,
  createPairingInviteRequest,
  forgetRemoteMachineRequest,
  joinPairingInviteRequest,
  listPairedClientsRequest,
  listRemoteMachineKernelsRequest,
  listRemoteMachinesRequest,
  recordPairedClientRequest,
  relayStatusRequest,
  renameRemoteMachineRequest,
  revokePairedClientRequest,
} from "./ipc-requests.js"
import type { ParsedShellCommand, ShellCommandResult } from "./shell-core.js"
import {
  formatPairedClientLabel,
  formatPairedClients,
  formatPairingInvite,
  formatPairingJoin,
  formatRelayStatus,
  formatRemoteKernels,
  formatRemoteMachineLabel,
  formatRemoteMachines,
} from "./shell-remote-format.js"

type ShellRemoteCommandDeps = {
  client: {
    send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  }
}

export async function executeClientCommand(
  parsed: ParsedShellCommand,
  deps: ShellRemoteCommandDeps,
): Promise<ShellCommandResult> {
  const [action, clientId, publicKeyThumbprint, ...rest] = parsed.args
  switch (action) {
    case "invite": {
      if (clientId !== "create") {
        return { ok: false, message: "usage: client invite create [alias]" }
      }
      const alias = publicKeyThumbprint ? [publicKeyThumbprint, ...rest].join(" ") : null
      const response = await deps.client.send(createPairingInviteRequest("client", alias))
      const payload = expectVariant<{ invite: PairingInviteRecord }>(response, "PairingInviteCreated")
      return { ok: true, message: formatPairingInvite(payload.invite), data: payload }
    }
    case "join": {
      if (!clientId) {
        return { ok: false, message: "usage: client join <invite-token> [client-id] [alias]" }
      }
      const alias = rest.length > 0 ? rest.join(" ") : null
      const response = await deps.client.send(joinPairingInviteRequest(clientId, publicKeyThumbprint ?? null, null, alias))
      const payload = expectVariant<{ pairing: PairingJoinRecord }>(response, "PairingInviteJoined")
      return { ok: true, message: formatPairingJoin(payload.pairing), data: payload }
    }
    case "list":
    case "ls": {
      const response = await deps.client.send(listPairedClientsRequest())
      const clients = expectVariant<{ clients: PairedClientRecord[] }>(response, "PairedClientsListed").clients
      return { ok: true, message: formatPairedClients(clients), data: { clients } }
    }
    case "record": {
      if (!clientId || !publicKeyThumbprint) {
        return { ok: false, message: "usage: client record <client-id> <public-key-thumbprint> [alias]" }
      }
      const alias = rest.length > 0 ? rest.join(" ") : null
      const response = await deps.client.send(recordPairedClientRequest(clientId, publicKeyThumbprint, alias))
      const payload = expectVariant<{ client: PairedClientRecord }>(response, "PairedClientRecorded")
      return { ok: true, message: `paired client ${formatPairedClientLabel(payload.client)}`, data: payload }
    }
    case "revoke": {
      if (!clientId) {
        return { ok: false, message: "usage: client revoke <client-id>" }
      }
      const response = await deps.client.send(revokePairedClientRequest(clientId))
      const payload = expectVariant<{ client: PairedClientRecord }>(response, "PairedClientRevoked")
      return { ok: true, message: `revoked client ${formatPairedClientLabel(payload.client)}`, data: payload }
    }
    default:
      return { ok: false, message: "usage: client invite create|join|list|record|revoke" }
  }
}

export async function executeMachineCommand(
  parsed: ParsedShellCommand,
  deps: ShellRemoteCommandDeps,
): Promise<ShellCommandResult> {
  const [action, kernelRef, ...rest] = parsed.args
  switch (action) {
    case "invite": {
      if (kernelRef !== "create") {
        return { ok: false, message: "usage: machine invite create [alias]" }
      }
      const alias = rest.length > 0 ? rest.join(" ") : null
      const response = await deps.client.send(createPairingInviteRequest("machine", alias))
      const payload = expectVariant<{ invite: PairingInviteRecord }>(response, "PairingInviteCreated")
      return { ok: true, message: formatPairingInvite(payload.invite), data: payload }
    }
    case "join": {
      if (!kernelRef) {
        return { ok: false, message: "usage: machine join <invite-token> [machine-id] [alias]" }
      }
      const subjectId = rest[0] ?? null
      const alias = rest.length > 1 ? rest.slice(1).join(" ") : null
      const response = await deps.client.send(joinPairingInviteRequest(kernelRef, subjectId, null, alias))
      const payload = expectVariant<{ pairing: PairingJoinRecord }>(response, "PairingInviteJoined")
      return { ok: true, message: formatPairingJoin(payload.pairing), data: payload }
    }
    case "list":
    case "ls": {
      const response = await deps.client.send(listRemoteMachinesRequest())
      const machines = expectVariant<{ machines: RemoteMachineRecord[] }>(response, "RemoteMachinesListed").machines
      return { ok: true, message: formatRemoteMachines(machines), data: { machines } }
    }
    case "kernels": {
      if (!kernelRef) {
        return { ok: false, message: "usage: machine kernels <machine-ref>" }
      }
      const response = await deps.client.send(listRemoteMachineKernelsRequest(kernelRef))
      const payload = expectVariant<{ kernels: RelayKernelPresence[] }>(response, "RemoteMachineKernelsListed")
      return { ok: true, message: formatRemoteKernels(payload.kernels, kernelRef), data: payload }
    }
    case "approve": {
      if (!kernelRef) {
        return { ok: false, message: "usage: machine approve <machine-ref>" }
      }
      const response = await deps.client.send(approveRemoteMachineRequest(kernelRef))
      const payload = expectVariant<{ machine: RemoteMachineRecord }>(response, "RemoteMachineApproved")
      return { ok: true, message: `approved machine ${formatRemoteMachineLabel(payload.machine)}`, data: payload }
    }
    case "rename": {
      if (!kernelRef || rest.length === 0) {
        return { ok: false, message: "usage: machine rename <machine-ref> <alias>" }
      }
      const alias = rest.join(" ")
      const response = await deps.client.send(renameRemoteMachineRequest(kernelRef, alias))
      const payload = expectVariant<{ machine: RemoteMachineRecord }>(response, "RemoteMachineRenamed")
      return { ok: true, message: `renamed machine ${formatRemoteMachineLabel(payload.machine)}`, data: payload }
    }
    case "forget":
    case "revoke": {
      if (!kernelRef) {
        return { ok: false, message: "usage: machine revoke <machine-ref>" }
      }
      const response = await deps.client.send(forgetRemoteMachineRequest(kernelRef))
      const payload = expectVariant<{ machine: RemoteMachineRecord }>(response, "RemoteMachineForgotten")
      return { ok: true, message: `revoked machine ${formatRemoteMachineLabel(payload.machine)}`, data: payload }
    }
    default:
      return { ok: false, message: "usage: machine invite create|join|list|kernels|approve|rename|revoke" }
  }
}

export async function executeRelayCommand(
  parsed: ParsedShellCommand,
  deps: ShellRemoteCommandDeps,
): Promise<ShellCommandResult> {
  const [action] = parsed.args
  if (action && action !== "status") {
    return { ok: false, message: "usage: relay status" }
  }
  const response = await deps.client.send(relayStatusRequest())
  const status = expectVariant<{ status: RelayStatus }>(response, "RelayStatus").status
  return { ok: true, message: formatRelayStatus(status), data: { status } }
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`unexpected response variant: expected ${variant}`)
  }
  return response[variant] as T
}
