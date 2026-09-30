mod failed_requests;
mod service;
mod service_store;
mod store;
mod types;

pub use failed_requests::FailedRequest;
pub(crate) use failed_requests::{failed_request_notice, failed_request_reason};
pub use service::AgentService;
pub(crate) use service::new_agent_focus_target;
pub use service_store::AgentServiceStore;
pub(crate) use service_store::ProviderResumeClearOutcome;
pub use store::AgentStore;
pub use types::{
    calculate_agent_layout, generate_agent_ref, recalculate_positions, AgentInstance,
    AgentOperatingMode, AgentRole, AgentState, AgentSubstituteProfile, AgentSubstitutionRecord,
    CreateAgentRequest, GitWorktreePlacement, GridPosition, RemoteAgentBinding,
};
