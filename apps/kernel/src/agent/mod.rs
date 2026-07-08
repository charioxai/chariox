mod service;
mod service_store;
mod store;
mod types;

pub use service::AgentService;
pub use service_store::AgentServiceStore;
pub use store::AgentStore;
pub use types::{
    AgentInstance, AgentOperatingMode, AgentRole, AgentState, AgentSubstituteProfile,
    AgentSubstitutionRecord, CreateAgentRequest, GitWorktreePlacement, GridPosition,
    RemoteAgentBinding, calculate_agent_layout, generate_agent_ref, recalculate_positions,
};
