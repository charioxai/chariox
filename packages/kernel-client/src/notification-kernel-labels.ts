/** MP-08 / MP-10: the machine alias a notification source's kernel is listed under. */
import type { RelayKernelPresence, RelayStatus, RemoteMachineRecord } from "./kernel-types.js"
import { relayStatusRequest } from "./ipc-relay-control-requests.js"
import { listRemoteMachineKernelsRequest, listRemoteMachinesRequest } from "./ipc-remote-connection-requests.js"
import { formatRemoteMachineLabel } from "./shell-remote-format.js"

type Client = { send(request: Record<string, unknown>): Promise<Record<string, unknown>> }

/**
 * Labels for the given kernel ids: this kernel's machine alias, then the alias of each approved
 * machine that hosts one of the others. A kernel the relay does not list keeps no label, so the
 * caller shows its id.
 */
export async function notificationKernelLabels(client: Client, kernelIds: ReadonlySet<string>): Promise<ReadonlyMap<string, string>> {
  const labels = new Map<string, string>()
  if (kernelIds.size === 0) return labels
  const own = (await client.send(relayStatusRequest())).RelayStatus as { status: RelayStatus } | undefined
  if (own && kernelIds.has(own.status.daemon_id)) labels.set(own.status.daemon_id, own.status.machine_alias ?? own.status.machine_id)
  if ([...kernelIds].every(id => labels.has(id))) return labels
  // The labels are display only: with no relay the other kernels show their ids.
  try {
    const machines = ((await client.send(listRemoteMachinesRequest())).RemoteMachinesListed as { machines: RemoteMachineRecord[] }).machines
    for (const machine of machines.filter(m => m.online && m.kernel_count > 0)) {
      if ([...kernelIds].every(id => labels.has(id))) break
      const listed = (await client.send(listRemoteMachineKernelsRequest(machine.machine_id))).RemoteMachineKernelsListed as { kernels: RelayKernelPresence[] }
      for (const kernel of listed.kernels) if (kernelIds.has(kernel.kernel_id)) labels.set(kernel.kernel_id, formatRemoteMachineLabel(machine))
    }
  } catch {
    return labels
  }
  return labels
}
