//! Deferred provider reload polling.

use super::*;

impl KernelRuntimeState {
    /// A runtime tool catalog change keeps its typed cause in the same idle
    /// queue as launch-input changes, so a merged reload still forces it.
    pub(super) fn remember_pending_provider_catalog_reload(
        &self,
        session_id: &str,
        agent_id: &str,
    ) {
        self.remember_pending_provider_reload(
            session_id,
            agent_id,
            ProviderReloadReason::RuntimeToolCatalog,
        );
    }

    pub(super) fn remember_pending_provider_reload(
        &self,
        session_id: &str,
        agent_id: &str,
        reason: ProviderReloadReason,
    ) {
        let mut pending = self.owned.pending_provider_reloads.write();
        let authority = if pending
            .get(agent_id)
            .is_some_and(|previous| previous.authority.is_none())
        {
            None
        } else {
            self.prompt_command_authority.clone()
        };
        let reason = pending
            .get(agent_id)
            .map_or(reason.clone(), |previous| reason.merge(&previous.reason));
        pending.insert(
            agent_id.to_string(),
            PendingProviderReload {
                authority,
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
                reason,
            },
        );
        let poller = self.owned.pending_provider_reloads.pollers.claim(agent_id);
        drop(pending);
        let Some(mut poller) = poller else {
            return;
        };
        let state = self.clone();
        let session_id = session_id.to_string();
        let agent_id = agent_id.to_string();
        tokio::spawn(async move {
            while poller.next().await {
                let authority = state
                    .owned
                    .pending_provider_reloads
                    .write()
                    .get(&agent_id)
                    .and_then(|pending| pending.authority.clone());
                if let Some(authority) = authority {
                    if state
                        .with_prompt_command_authority(Some(authority.as_request()))
                        .authorize_current_prompt_command()
                        .is_err()
                    {
                        let mut queued = state.owned.pending_provider_reloads.write();
                        if queued
                            .get(&agent_id)
                            .and_then(|pending| pending.authority.as_ref())
                            .is_some_and(|current| current.grant_id == authority.grant_id)
                        {
                            queued.remove(&agent_id);
                        }
                        if !queued.contains_key(&agent_id) {
                            poller.release();
                            return;
                        }
                        continue;
                    }
                }
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
                        let authorized = state.with_prompt_command_authority(
                            pending
                                .authority
                                .as_ref()
                                .map(PromptCommandAuthority::as_request),
                        );
                        match authorized
                            .reload_agent_provider_if_idle_for_reason(
                                &pending.session_id,
                                &pending.agent_id,
                                &pending.reason,
                            )
                            .await
                        {
                            Ok(ProviderReloadOutcome::Deferred) => {
                                let mut queued = state.owned.pending_provider_reloads.write();
                                if let Some(newer) = queued.get_mut(&agent_id) {
                                    newer.reason = newer.reason.clone().merge(&pending.reason);
                                } else {
                                    queued.insert(agent_id.clone(), pending);
                                }
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
                }
                let queued = state.owned.pending_provider_reloads.write();
                if !queued.contains_key(&agent_id) {
                    poller.release();
                    return;
                }
            }
            // Leave the pending reason available for a later explicit change;
            // a Deferred attempt never starts a replacement polling budget.
            let queued = state.owned.pending_provider_reloads.write();
            poller.release();
            if queued.contains_key(&agent_id) {
                crate::logging::warn_with_fields(
                    "daemon.provider",
                    "pending provider reload polling expired",
                    serde_json::json!({"session_id": session_id, "agent_id": agent_id}),
                );
            }
        });
    }
}
