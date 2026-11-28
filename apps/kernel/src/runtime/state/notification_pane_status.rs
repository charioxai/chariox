//! The pane reads the subscription's canonical health; reachability alone is not a stopped subscription.
use super::*;
use crate::local::{NotificationSubscriptionHealth, WorkflowNotificationTargetKind};
use crate::session::AgentSubscriptionError;
use std::collections::BTreeMap;

impl KernelRuntimeState {
    pub(super) fn sync_agent_subscription_errors(&self) -> Result<(), DaemonError> {
        let rooms = self.owned.session_store.list_sessions();
        let mut by_owner = BTreeMap::new();
        for room in rooms {
            let owner = room.owner_user_id().to_owned();
            if !by_owner.contains_key(&owner) {
                by_owner.insert(
                    owner.clone(),
                    self.as_kernel().notification_subscription_rows(&owner)?,
                );
            }
            let mut errors: Vec<_> = by_owner[&owner]
                .iter()
                .filter(|row| {
                    row.subscription.session_id == room.id()
                        && row.subscription.target_kind == WorkflowNotificationTargetKind::Agent
                        && !matches!(
                            row.health,
                            NotificationSubscriptionHealth::Ok
                                | NotificationSubscriptionHealth::Paused
                        )
                })
                .map(|row| AgentSubscriptionError {
                    agent_id: row.subscription.endpoint_id.clone(),
                    subscription_id: row.subscription.subscription_id.clone(),
                    health: row.health,
                })
                .collect();
            errors.sort_by(|left, right| left.subscription_id.cmp(&right.subscription_id));
            let mut sessions = self.owned.session_store.write();
            let mut current = sessions.get_session(room.id())?;
            if current.set_notification_errors(errors) {
                sessions.restore_session(current);
                drop(sessions);
                self.owned
                    .persist_agent_trigger_session(room.id(), "notification_health")?;
                self.owned
                    .terminal_stream
                    .notify_terminal_projection_change(room.id());
            }
        }
        Ok(())
    }
}
