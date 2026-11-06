//! The agents whose terminal input a person approved with "Allow for this session".
//!
//! The grant is the kernel's memory for the agent, not for one provider process, so a restart of
//! the provider does not end it. It ends when the agent's permissions change or its session ends.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(crate) struct TerminalInputGrants {
    agents: Arc<Mutex<BTreeSet<String>>>,
}

impl TerminalInputGrants {
    pub(crate) fn allow(&self, agent_id: &str) {
        self.agents
            .lock()
            .expect("terminal input grants lock")
            .insert(agent_id.to_string());
    }

    pub(crate) fn is_allowed(&self, agent_id: &str) -> bool {
        self.agents
            .lock()
            .expect("terminal input grants lock")
            .contains(agent_id)
    }

    pub(crate) fn forget(&self, agent_id: &str) {
        self.agents
            .lock()
            .expect("terminal input grants lock")
            .remove(agent_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grant_is_per_agent_and_forgotten_on_request() {
        let grants = TerminalInputGrants::default();
        grants.allow("a");
        assert!(grants.is_allowed("a"));
        assert!(!grants.is_allowed("b"));
        grants.forget("a");
        assert!(!grants.is_allowed("a"));
    }
}
