//! MP-08 / MP-09 / MP-10 / MP-11: every completed Room job feeds outside parts.
use super::*;
impl RoomTerminalService {
    pub(crate) fn bind_outside_events(&self, store: crate::durable_state::DurableKernelStateStore) {
        *self
            .inner
            .outside_events
            .write()
            .expect("outside terminal events poisoned") = Some(store);
    }
    pub(super) fn publish_outside_jobs(&self, terminal: &mut Terminal) {
        let Some(store) = self
            .inner
            .outside_events
            .read()
            .expect("outside terminal events poisoned")
            .clone()
        else {
            terminal.outside_ends.clear();
            return;
        };
        let agent = match &terminal.record.opened_by {
            TerminalActor::Agent(agent_id) => Some(agent_id.as_str()),
            _ => None,
        };
        terminal.outside_ends.retain(|end| {
            let result = store.outside_publish_job(
                &terminal.record.session_id,
                agent,
                &terminal.record.terminal_id,
                end.command,
                end.output_offset,
                end.exit_code,
            );
            if result.is_err() {
                crate::logging::error_with_fields(
                    "room_terminal.events",
                    "outside terminal completion could not be committed; it will be retried",
                    serde_json::json!({}),
                );
            }
            result.is_err()
        });
    }
}
