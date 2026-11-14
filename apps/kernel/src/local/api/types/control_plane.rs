use super::*;

/// Optional operations supplied by the owner's authenticated connected service.
/// Descriptors confer no authority; the service validates each operation again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlPlaneRequest {
    Capabilities {},
    Acquire {
        capability_revision: String,
        request_id: String,
    },
    AcquireJob {
        capability_revision: String,
        request_id: String,
        job_ref: String,
        #[serde(default)]
        needs: super::JobNeeds,
        timeout_s: u64,
    },
    GetJob {
        job_ref: String,
    },
    CancelJob {
        job_ref: String,
    },
    JobStarted {
        allocation_ref: String,
        capability_revision: String,
    },
    List {},
    Get {
        allocation_ref: String,
    },
    Release {
        allocation_ref: String,
        capability_revision: String,
        request_id: String,
        expected_revision: u64,
    },
    Start {
        allocation_ref: String,
        capability_revision: String,
        request_id: String,
        expected_revision: u64,
    },
    Stop {
        allocation_ref: String,
        capability_revision: String,
        request_id: String,
        expected_revision: u64,
    },
    StartSavedState {
        allocation_ref: String,
        saved_state_ref: String,
        capability_revision: String,
        request_id: String,
        expected_revision: u64,
    },
    DeleteSavedState {
        allocation_ref: String,
        saved_state_ref: String,
        capability_revision: String,
        request_id: String,
        expected_revision: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlPlaneResponse {
    JobCancelled {
        job_ref: String,
    },
    Capabilities {
        machine_allocation: Option<MachineAllocationCapability>,
    },
    Machines {
        machines: Vec<ControlPlaneMachineRow>,
        saved_states: Vec<ControlPlaneSavedStateRow>,
    },
    Machine {
        revision: u64,
        available_actions: Vec<String>,
        allocation_ref: String,
        state: String,
        kernel_target: Option<Box<ControlPlaneMachineTarget>>,
        access_state: String,
        access_reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        purpose: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        machine_label: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        address: Option<String>,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineAllocationCapability {
    pub revision: String,
    pub actions: Vec<String>,
    pub offer: Option<MachineAcquisitionOffer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub job_offers: Vec<JobMachineOffer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_default_class_id: Option<String>,
}

/// Display facts supplied by the connected service for this capability revision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineAcquisitionOffer {
    pub machine_type: String,
    pub vcpu: u32,
    pub ram_gb: f64,
    pub disk_gb: u32,
    pub location: String,
    /// Amount, currency and period exactly as the connected service offers them; the kernel
    /// never computes amounts.
    pub terms: MachineAcquisitionTerms,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_per_minute: Option<JobMachineRate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineAcquisitionTerms {
    pub amount: String,
    pub currency: String,
    pub period: MachineAcquisitionPeriod,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineAcquisitionPeriod {
    Hour,
    Month,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlPlaneMachineTarget {
    pub kernel_id: String,
    pub machine_id: String,
    pub relay_realm_id: String,
    pub key_thumbprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence: Option<chariox_relay::protocol::RelayKernelPresence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlPlaneMachineRow {
    pub allocation_ref: String,
    pub name: String,
    pub state: String,
    pub revision: u64,
    pub since: String,
    pub kernel_id: Option<String>,
    pub machine_id: Option<String>,
    pub available_actions: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlPlaneSavedStateRow {
    pub saved_state_ref: String,
    pub allocation_ref: String,
    pub name: String,
    pub state: String,
    pub revision: u64,
    pub since: String,
    pub available_actions: Vec<String>,
}

/// Opaque machine offers and rates supplied by the connected service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobMachineOffer {
    pub class_id: String,
    pub vcpu: u32,
    pub usable_memory_mib: u64,
    pub disk_gib: u32,
    pub rate_per_minute: JobMachineRate,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobMachineRate {
    pub amount_micros: u64,
    pub currency: String,
}
