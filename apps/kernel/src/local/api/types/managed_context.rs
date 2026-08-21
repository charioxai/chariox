use super::*;

pub use crate::managed_context::outbound_service::{
    ManagedContextOutboundOperationPhase, ManagedContextOutboundOperationStatus,
    ManagedContextTransferTarget, ManagedContextTransferTicket,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartManagedContextTransferRequest {
    pub ticket: ManagedContextTransferTicket,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetManagedContextTransferStatusRequest {
    pub context_id: String,
}
