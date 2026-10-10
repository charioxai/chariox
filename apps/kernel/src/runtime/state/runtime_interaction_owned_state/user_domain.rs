//! Unattached kernel decisions in the same PendingInteraction authority and
//! resolution lock as Room decisions. Empty routing scope is not a Session.
use super::*;
use crate::session::{RuntimeInteraction, RuntimeInteractionKind};

impl KernelRuntimeOwnedState {
    /// Called under the shared interaction mutation/admission guards.
    pub(super) fn register_user_domain_interaction(
        &self,
        interaction: RuntimeInteraction,
        responder: tokio::sync::oneshot::Sender<super::super::PendingInteractionResolution>,
        owner: Option<&str>,
    ) -> Result<(), DaemonError> {
        let owner =
            owner.ok_or_else(|| interaction_error("Unattached decisions require an owner"))?;
        if interaction.agent_id().is_some()
            || interaction.kernel_operation_id().is_none()
            || interaction.kind() != RuntimeInteractionKind::Permission
            || interaction.default_on_timeout().is_some()
            || interaction.custom_choice().is_some()
            || !interaction
                .timeout_sec()
                .is_some_and(|seconds| (1..=3600).contains(&seconds))
        {
            return Err(interaction_error("Invalid unattached kernel decision"));
        }
        let mut pending = self.pending_interactions.write();
        if pending.contains_key(interaction.id())
            || pending.values().any(|p| {
                p.belongs_to(&self.session_store)
                    && p.user_domain_interaction
                        .as_ref()
                        .is_some_and(|existing| existing.subject() == interaction.subject())
            })
        {
            return Err(interaction_error(INTERACTION_ALREADY_PENDING));
        }
        if pending
            .values()
            .filter(|p| p.belongs_to(&self.session_store) && p.kernel_operation_owner.is_some())
            .count()
            >= 32
            || pending
                .values()
                .filter(|p| {
                    p.belongs_to(&self.session_store)
                        && p.kernel_operation_owner.as_deref() == Some(owner)
                })
                .count()
                >= 8
        {
            return Err(interaction_error(KERNEL_DECISION_LIMIT));
        }
        let popup = super::super::passkey_prompts::passkey_prompt_for_scope(
            "",
            None,
            &interaction,
            crate::session::unix_epoch_ms(),
        )?;
        let id = interaction.id().to_owned();
        let timeout = interaction.timeout_sec().expect("validated timeout");
        pending.insert(
            id.clone(),
            super::super::PendingInteraction {
                session_id: String::new(),
                session_store_identity: self.session_store.weak_identity(),
                agent_lifetime: None,
                kernel_operation_owner: Some(owner.into()),
                terminal_credential_owner: None,
                kernel_operation_deadline: Some(
                    std::time::Instant::now() + std::time::Duration::from_secs(timeout),
                ),
                passkey_prompt: popup.clone(),
                kernel_wide_interaction: None,
                user_domain_interaction: Some(interaction),
                responder: std::sync::Arc::new(std::sync::Mutex::new(Some(responder))),
            },
        );
        drop(pending);
        if popup.is_some() {
            self.passkey_prompts.forget_answered("", &id);
            self.passkey_prompts.record_change();
        }
        self.app_control.user_views().record_change(owner);
        Ok(())
    }

    pub(in crate::runtime::state) fn user_domain_interactions(
        &self,
        owner: &str,
    ) -> Vec<RuntimeInteraction> {
        self.pending_interactions
            .write()
            .values()
            .filter(|p| {
                p.belongs_to(&self.session_store)
                    && p.kernel_operation_owner.as_deref() == Some(owner)
            })
            .filter(|p| {
                p.kernel_operation_deadline
                    .is_some_and(|d| std::time::Instant::now() < d)
                    && !p.nobody_waits()
            })
            .filter_map(|p| p.user_domain_interaction.clone())
            .collect()
    }

    /// The caller already holds the shared mutation lock and has checked
    /// owner, deadline, kernel identity and the passkey verifier.
    pub(super) fn resolve_user_domain_interaction_locked(
        &self,
        id: &str,
        interaction: &RuntimeInteraction,
        choice: &str,
        passkey_verified: bool,
        take_host: bool,
    ) -> Result<(), DaemonError> {
        if take_host
            && !interaction.kernel_operation_id().is_some_and(|subject| {
                subject
                    .strip_prefix("host_action:")
                    .is_some_and(|op| id == format!("app_host_{op}"))
            })
        {
            return Err(interaction_error("Not an owner-bound App host offer"));
        }
        if choice == "accept_host_action" && !take_host {
            return Err(interaction_error(
                "App host actions use the dedicated terminal acceptance path",
            ));
        }
        // Host offers intentionally advertise only decline. The owner-bound
        // dedicated take path is their sole acceptance authority, as in Rooms.
        let (choice_id, reply) = if take_host {
            (
                "accept_host_action".to_owned(),
                "accept_host_action".to_owned(),
            )
        } else {
            let choice = interaction
                .choice(choice)
                .ok_or_else(|| interaction_error("Unknown decision choice"))?;
            if choice.requires_passkey() && !passkey_verified {
                return Err(super::super::critical_approval_passkey::passkey_error(
                    super::super::critical_approval_passkey::PASSKEY_REQUIRED,
                    "approving this critical action needs your Chariox passkey",
                ));
            }
            (choice.id().to_owned(), choice.reply().to_owned())
        };
        let pending = self
            .pending_interactions
            .write()
            .remove(id)
            .ok_or_else(|| interaction_error("interaction is no longer pending"))?;
        self.finish_user_domain_interaction(
            &pending,
            id,
            super::super::PendingInteractionResolution {
                status: "answered",
                choice_id: Some(choice_id),
                reply: Some(reply),
            },
        );
        Ok(())
    }

    pub(super) fn finish_user_domain_interaction(
        &self,
        pending: &super::super::PendingInteraction,
        id: &str,
        resolution: super::super::PendingInteractionResolution,
    ) {
        if pending.passkey_prompt.is_some() {
            if resolution.status == "answered" {
                self.passkey_prompts.record_answered("", id);
            } else {
                self.passkey_prompts.record_change();
            }
        }
        if let Some(owner) = &pending.kernel_operation_owner {
            self.app_control.user_views().record_change(owner);
        }
        if let Some(sender) = pending
            .responder
            .lock()
            .expect("interaction responder")
            .take()
        {
            let _ = sender.send(resolution);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::DaemonApp, config::DaemonConfig, runtime::router::CommandRouter,
        session::RuntimeInteractionChoice,
    };

    fn fixture() -> KernelRuntimeState {
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        assert!(app.sessions().list_sessions().is_empty());
        CommandRouter::with_interactive_capacity(
            std::sync::Arc::new(tokio::sync::Mutex::new(app)),
            1,
        )
        .runtime_state()
    }
    fn interaction(id: &str) -> RuntimeInteraction {
        RuntimeInteraction::for_kernel_operation(
            id,
            format!("fixture:{id}"),
            "Decide",
            "Fixture",
            vec![RuntimeInteractionChoice::new("deny", "Deny", "deny", None)],
        )
    }
    #[tokio::test]
    async fn detached_decisions_share_owner_authority_limits_and_cleanup() {
        let state = fixture();
        let mut receivers = Vec::new();
        for number in 0..8 {
            receivers.push(
                state
                    .create_kernel_operation_interaction(
                        "",
                        "alice",
                        interaction(&format!("detached-{number}")),
                    )
                    .await
                    .unwrap(),
            );
        }
        assert_eq!(state.owned.user_domain_interactions("alice").len(), 8);
        assert!(state.owned.user_domain_interactions("bob").is_empty());
        assert!(state
            .create_kernel_operation_interaction("", "alice", interaction("over-limit"))
            .await
            .is_err());
        assert!(state
            .resolve_terminal_runtime_interaction("", "detached-0", "deny", None, Some("bob"))
            .await
            .is_err());
        assert!(state
            .resolve_terminal_runtime_interaction(
                "wrong-session",
                "detached-0",
                "deny",
                None,
                Some("alice")
            )
            .await
            .is_err());
        assert!(state
            .resolve_terminal_runtime_interaction("", "detached-0", "unknown", None, Some("alice"))
            .await
            .is_err());
        state
            .resolve_terminal_runtime_interaction("", "detached-0", "deny", None, Some("alice"))
            .await
            .unwrap();
        let resolved = receivers.remove(0).await.unwrap();
        assert_eq!(resolved.choice_id.as_deref(), Some("deny"));
        assert!(state
            .resolve_terminal_runtime_interaction("", "detached-0", "deny", None, Some("alice"))
            .await
            .is_err());
        // Expiry uses the same shared sweep and consumes no default approval.
        state
            .owned
            .pending_interactions
            .write()
            .get_mut("detached-1")
            .unwrap()
            .kernel_operation_deadline = Some(std::time::Instant::now());
        state.owned.sweep_kernel_operation_interactions(false);
        assert_eq!(receivers.remove(0).await.unwrap().status, "timed_out");
        // Abandoned receivers release slots; shutdown clears the remainder.
        drop(receivers.remove(0));
        state.owned.sweep_kernel_operation_interactions(false);
        assert_eq!(state.owned.user_domain_interactions("alice").len(), 5);
        state.owned.sweep_kernel_operation_interactions(true);
        for receiver in receivers {
            assert_eq!(receiver.await.unwrap().status, "timed_out");
        }
        assert!(state.owned.user_domain_interactions("alice").is_empty());
        assert!(state.owned.session_store.list_sessions().is_empty());
    }
    #[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
    #[tokio::test]
    async fn native_user_views_route_decisions_outside_an_existing_room() {
        let state = fixture();
        let mut room = crate::session::RuntimeSession::new(
            "unrelated-room",
            None,
            "workspace",
            "worktree",
            "machine",
            "kernel",
        );
        room.set_owner_user_id("alice");
        state.owned.session_store.write().restore_session(room);
        assert_eq!(
            state.validation_session("alice").as_deref(),
            Some("unrelated-room")
        );
        let binding = crate::runtime::app_views::AppViewBinding {
            owner: "alice".into(),
            installation: "todo".into(),
            generation: 1,
            logical_tab: None,
            panel: Default::default(),
        };
        let view = state
            .app_control()
            .user_views()
            .open("alice", binding, "a")
            .unwrap();
        assert_eq!(state.validation_session("alice").as_deref(), Some(""));
        assert_eq!(state.validation_session("bob"), None);
        state
            .app_control()
            .user_views()
            .close("alice", &view.view_id);
        assert_eq!(
            state.validation_session("alice").as_deref(),
            Some("unrelated-room")
        );
    }
}
