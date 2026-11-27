/** Owner connections acquire and attach machines through the kernel from this version. */
export const machineAllocationMinimumProtocolVersion = 1006

/** Optional connected service data. An absent machine_allocation hides hosted actions. */
export type ControlPlaneRequest =
  | { action: "capabilities" }
  | { action: "acquire"; capability_revision: string; request_id: string; class_id?: string; region?: string; timeout_s?: number }
  | { action: "acquire_job"; capability_revision: string; request_id: string; job_ref: string; needs: { memory_gib?: number; cpus?: number }; timeout_s: number }
  | { action: "get_job" | "cancel_job"; job_ref: string }
  | { action: "job_started"; allocation_ref: string; capability_revision: string }
  | { action: "list" }
  | { action: "limits" }
  // The person's approval of a raise is attached by the kernel, never sent by a client.
  | { action: "set_limits"; expected_version: number; machines_at_once: number; spend_per_day_micros: number }
  | { action: "get"; allocation_ref: string }
  | { action: "release" | "start" | "stop"; allocation_ref: string; capability_revision: string; request_id: string; expected_revision: number }
  | { action: "start_saved_state" | "delete_saved_state"; allocation_ref: string; saved_state_ref: string; capability_revision: string; request_id: string; expected_revision: number }

export type ControlPlaneResponse =
  | { kind: "job_cancelled"; job_ref: string }
  | { kind: "capabilities"; machine_allocation: null | { revision: string; actions: string[]; offer: MachineAcquisitionOffer | null; offers?: MachineOption[]; job_offers: JobMachineOffer[]; job_default_class_id: string | null } }
  | { kind: "limits"; machines_at_once: number; spend_per_day_micros: number; currency: string; version: number; machines_in_use?: number; spend_today_micros?: number }
  | { kind: "machines"; machines: ControlPlaneMachineRow[]; saved_states: ControlPlaneSavedStateRow[] }
  | { kind: "machine"; revision: number; available_actions: string[]; allocation_ref: string; state: string; access_state: string; access_reason: string | null; purpose?: string; machine_label?: string; address?: string | null; kernel_diagnostics?: MachineKernelDiagnostics; kernel_target: null | {
      kernel_id: string; machine_id: string; relay_realm_id: string; key_thumbprint: string
    } }

/**
 * What `MachineAcquisition { action: "limits" }` answers (`result.limits`): the account's limits, which
 * the connected service holds (null when it has none), and the person's own limits for acquiring without
 * asking with what applies once the account's bound them.
 */
export type MachineLimitsReading = {
  account: { machines_at_once: number; machines_in_use?: number; spend_per_day_micros: number; spend_today_micros?: number; currency: string; version: number } | null
  no_ask: {
    enabled: boolean
    machines_at_once: number
    spend_per_day_micros: number | null
    classes: string[] | null
    effective_machines_at_once: number
    effective_spend_per_day_micros: number | null
    clamped: boolean
  }
}

export type MachineKernelDiagnostics = {
  relay_error?: string
  kernel_version: string | null
  kernel_protocol: number | null
  last_relay_refusal: {
    reason: string
    next_step: string
    since_ms: number
    next_retry_at_ms: number | null
  } | null
}

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
  rate_per_minute?: { amount_micros: number; currency: string }
  machine_type: string; vcpu: number; ram_gb: number; disk_gb: number; location: string; terms: { amount: string; currency: string; period: "hour" | "month" }
}

/** One machine the connected service offers: size, rate in the account's currency, region and stock now. */
export type MachineOption = {
  class_id: string; region: string; vcpu: number; ram_gb: number; disk_gb: number
  rate_per_minute: { amount_micros: number; currency: string }; available: boolean; note?: string
}

export type JobMachineOffer = { class_id: string; vcpu: number; usable_memory_mib: number; disk_gib: number; rate_per_minute: { amount_micros: number; currency: string } }
