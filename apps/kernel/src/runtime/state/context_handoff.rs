use std::collections::BTreeMap;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard};

use crate::provider::RuntimeProviderRun;

mod builder;
use builder::build_agent_context_handoff_from_history;

#[derive(Debug, Clone)]
pub(super) struct PendingAgentContextHandoff {
    pub(super) source_provider_run_id: String,
    pub(super) source_provider: String,
    pub(super) source_model: String,
    pub(super) target_provider_run_id: String,
    pub(super) target_provider: String,
    pub(super) target_model: String,
    pub(super) context: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PendingAgentContextHandoffStore {
    inner: Arc<StdMutex<BTreeMap<String, PendingAgentContextHandoff>>>,
}

impl PendingAgentContextHandoffStore {
    fn write(&self) -> StdMutexGuard<'_, BTreeMap<String, PendingAgentContextHandoff>> {
        self.inner
            .lock()
            .expect("pending agent context handoff mutex poisoned")
    }

    pub(super) fn set(
        &self,
        session_id: &str,
        agent_id: &str,
        handoff: PendingAgentContextHandoff,
    ) {
        self.write()
            .insert(pending_handoff_key(session_id, agent_id), handoff);
    }

    pub(super) fn consume(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<PendingAgentContextHandoff> {
        self.write()
            .remove(&pending_handoff_key(session_id, agent_id))
    }
}

pub(super) fn inject_context_handoff(prompt: &str, handoff: &PendingAgentContextHandoff) -> String {
    format!(
        "{}\n\nProvider switch: {} ({}) from run {} -> {} ({}) run {}.\n\n<user_request>\n{}\n</user_request>",
        handoff.context.trim(),
        handoff.source_provider,
        model_label(&handoff.source_model),
        handoff.source_provider_run_id,
        handoff.target_provider,
        model_label(&handoff.target_model),
        handoff.target_provider_run_id,
        prompt
    )
}

impl super::KernelRuntimeOwnedState {
    pub(super) fn prepare_provider_switch_context_handoff(
        &self,
        source_run: &RuntimeProviderRun,
        target_run: &RuntimeProviderRun,
    ) {
        let Some(agent_id) = target_run.agent_instance_id() else {
            return;
        };
        if source_run.agent_instance_id() != Some(agent_id) {
            return;
        }
        if source_run.provider() == target_run.provider()
            && source_run.model() == target_run.model()
        {
            return;
        }
        match build_agent_context_handoff_from_history(
            &self.operational_history_store,
            target_run.session_id(),
            agent_id,
        ) {
            Ok(Some(context)) => {
                self.pending_agent_context_handoffs.set(
                    target_run.session_id(),
                    agent_id,
                    PendingAgentContextHandoff {
                        source_provider_run_id: source_run.id().to_string(),
                        source_provider: source_run.provider().to_string(),
                        source_model: source_run.model().to_string(),
                        target_provider_run_id: target_run.id().to_string(),
                        target_provider: target_run.provider().to_string(),
                        target_model: target_run.model().to_string(),
                        context,
                    },
                );
            }
            Ok(None) => {}
            Err(error) => {
                crate::logging::warn_with_fields(
                    "daemon.provider_context_handoff",
                    "failed to prepare provider switch context handoff",
                    serde_json::json!({
                        "session_id": target_run.session_id(),
                        "agent_id": agent_id,
                        "source_provider_run_id": source_run.id(),
                        "target_provider_run_id": target_run.id(),
                        "error": error.to_string(),
                    }),
                );
            }
        }
    }

    pub(super) fn prompt_with_pending_context_handoff(
        &self,
        session_id: &str,
        agent_id: &str,
        _source_attachment_id: &str,
        prompt: &str,
    ) -> String {
        self.pending_agent_context_handoffs
            .consume(session_id, agent_id)
            .map(|handoff| inject_context_handoff(prompt, &handoff))
            .unwrap_or_else(|| prompt.to_string())
    }
}

fn pending_handoff_key(session_id: &str, agent_id: &str) -> String {
    format!("{session_id}\n{agent_id}")
}

fn model_label(model: &str) -> &str {
    if model.trim().is_empty() {
        "unknown model"
    } else {
        model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_handoff_is_consumed_once() {
        let store = PendingAgentContextHandoffStore::default();
        store.set(
            "session",
            "agent",
            PendingAgentContextHandoff {
                source_provider_run_id: "run-old".to_string(),
                source_provider: "opencode".to_string(),
                source_model: "model-old".to_string(),
                target_provider_run_id: "run-new".to_string(),
                target_provider: "codex".to_string(),
                target_model: "model-new".to_string(),
                context: "<arroba_context_handoff>prior context</arroba_context_handoff>"
                    .to_string(),
            },
        );

        let first = store.consume("session", "agent");
        let second = store.consume("session", "agent");

        assert!(first.is_some());
        assert!(second.is_none());
        let injected = inject_context_handoff("next request", &first.unwrap());
        assert!(injected.contains("prior context"));
        assert!(injected.contains("<user_request>\nnext request\n</user_request>"));
    }

    #[test]
    fn handoff_injection_is_source_agnostic() {
        let store = PendingAgentContextHandoffStore::default();
        store.set(
            "session",
            "agent",
            PendingAgentContextHandoff {
                source_provider_run_id: "run-old".to_string(),
                source_provider: "opencode".to_string(),
                source_model: "model-old".to_string(),
                target_provider_run_id: "run-new".to_string(),
                target_provider: "codex".to_string(),
                target_model: "model-new".to_string(),
                context: "<arroba_context_handoff>workflow context</arroba_context_handoff>"
                    .to_string(),
            },
        );

        let handoff = store.consume("session", "agent");

        assert!(handoff.is_some());
        let injected = inject_context_handoff("run workflow node", &handoff.unwrap());
        assert!(injected.contains("workflow context"));
        assert!(store.consume("session", "agent").is_none());
    }
}
