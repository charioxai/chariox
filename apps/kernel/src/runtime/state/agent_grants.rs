//! The agents a person approved for one kind of action with an "Allow for ..." choice: terminal
//! input ("Allow for this session") or kernel-run watched commands ("Allow for this agent").
//!
//! The grant is the kernel's memory for the agent, not for one provider process, so a restart of
//! the provider does not end it. It ends when the agent's permissions change, its session ends, or
//! a parent revokes it.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(crate) struct AgentGrants {
    agents: Arc<Mutex<BTreeSet<String>>>,
}

impl AgentGrants {
    pub(crate) fn allow(&self, agent_id: &str) {
        self.agents
            .lock()
            .expect("agent grants lock")
            .insert(agent_id.to_string());
    }

    pub(crate) fn is_allowed(&self, agent_id: &str) -> bool {
        self.agents
            .lock()
            .expect("agent grants lock")
            .contains(agent_id)
    }

    /// Whether the agent held the grant.
    pub(crate) fn forget(&self, agent_id: &str) -> bool {
        self.agents
            .lock()
            .expect("agent grants lock")
            .remove(agent_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grant_is_per_agent_and_forgotten_on_request() {
        let grants = AgentGrants::default();
        grants.allow("a");
        assert!(grants.is_allowed("a"));
        assert!(!grants.is_allowed("b"));
        grants.forget("a");
        assert!(!grants.is_allowed("a"));
    }
}
