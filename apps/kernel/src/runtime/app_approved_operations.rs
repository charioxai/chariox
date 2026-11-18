//! The App tool calls a person approved. An agent's call acts with the
//! person's authority over routing only while its operation is listed here,
//! for exactly the tool call the person approved and no longer.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

#[derive(Clone, Default)]
pub(crate) struct ApprovedOperations(Arc<Mutex<HashSet<String>>>);

/// Approval of one operation; it ends when the call does.
pub(crate) struct Approval {
    operations: ApprovedOperations,
    operation_id: String,
}

impl ApprovedOperations {
    pub(crate) fn approve(&self, operation_id: &str) -> Approval {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(operation_id.to_owned());
        Approval {
            operations: self.clone(),
            operation_id: operation_id.to_owned(),
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    pub(crate) fn covers(&self, operation_id: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(operation_id)
    }
}

impl Drop for Approval {
    fn drop(&mut self) {
        self.operations
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.operation_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_approval_covers_its_operation_until_the_call_ends() {
        let operations = ApprovedOperations::default();
        assert!(!operations.covers("op-1"));
        let approval = operations.approve("op-1");
        assert!(operations.covers("op-1"));
        assert!(!operations.covers("op-2"));
        drop(approval);
        assert!(!operations.covers("op-1"));
    }
}
