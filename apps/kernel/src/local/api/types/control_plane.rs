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
        kernel_target: Option<ControlPlaneMachineTarget>,
        access_state: String,
        access_reason: Option<String>,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineAllocationCapability {
    pub revision: String,
    pub actions: Vec<String>,
    pub offer: Option<MachineAcquisitionOffer>,
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
    /// Includes currency and billing interval; the kernel never calculates prices.
    pub price: MachineAcquisitionPrice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineAcquisitionPrice {
    pub amount: String,
    pub currency: String,
    pub period: MachineAcquisitionPricePeriod,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineAcquisitionPricePeriod {
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
