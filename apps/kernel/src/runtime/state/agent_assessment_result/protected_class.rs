//! MP-08/MP-11: kernel-owned reason class for protected task results.
//! Derived only from the parsed disposition and kernel-observed paste outcomes;
//! provider prose never contributes.
use crate::durable_state::agent_lifecycle::goals::Assessment;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

const WITHHELD: &str = "[protected completion details withheld]";

/// Last protected-input outcome the kernel itself observed for an agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProtectedFact {
    Submitted,
    ApprovalDenied,
    ApprovalTimedOut,
    FieldNotFound,
}

#[derive(Clone, Default)]
pub(in crate::runtime::state) struct ProtectedFacts(
    Arc<Mutex<HashMap<(String, String), ProtectedFact>>>,
);

impl ProtectedFacts {
    pub(in crate::runtime::state) fn record(&self, room: &str, agent: &str, fact: ProtectedFact) {
        let mut facts = self.0.lock().unwrap_or_else(|e| e.into_inner());
        facts.insert((room.to_string(), agent.to_string()), fact);
    }
    pub(super) fn peek(&self, room: &str, agent: &str) -> Option<ProtectedFact> {
        let facts = self.0.lock().unwrap_or_else(|e| e.into_inner());
        facts.get(&(room.to_string(), agent.to_string())).copied()
    }
    pub(in crate::runtime::state) fn clear(&self, room: &str, agent: &str) {
        let mut facts = self.0.lock().unwrap_or_else(|e| e.into_inner());
        facts.remove(&(room.to_string(), agent.to_string()));
    }
}

pub(super) fn is_terminal(value: &Assessment) -> bool {
    matches!(value, Assessment::Achieved { .. } | Assessment::Blocked { .. })
}

/// Fixed kernel-authored text; `None` fact means no protected input was observed.
pub(super) fn public_text(value: &Assessment, fact: Option<ProtectedFact>) -> &'static str {
    use ProtectedFact::*;
    match (value, fact) {
        (Assessment::Achieved { .. }, None | Some(Submitted)) => "[protected task: completed]",
        (Assessment::Achieved { .. }, _) => WITHHELD,
        (_, Some(ApprovalDenied)) => "[protected task: approval denied]",
        (_, Some(ApprovalTimedOut)) => "[protected task: timed out]",
        (_, Some(FieldNotFound)) => "[protected task: protected field not found]",
        (Assessment::Waiting { .. }, _) => "[protected task: waiting]",
        _ => "[protected task: agent did not complete the protected step]",
    }
}
