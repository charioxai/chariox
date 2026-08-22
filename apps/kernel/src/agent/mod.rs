mod service;
mod service_store;
mod store;
mod types;

pub use service::AgentService;
pub use service_store::AgentServiceStore;
pub(crate) use service_store::ProviderResumeClearOutcome;
pub use store::AgentStore;
pub use types::{
    calculate_agent_layout, generate_agent_ref, recalculate_positions, AgentInstance,
    AgentOperatingMode, AgentRole, AgentState, AgentSubstituteProfile, AgentSubstitutionRecord,
    CreateAgentRequest, GitWorktreePlacement, GridPosition, RemoteAgentBinding,
};
