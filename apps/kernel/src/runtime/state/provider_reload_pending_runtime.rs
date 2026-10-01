//! Deferred provider reload polling.

use super::*;

impl KernelRuntimeState {
    pub(super) fn remember_pending_provider_reload(
        &self,
        session_id: &str,
        agent_id: &str,
        reason: &str,
    ) {
        self.remember_pending_provider_reload_inner(session_id, agent_id, reason, false);
    }

    pub(super) fn remember_pending_provider_catalog_reload(
        &self,
        session_id: &str,
        agent_id: &str,
    ) {
        self.remember_pending_provider_reload_inner(
            session_id,
            agent_id,
            "runtime tool catalog",
            true,
        );
    }

    fn remember_pending_provider_reload_inner(
        &self,
        session_id: &str,
        agent_id: &str,
        reason: &str,
        force_catalog_reload: bool,
    ) {
        let mut pending = self.owned.pending_provider_reloads.write();
        if let Some(existing) = pending.get_mut(agent_id) {
            existing.force_catalog_reload |= force_catalog_reload;
            return;
        }
        pending.insert(
            agent_id.to_string(),
            PendingProviderReload {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
                reason: reason.to_string(),
                force_catalog_reload,
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
                        match state
                            .reload_agent_provider_if_idle_inner(
                                &pending.session_id,
                                &pending.agent_id,
                                &pending.reason,
                                pending.force_catalog_reload,
                            )
                            .await
                        {
                            Ok(ProviderReloadOutcome::Deferred) => {
                                state.remember_pending_provider_reload_inner(
                                    &pending.session_id,
                                    &pending.agent_id,
                                    &pending.reason,
                                    pending.force_catalog_reload,
                                );
                            }
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
