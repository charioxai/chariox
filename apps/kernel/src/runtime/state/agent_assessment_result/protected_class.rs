//! MP-08/MP-11: kernel-owned reason class for protected task results.
//! Derived only from the parsed disposition and kernel-observed paste outcomes;
//! provider prose never contributes.
use crate::durable_state::agent_lifecycle::goals::Assessment;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

const WITHHELD: &str = "[protected completion details withheld]";
const DENIED: &str = "[protected task: approval denied]";
const TIMED_OUT: &str = "[protected task: timed out]";

/// Last protected-input outcome the kernel itself observed for an agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProtectedFact {
    Submitted,
    ApprovalDenied,
    ApprovalTimedOut,
    FieldNotFound,
    /// A protected step failed for a reason the kernel does not classify.
    Failed,
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
    /// Unclassified failure: replaces an earlier success, keeps a specific failure.
    pub(in crate::runtime::state) fn record_failure(&self, room: &str, agent: &str) {
        let mut facts = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let slot = facts.entry((room.to_string(), agent.to_string()));
        slot.and_modify(|fact| {
            if *fact == ProtectedFact::Submitted {
                *fact = ProtectedFact::Failed;
            }
        })
        .or_insert(ProtectedFact::Failed);
    }
    pub(in crate::runtime::state) fn peek(&self, room: &str, agent: &str) -> Option<ProtectedFact> {
        let facts = self.0.lock().unwrap_or_else(|e| e.into_inner());
        facts.get(&(room.to_string(), agent.to_string())).copied()
    }
    pub(in crate::runtime::state) fn clear(&self, room: &str, agent: &str) {
        let mut facts = self.0.lock().unwrap_or_else(|e| e.into_inner());
        facts.remove(&(room.to_string(), agent.to_string()));
    }
}

pub(super) fn is_terminal(value: &Assessment) -> bool {
    matches!(
        value,
        Assessment::Achieved { .. } | Assessment::Blocked { .. }
    )
}

/// The class of a denial or timeout the kernel observed; it outlives an unusable completion check.
pub(super) fn known_block(fact: Option<ProtectedFact>) -> Option<&'static str> {
    match fact {
        Some(ProtectedFact::ApprovalDenied) => Some(DENIED),
        Some(ProtectedFact::ApprovalTimedOut) => Some(TIMED_OUT),
        _ => None,
    }
}

/// Fixed kernel-authored text; `None` fact means no protected input was observed.
pub(super) fn public_text(value: &Assessment, fact: Option<ProtectedFact>) -> &'static str {
    use ProtectedFact::*;
    match (value, fact) {
        (Assessment::Achieved { .. }, None | Some(Submitted)) => "[protected task: completed]",
        // The kernel's own denial or timeout outranks the agent's "achieved" claim.
        (_, Some(ApprovalDenied)) => DENIED,
        (_, Some(ApprovalTimedOut)) => TIMED_OUT,
        (Assessment::Achieved { .. }, _) => WITHHELD,
        (_, Some(FieldNotFound)) => "[protected task: protected field not found]",
        (Assessment::Waiting { .. }, _) => "[protected task: waiting]",
        _ => "[protected task: agent did not complete the protected step]",
    }
}
