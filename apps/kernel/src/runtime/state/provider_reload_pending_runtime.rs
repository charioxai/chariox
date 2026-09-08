//! Deferred provider reload polling.

use super::*;

impl KernelRuntimeState {
    pub(super) fn remember_pending_provider_reload(
        &self,
        session_id: &str,
        agent_id: &str,
        reason: ProviderReloadReason,
    ) {
        let mut pending = self.owned.pending_provider_reloads.write();
        let reason = pending
            .get(agent_id)
            .map_or(reason.clone(), |previous| reason.merge(&previous.reason));
        pending.insert(
            agent_id.to_string(),
            PendingProviderReload {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
                reason,
            },
        );
        drop(pending);
        let state = self.clone();
        let session_id = session_id.to_string();
        let agent_id = agent_id.to_string();
        tokio::spawn(async move {
            for _ in 0..240 {
                let is_idle = state
                    .owned
                    .session_store
                    .get_session(&session_id)
                    .ok()
                    .is_some_and(|session| {
                        state
                            .owned
                            .prompt_state_owner
                            .active_prompt_for_agent(&session, &agent_id)
                            .is_none()
                    });
                if is_idle {
                    let pending = {
                        let mut pending = state.owned.pending_provider_reloads.write();
                        pending.remove(&agent_id)
                    };
                    if let Some(pending) = pending {
                        match state.reload_agent_provider_if_idle_for_reason(
                            &pending.session_id,
                            &pending.agent_id,
                            &pending.reason,
                        ) {
                            Ok(ProviderReloadOutcome::Deferred) => state
                                .remember_pending_provider_reload(
                                    &pending.session_id,
                                    &pending.agent_id,
                                    pending.reason,
                                ),
                            Ok(_) => {}
                            Err(error) => {
                                crate::logging::warn_with_fields(
                                    "daemon.provider",
                                    "pending provider reload failed",
                                    serde_json::json!({
                                        "session_id": pending.session_id,
                                        "agent_id": pending.agent_id,
                                        "error": error.to_string(),
                                    }),
                                );
                            }
                        }
                    }
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        });
    }
}
