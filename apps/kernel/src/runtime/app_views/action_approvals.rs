//! MP-08/MP-10/MP-11: retain the existing Allow for session answer, never
//! an App grant or a new permission type. Revocation invalidates its authority.
use super::{AppViewBinding, AppViews};
use crate::runtime::app_view_authority::AppViewAuthority;
use std::collections::HashMap;

type ActionKey = (String, String, String, String);
pub(super) type ActionApprovals = HashMap<ActionKey, ActionApproval>;
#[derive(Clone)]
pub(super) struct ActionApproval {
    generation: u64,
    authority: AppViewAuthority,
}
fn key(agent: &str, binding: &AppViewBinding, action: &str) -> ActionKey {
    (
        agent.to_owned(),
        binding.owner.clone(),
        binding.installation.clone(),
        action.to_owned(),
    )
}
impl AppViews {
    pub(crate) fn action_approved(
        &self,
        session: &str,
        agent: &str,
        binding: &AppViewBinding,
        action: &str,
    ) -> bool {
        // Release the view lock before checking the agent store's authority.
        let approved = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(session)
            .and_then(|views| views.action_approvals.get(&key(agent, binding, action)))
            .filter(|approval| approval.generation == binding.generation)
            .cloned();
        approved.is_some_and(|approval| approval.authority.check().is_ok())
    }
    pub(crate) fn approve_action(
        &self,
        session: &str,
        agent: &str,
        binding: &AppViewBinding,
        action: &str,
        authority: AppViewAuthority,
    ) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(session.to_owned())
            .or_default()
            .action_approvals
            .insert(
                key(agent, binding, action),
                ActionApproval {
                    generation: binding.generation,
                    authority,
                },
            );
    }
}
