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
    pub(super) fn settle_outside_jobs(&self, terminal: &mut Terminal) -> Result<(), DaemonError> {
        if let Some(pending) = terminal.pending_outside.take() {
            pending.wait()?;
            terminal.outside_ends.remove(0);
        }
        let store = self
            .inner
            .outside_events
            .read()
            .expect("outside terminal events poisoned")
            .clone();
        if let Some(store) = store {
            for end in &terminal.outside_ends {
                if let Some(pending) =
                    store.enqueue_outside_publish_job(completion(terminal, end))?
                {
                    pending.wait()?;
                }
            }
        }
        terminal.outside_ends.clear();
        Ok(())
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
        if let Some(result) = terminal
            .pending_outside
            .as_ref()
            .and_then(|pending| pending.try_result())
        {
            terminal.pending_outside = None;
            match result {
                Ok(()) => {
                    terminal.outside_ends.remove(0);
                }
                Err(error) => crate::logging::error_with_fields(
                    "room_terminal.events",
                    "outside terminal completion could not be committed; it will be retried",
                    serde_json::json!({"error": error.to_string()}),
                ),
            }
        }
        if terminal.pending_outside.is_some() {
            return;
        }
        let Some(end) = terminal.outside_ends.first() else {
            return;
        };
        match store.enqueue_outside_publish_job(completion(terminal, end)) {
            Ok(Some(pending)) => terminal.pending_outside = Some(pending),
            Ok(None) => {
                terminal.outside_ends.remove(0);
            }
            Err(error) => crate::logging::error_with_fields(
                "room_terminal.events",
                "outside terminal completion could not be queued; it will be retried",
                serde_json::json!({"error": error.to_string()}),
            ),
        }
    }
}

fn completion(
    terminal: &Terminal,
    end: &super::super::terminal::ProgramEnd,
) -> crate::durable_state::agent_lifecycle::outside::TerminalCompletion {
    crate::durable_state::agent_lifecycle::outside::TerminalCompletion {
        session: terminal.record.session_id.clone(),
        agent: match &terminal.record.opened_by {
            TerminalActor::Agent(id) => Some(id.clone()),
            _ => None,
        },
        terminal: terminal.record.terminal_id.clone(),
        command: end.command,
        offset: end.output_offset,
        exit_code: end.exit_code,
    }
}
