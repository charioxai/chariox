mod service;
mod store;
mod types;

pub use service::{AgentService, AgentServiceStore};
pub use store::AgentStore;
pub use types::{
    calculate_agent_layout, generate_agent_ref, recalculate_positions, AgentInstance, AgentState,
    CreateAgentRequest, GitWorktreePlacement, GridPosition, RemoteAgentBinding,
};
