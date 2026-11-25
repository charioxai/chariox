use super::*;
use crate::agent_release_notes::NOTE;

impl KernelRuntimeOwnedState {
    pub(super) fn context_with_agent_release_note(
        &self,
        agent_id: &str,
        prompt_id: &str,
        origin: crate::session::PromptOrigin,
        steering: bool,
        context: String,
    ) -> String {
        if steering || origin == crate::session::PromptOrigin::CompletionCheck {
            return context;
        }
        let Some(delivery) = self.durable_state_store.agent_release_delivery() else {
            return context;
        };
        let Ok(agent) = self.agent_store.get_agent(agent_id) else {
            return context;
        };
        if delivery.prepare_turn(agent_id, agent.created_at_ms(), prompt_id) {
            let note = crate::prompt_assembly::prompt_component("agent-release-note", NOTE);
            super::local_prompt_dispatch_runtime::join_hidden_context(&context, &note)
        } else {
            context
        }
    }

    pub(super) fn acknowledge_agent_release_note(&self, agent_id: &str, prompt_id: &str) {
        let store = &self.durable_state_store;
        if let Some(delivery) = store.agent_release_delivery() {
            if let Err(error) = delivery.acknowledge_turn(store, agent_id, prompt_id) {
                log_receipt_error(&error);
            }
        }
    }

    pub(super) fn announce_agent_release_to_client(
        &self,
        attachment: &crate::attachment::RuntimeAttachment,
    ) {
        let store = &self.durable_state_store;
        let Some(delivery) = store.agent_release_delivery() else {
            return;
        };
        let session_id = attachment.session_id();
        let Ok(session) = self.session_store.get_session(session_id) else {
            return;
        };
        match delivery.claim_client(
            store,
            session_id,
            session.created_at_ms(),
            attachment.client_id(),
        ) {
            Ok(true) => {
                self.record_notice(session_id, None, vec![attachment.id().into()], NOTE.trim())
            }
            Ok(false) => {}
            Err(error) => log_receipt_error(&error),
        }
    }
}

fn log_receipt_error(error: &DaemonError) {
    crate::logging::warn_with_fields(
        "daemon.agent_release_notes",
        "could not save an agent release note receipt",
        serde_json::json!({"error": error.to_string()}),
    );
}
