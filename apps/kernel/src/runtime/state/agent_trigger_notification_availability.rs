//! Whether the source of an agent's notification trigger can reach this kernel. The trigger
//! carries the answer so every client shows the same state without asking the inventory.
use super::workflow_notification_router::source_available;
use super::*;
use crate::session::{AgentTriggerNotificationSource, AgentTriggerSource};

impl KernelRuntimeState {
    /// Recomputes `source_available` of every notification trigger from the notification
    /// inventory and publishes each room whose triggers changed. Call it after the inventory
    /// changes.
    pub(crate) fn sync_agent_notification_trigger_availability(&self) -> Result<(), DaemonError> {
        let rooms = self.owned.session_store.read().list_sessions();
        for session in rooms {
            let availability = self.agent_notification_trigger_availability(&session)?;
            if availability.is_empty() {
                continue;
            }
            let changed = self
                .owned
                .session_store
                .write()
                .set_notification_trigger_availability(session.id(), &availability)?;
            if changed {
                self.owned.persist_agent_trigger_session(
                    session.id(),
                    "agent_trigger_source_availability",
                )?;
            }
        }
        Ok(())
    }

    fn agent_notification_trigger_availability(
        &self,
        session: &crate::session::RuntimeSession,
    ) -> Result<Vec<(String, bool)>, DaemonError> {
        let triggers: Vec<_> = session
            .agent_triggers()
            .iter()
            .filter_map(|trigger| match trigger.source() {
                AgentTriggerSource::Notification {
                    source: AgentTriggerNotificationSource::Workflow { source_id },
                    ..
                } => Some((trigger.id().to_owned(), source_id.clone())),
                _ => None,
            })
            .collect();
        if triggers.is_empty() {
            return Ok(Vec::new());
        }
        let owner = session.owner_user_id();
        let (local, _, _) = self
            .owned
            .durable_state_store
            .notification_inventory(owner)?;
        let cached = self
            .owned
            .durable_state_store
            .notification_cached_sources(owner)?;
        let sessions = self.owned.session_store.read();
        Ok(triggers
            .into_iter()
            .map(|(trigger_id, source_id)| {
                let available = local.iter().any(|s| {
                    s.source_id == source_id && s.enabled && source_available(&sessions, s)
                }) || cached
                    .iter()
                    .any(|s| s.source_id == source_id && s.available);
                (trigger_id, available)
            })
            .collect())
    }
}
