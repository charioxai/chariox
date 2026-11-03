// MP-02 / MP-08 / MP-10 / MP-11: retain the connected owner when its peer directory excludes itself.
import type { WaitingRoomInventory } from "./waiting-room-inventory-api.js"

export function waitingRoomInventoryWithDirectTarget(snapshot: WaitingRoomInventory, directTargetKernelId: string | null): WaitingRoomInventory {
  if (!directTargetKernelId || snapshot.kernelId !== directTargetKernelId) return snapshot
  const machine = snapshot.remoteMachines.find(row => row.machine_id === snapshot.machineId)
  const kernel = snapshot.remoteKernels.find(row => row.kernel_id === snapshot.kernelId)
  return {
    ...snapshot,
    remoteMachines: [...snapshot.remoteMachines.filter(row => row.machine_id !== snapshot.machineId), {
      machine_id: snapshot.machineId,
      display_name: snapshot.machineAlias ?? "Unnamed machine",
      machine_alias: snapshot.machineAlias ?? null,
      trust_status: "pending",
      pending: true,
      ...machine,
      online: true,
      kernel_count: Math.max(1, machine?.kernel_count ?? 0),
    }],
    remoteKernels: [...snapshot.remoteKernels.filter(row => row.kernel_id !== snapshot.kernelId), {
      kernel_id: snapshot.kernelId,
      machine_id: snapshot.machineId,
      kernel_alias: snapshot.kernelAlias ?? null,
      machine_alias: snapshot.machineAlias ?? null,
      capabilities: ["kernel_ws"],
      local_session_count: snapshot.sessions.length,
      // A read connection proves identity and presence, never lease admission.
      accepting_remote_leases: false,
      ...kernel,
    }],
  }
}
