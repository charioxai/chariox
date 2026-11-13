/** Owner connections acquire and attach machines through the kernel from this version. */
export const machineAllocationMinimumProtocolVersion = 1027

/** Optional connected service data. An absent machine_allocation hides hosted actions. */
export type ControlPlaneRequest =
  | { action: "capabilities" }
  | { action: "acquire"; capability_revision: string; request_id: string }
  | { action: "list" }
  | { action: "get"; allocation_ref: string }
  | { action: "release" | "start" | "stop"; allocation_ref: string; capability_revision: string; request_id: string; expected_revision: number }
  | { action: "start_saved_state" | "delete_saved_state"; allocation_ref: string; saved_state_ref: string; capability_revision: string; request_id: string; expected_revision: number }

export type ControlPlaneResponse =
  | { kind: "capabilities"; machine_allocation: null | { revision: string; actions: string[]; offer: MachineAcquisitionOffer | null } }
  | { kind: "machines"; machines: ControlPlaneMachineRow[]; saved_states: ControlPlaneSavedStateRow[] }
  | { kind: "machine"; revision: number; available_actions: string[]; allocation_ref: string; state: string; access_state: string; access_reason: string | null; kernel_target: null | {
      kernel_id: string; machine_id: string; relay_realm_id: string; key_thumbprint: string
    } }

export function controlPlaneRequest(request: ControlPlaneRequest) { return { ControlPlane: request } }

export type ControlPlaneMachineRow = {
  allocation_ref: string; name: string; state: string; revision: number; since: string;
  kernel_id: string | null; machine_id: string | null; available_actions: string[]
}
export type ControlPlaneSavedStateRow = {
  saved_state_ref: string; allocation_ref: string; name: string; state: string;
  revision: number; since: string; available_actions: string[]
}

/** Display facts supplied by the connected service for this capability revision. */
export type MachineAcquisitionOffer = {
  machine_type: string; vcpu: number; ram_gb: number; disk_gb: number; location: string; terms: { amount: string; currency: string; period: "hour" | "month" }
}
