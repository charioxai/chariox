/** Optional connected service data. An absent machine_allocation hides hosted actions. */
export type ControlPlaneRequest =
  | { action: "capabilities" }
  | { action: "acquire"; capability_revision: string; request_id: string }
  | { action: "acquire_job"; capability_revision: string; request_id: string; job_ref: string; needs: { memory_gib?: number; cpus?: number }; timeout_s: number }
  | { action: "get_job" | "cancel_job"; job_ref: string }
  | { action: "job_started"; allocation_ref: string; capability_revision: string }
  | { action: "list" }
  | { action: "get"; allocation_ref: string }
  | { action: "release" | "start" | "stop"; allocation_ref: string; capability_revision: string; request_id: string; expected_revision: number }
  | { action: "start_saved_state" | "delete_saved_state"; allocation_ref: string; saved_state_ref: string; capability_revision: string; request_id: string; expected_revision: number }

export type ControlPlaneResponse =
  | { kind: "job_cancelled"; job_ref: string }
  | { kind: "capabilities"; machine_allocation: null | { revision: string; actions: string[]; offer: MachineAcquisitionOffer | null; job_offers: JobMachineOffer[]; job_default_class_id: string | null } }
  | { kind: "machines"; machines: ControlPlaneMachineRow[]; saved_states: ControlPlaneSavedStateRow[] }
  | { kind: "machine"; revision: number; available_actions: string[]; allocation_ref: string; state: string; access_state: string; access_reason: string | null; purpose?: string; machine_label?: string; address?: string | null; kernel_target: null | {
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

export type JobMachineOffer = { class_id: string; vcpu: number; usable_memory_mib: number; disk_gib: number; rate_per_minute: { amount_micros: number; currency: string } }
