use super::*;

mod agent_lifetime;
mod kernel_wide;
mod maintenance;
mod registration;
#[cfg(test)]
mod regression_tests;
mod user_domain;

/// Refusals that clear once the pending decision is answered.
pub(crate) const INTERACTION_ALREADY_PENDING: &str =
    "Interaction subject or identity is already pending";
pub(crate) const KERNEL_DECISION_LIMIT: &str = "Kernel decision limit reached";

/// Whether registering an interaction was refused only until a pending one is
/// answered.
pub(crate) fn interaction_waits(error: &DaemonError) -> bool {
    matches!(
        error,
        DaemonError::LocalTransport { operation: "runtime interaction", message }
            if message == INTERACTION_ALREADY_PENDING || message == KERNEL_DECISION_LIMIT
    )
}

fn interaction_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "runtime interaction",
        message: message.into(),
    }
}

// Empty scope is the detached user domain. Attribute reference failures at
// their authoritative origin, including races after public owner admission;
// leave Room diagnostics and unrelated passkey/storage failures unchanged.
fn interaction_reference_error(
    session_id: &str,
    operation: &'static str,
    message: String,
) -> DaemonError {
    if session_id.is_empty() {
        DaemonError::UserDomainRefused {
            reason: crate::error::UserDomainRefusalReason::NotGranted,
        }
    } else {
        DaemonError::LocalTransport { operation, message }
    }
}

impl KernelRuntimeOwnedState {
    /// The owner and operation of a pending decision whose chosen choice needs
    /// the passkey, when the caller owns that decision and it has not expired
    /// (its popup is closed then, so no passkey is checked for it).
    pub(super) fn passkey_gate(
        &self,
        session_id: &str,
        interaction_id: &str,
        choice_id: &str,
        caller_user_id: Option<&str>,
    ) -> Option<(String, String)> {
        let pending = self
            .pending_interactions
            .write()
            .get(interaction_id)
            .filter(|pending| {
                pending.session_id == session_id && pending.belongs_to(&self.session_store)
            })
            .filter(|pending| {
                pending
                    .kernel_operation_deadline
                    .is_none_or(|deadline| std::time::Instant::now() < deadline)
            })
            .cloned()?;
        let owner = pending
            .kernel_operation_owner
            .clone()
            .filter(|owner| Some(owner.as_str()) == caller_user_id)?;
        let interaction = if let Some(interaction) = pending
            .user_domain_interaction
            .or(pending.kernel_wide_interaction)
        {
            interaction
        } else {
            self.session_store
                .get_session(session_id)
                .ok()?
                .active_interactions()
                .iter()
                .find(|interaction| interaction.id() == interaction_id)?
                .clone()
        };
        interaction
            .choice(choice_id)
            .filter(|choice| choice.requires_passkey())?;
        Some((owner, interaction.kernel_operation_id()?.to_owned()))
    }

    pub(super) fn update_provider_login_interaction(
        &self,
        session_id: &str,
        interaction_id: &str,
        login: crate::session::RuntimeProviderLogin,
    ) -> Result<(), DaemonError> {
        let activity_mutation = self.begin_managed_activity_mutation();
        let mut sessions = self.session_store.write();
        let mut session = sessions.get_session(session_id)?;
        let Some(interaction) = session.remove_active_interaction(interaction_id) else {
            return Ok(());
        };
        session.add_active_interaction(interaction.with_provider_login(login));
        sessions.restore_session(session);
        activity_mutation.record();
        drop(sessions);
        self.session_snapshot(session_id)?;
        self.terminal_stream
            .notify_terminal_projection_change(session_id);
        Ok(())
    }

    /// `passkey_verified`: the answer proved the owner's presence (see
    /// `critical_approval_passkey`); a choice that requires the passkey is
    /// refused without it.
    pub(super) fn resolve_runtime_interaction(
        &self,
        session_id: &str,
        interaction_id: &str,
        choice_id: &str,
        custom_reply: Option<&str>,
        caller_user_id: Option<&str>,
        passkey_verified: bool,
    ) -> Result<(), DaemonError> {
        self.resolve_runtime_interaction_inner(
            session_id,
            interaction_id,
            choice_id,
            custom_reply,
            caller_user_id,
            passkey_verified,
            false,
        )
    }

    /// The dedicated terminal take path competes with decline/expiry under
    /// the same interaction mutation lock. No generic reply can take an offer.
    pub(super) fn take_app_host_interaction(
        &self,
        session_id: &str,
        operation_id: &str,
        owner: &str,
    ) -> Result<(), DaemonError> {
        self.resolve_runtime_interaction_inner(
            session_id,
            &format!("app_host_{operation_id}"),
            "accept_host_action",
            None,
            Some(owner),
            false,
            true,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Preserve the existing resolve_runtime_interaction_inner operation signature and explicit context arguments"
    )]
    fn resolve_runtime_interaction_inner(
        &self,
        session_id: &str,
        interaction_id: &str,
        choice_id: &str,
        custom_reply: Option<&str>,
        caller_user_id: Option<&str>,
        passkey_verified: bool,
        take_host: bool,
    ) -> Result<(), DaemonError> {
        self.resolve_runtime_interaction_authorized(
            session_id,
            interaction_id,
            choice_id,
            custom_reply,
            caller_user_id,
            passkey_verified,
            None,
            None,
            take_host,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn resolve_runtime_interaction_authorized(
        &self,
        session_id: &str,
        interaction_id: &str,
        choice_id: &str,
        custom_reply: Option<&str>,
        caller_user_id: Option<&str>,
        passkey_verified: bool,
        sudo: Option<&crate::local::KernelSudoTurn>,
        authorizing_terminal: Option<&str>,
        take_host: bool,
    ) -> Result<(), DaemonError> {
        let _mutation = self
            .pending_interactions
            .mutation
            .lock()
            .map_err(|_| interaction_error("Interaction store is unavailable"))?;
        crate::logging::debug_with_fields(
            "runtime.interaction",
            "resolve runtime interaction requested",
            serde_json::json!({
                "session_id": session_id,
                "interaction_id": interaction_id,
                "choice_id": choice_id,
                "pending_store_ptr": format!("{:p}", std::sync::Arc::as_ptr(&self.pending_interactions.inner)),
                "pending_interaction_count_before": self.pending_interactions.write().len(),
            }),
        );
        let pending = {
            let pending = self.pending_interactions.write();
            pending.get(interaction_id).cloned().ok_or_else(|| {
                interaction_reference_error(
                    session_id,
                    "resolve runtime interaction",
                    format!("interaction {interaction_id} was not pending"),
                )
            })?
        };
        if sudo.is_some()
            && interaction_id.starts_with("capability-request-")
            && pending.kernel_operation_owner.is_some()
        {
            return Err(interaction_error(
                "sudo cannot authorize resource acquisition",
            ));
        }
        if sudo.is_some()
            && (pending.terminal_credential_owner.is_some()
                || pending.passkey_prompt.as_ref().is_some_and(|prompt| {
                    prompt.kind != crate::local::PasskeyPromptKind::CriticalApproval
                }))
        {
            return Err(interaction_error(
                "sudo cannot answer authority or credential prompts",
            ));
        }
        if pending.session_id != session_id || !pending.belongs_to(&self.session_store) {
            return Err(interaction_reference_error(
                session_id,
                "resolve runtime interaction",
                "interaction does not belong to the requested session".into(),
            ));
        }
        if !self.agent_interaction_is_live(&pending) {
            self.withdraw_agent_interaction_locked(interaction_id, &pending)?;
            return Err(interaction_error(
                "Agent interaction was withdrawn because its turn or agent ended",
            ));
        }
        if pending
            .kernel_operation_owner
            .as_deref()
            .or(pending.terminal_credential_owner.as_deref())
            .is_some_and(|owner| Some(owner) != caller_user_id)
        {
            return Err(interaction_reference_error(
                session_id,
                "runtime interaction",
                "Only the operation owner can answer this decision".into(),
            ));
        }
        if pending
            .kernel_operation_deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            return Err(interaction_error("Kernel operation decision expired"));
        }
        if let Some(interaction) = &pending.user_domain_interaction {
            if custom_reply.is_some() || sudo.is_some() || authorizing_terminal.is_some() {
                return Err(interaction_error(
                    "Invalid unattached decision reply authority",
                ));
            }
            return self.resolve_user_domain_interaction_locked(
                interaction_id,
                interaction,
                choice_id,
                passkey_verified,
                take_host,
            );
        }
        let activity_mutation = self.begin_managed_activity_mutation();
        let mut sessions = self.session_store.write();
        let mut session = if pending.kernel_wide_interaction.is_some() {
            None
        } else {
            Some(sessions.get_session(session_id)?.clone())
        };
        if session
            .as_ref()
            .is_some_and(|session| !self.agent_interaction_turn_is_live(&pending, session))
        {
            drop(sessions);
            drop(activity_mutation);
            self.withdraw_agent_interaction_locked(interaction_id, &pending)?;
            return Err(interaction_error(
                "Agent interaction was withdrawn because its turn ended",
            ));
        }
        let interaction = pending
            .kernel_wide_interaction
            .clone()
            .or_else(|| {
                session
                    .as_ref()?
                    .active_interactions()
                    .iter()
                    .find(|interaction| interaction.id() == interaction_id)
                    .cloned()
            })
            .ok_or_else(|| interaction_error("interaction is not active"))?;
        if !passkey_verified
            && interaction
                .choice(choice_id)
                .is_some_and(crate::session::RuntimeInteractionChoice::requires_passkey)
        {
            return Err(super::critical_approval_passkey::passkey_error(
                super::critical_approval_passkey::PASSKEY_REQUIRED,
                "approving this critical action needs your Chariox passkey",
            ));
        }
        if take_host
            && (pending.kernel_operation_owner.as_deref() != caller_user_id
                || !interaction.kernel_operation_id().is_some_and(|subject| {
                    subject
                        .strip_prefix("host_action:")
                        .is_some_and(|operation| interaction_id == format!("app_host_{operation}"))
                }))
        {
            return Err(interaction_error("Not an owner-bound App host offer"));
        }
        let resolved_reply = if take_host {
            "accept_host_action".to_owned()
        } else if let Some(choice) = interaction.choice(choice_id) {
            if let Some(reply) = custom_reply {
                if interaction
                    .kernel_operation_id()
                    .is_some_and(|id| id.starts_with("access-"))
                    && choice.requires_passkey()
                {
                    let minutes = reply
                        .parse::<u32>()
                        .map_err(|_| interaction_error("Access lifetime must be whole minutes"))?;
                    if minutes == 0
                        || minutes
                            > self
                                .config_projection
                                .snapshot()
                                .user_config
                                .kernel_access
                                .grant_max_minutes
                    {
                        return Err(interaction_error("Access lifetime exceeds kernel policy"));
                    }
                    reply.to_owned()
                } else {
                    let custom_choice =
                        interaction
                            .custom_choice()
                            .ok_or_else(|| {
                                DaemonError::LocalTransport {
                            operation: "resolve runtime interaction",
                            message:
                                "custom_reply is only valid for interactions with a custom choice"
                                    .to_string(),
                        }
                            })?;
                    validate_runtime_interaction_custom_reply(custom_choice, reply)?;
                    reply.to_string()
                }
            } else {
                choice.reply().to_string()
            }
        } else if let Some(custom_choice) = interaction.custom_choice() {
            if custom_choice.id() != choice_id {
                return Err(DaemonError::LocalTransport {
                    operation: "resolve runtime interaction",
                    message: format!(
                        "interaction {interaction_id} does not define choice {choice_id}"
                    ),
                });
            }
            if interaction.kind() != crate::session::RuntimeInteractionKind::Choice {
                return Err(DaemonError::LocalTransport {
                    operation: "resolve runtime interaction",
                    message: "custom choices are only valid for choice interactions".to_string(),
                });
            }
            let reply = custom_reply.ok_or_else(|| DaemonError::LocalTransport {
                operation: "resolve runtime interaction",
                message: "custom_reply is required for the custom choice".to_string(),
            })?;
            validate_runtime_interaction_custom_reply(custom_choice, reply)?;
            reply.to_string()
        } else {
            return Err(DaemonError::LocalTransport {
                operation: "resolve runtime interaction",
                message: format!("interaction {interaction_id} does not define choice {choice_id}"),
            });
        };
        // The same monotonic decision deadline applies after waiting for the
        // session writer and immediately before consuming the decision.
        if pending
            .kernel_operation_deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            return Err(interaction_error("Kernel operation decision expired"));
        }
        let consume = || {
            let mut access = (sudo.is_some() || authorizing_terminal.is_some())
                .then(|| self.sudo_turns.lock().expect("access state poisoned"));
            if let Some(terminal) = authorizing_terminal {
                let turn = access
                    .as_mut()
                    .and_then(|state| state.get_mut(interaction_id))
                    .ok_or_else(|| interaction_error("sudo request revoked before the decision"))?;
                turn.terminal_id = terminal.into();
            }
            if let Some(turn) = sudo {
                if access.as_ref().and_then(|state| state.get(&turn.entry_id)) != Some(turn) {
                    return Err(interaction_error(
                        "sudo turn was revoked before the decision",
                    ));
                }
                self.durable_state_store.append_event(
                    "kernel_access.sudo_approval",
                    Some(interaction_id.into()),
                    super::sudo_approval_receipt(turn, session_id, interaction_id, choice_id),
                )?;
            }
            self.pending_interactions
                .write()
                .remove(interaction_id)
                .ok_or_else(|| interaction_error("interaction is no longer pending"))
        };
        let pending = if let Some(turn) = sudo {
            let source = sessions.get_session(&turn.session_id)?;
            if source.status() == crate::session::SessionStatus::Ended {
                return Err(interaction_error("sudo session ended"));
            }
            self.prompt_state_owner.with_running_prompt(
                &source,
                &turn.agent_id,
                turn.prompt_id
                    .as_deref()
                    .ok_or_else(|| interaction_error("sudo turn not started"))?,
                &turn.entry_id,
                consume,
            )?
        } else {
            consume()?
        };
        if pending.passkey_prompt.is_some() {
            // Every terminal closes the popup; a later answer is told why.
            self.passkey_prompts
                .record_answered(session_id, interaction_id);
        }
        if let Some(mut session) = session.take() {
            session.remove_active_interaction(interaction_id);
            sessions.restore_session(session);
        }
        activity_mutation.record();
        drop(sessions);
        if pending.kernel_wide_interaction.is_none() {
            self.session_snapshot(session_id)?;
            self.terminal_stream
                .notify_terminal_projection_change(session_id);
        }
        if let Some(sender) = pending
            .responder
            .lock()
            .expect("pending interaction responder mutex poisoned")
            .take()
        {
            let _ = sender.send(super::PendingInteractionResolution {
                status: "answered",
                choice_id: Some(choice_id.to_string()),
                reply: Some(resolved_reply),
            });
        }
        crate::logging::debug_with_fields(
            "runtime.interaction",
            "resolved runtime interaction",
            serde_json::json!({
                "session_id": session_id,
                "interaction_id": interaction_id,
                "choice_id": choice_id,
                "pending_interaction_count_after": self.pending_interactions.write().len(),
            }),
        );
        Ok(())
    }

    pub(super) fn timeout_runtime_interaction(
        &self,
        session_id: &str,
        interaction_id: &str,
    ) -> Result<(), DaemonError> {
        self.timeout_runtime_interaction_if_current(session_id, interaction_id, None)
    }

    fn timeout_runtime_interaction_if_current(
        &self,
        session_id: &str,
        interaction_id: &str,
        expected: Option<&super::PendingInteraction>,
    ) -> Result<(), DaemonError> {
        let _mutation = self
            .pending_interactions
            .mutation
            .lock()
            .map_err(|_| interaction_error("Interaction store is unavailable"))?;
        crate::logging::debug_with_fields(
            "runtime.interaction",
            "timeout runtime interaction requested",
            serde_json::json!({
                "session_id": session_id,
                "interaction_id": interaction_id,
                "pending_interaction_count_before": self.pending_interactions.write().len(),
            }),
        );
        let pending = {
            let pending = self.pending_interactions.write();
            if !pending.get(interaction_id).is_some_and(|value| {
                value.session_id == session_id
                    && value.belongs_to(&self.session_store)
                    && expected.is_none_or(|expected| {
                        std::sync::Arc::ptr_eq(&value.responder, &expected.responder)
                    })
            }) {
                return Ok(());
            }
            pending
                .get(interaction_id)
                .expect("checked pending interaction")
                .clone()
        };
        if !self.agent_interaction_is_live(&pending) {
            return self.withdraw_agent_interaction_locked(interaction_id, &pending);
        }
        self.pending_interactions.write().remove(interaction_id);
        if pending.passkey_prompt.is_some() {
            self.passkey_prompts.record_change();
        }
        if let Some(interaction) = &pending.user_domain_interaction {
            self.finish_user_domain_interaction(
                &pending,
                interaction_id,
                timeout_runtime_interaction_resolution(interaction),
            );
            return Ok(());
        }
        if let Some(interaction) = pending.kernel_wide_interaction.as_ref() {
            if let Some(sender) = pending
                .responder
                .lock()
                .expect("interaction responder")
                .take()
            {
                let _ = sender.send(timeout_runtime_interaction_resolution(interaction));
            }
            return Ok(());
        }
        let activity_mutation = self.begin_managed_activity_mutation();
        let mut sessions = self.session_store.write();
        let mut session = sessions.get_session(session_id)?.clone();
        if !self.agent_interaction_turn_is_live(&pending, &session) {
            drop(sessions);
            drop(activity_mutation);
            return self.withdraw_agent_interaction_locked(interaction_id, &pending);
        }
        let Some(interaction) = session.remove_active_interaction(interaction_id) else {
            return Ok(());
        };
        sessions.restore_session(session);
        activity_mutation.record();
        drop(sessions);
        self.session_snapshot(session_id)?;
        self.terminal_stream
            .notify_terminal_projection_change(session_id);
        let resolution = timeout_runtime_interaction_resolution(&interaction);
        if let Some(sender) = pending
            .responder
            .lock()
            .expect("pending interaction responder mutex poisoned")
            .take()
        {
            let _ = sender.send(resolution);
        }
        crate::logging::debug_with_fields(
            "runtime.interaction",
            "timed out runtime interaction",
            serde_json::json!({
                "session_id": session_id,
                "interaction_id": interaction_id,
                "pending_interaction_count_after": self.pending_interactions.write().len(),
            }),
        );
        Ok(())
    }
}

fn timeout_runtime_interaction_resolution(
    interaction: &crate::session::RuntimeInteraction,
) -> super::PendingInteractionResolution {
    if interaction.kernel_operation_id().is_some() {
        return super::PendingInteractionResolution {
            status: "timed_out",
            choice_id: None,
            reply: None,
        };
    }
    let Some(default_choice_id) = interaction.default_on_timeout() else {
        return super::PendingInteractionResolution {
            status: "timed_out",
            choice_id: None,
            reply: None,
        };
    };
    let Some(choice) = interaction.choice(default_choice_id) else {
        return super::PendingInteractionResolution {
            status: "timed_out",
            choice_id: None,
            reply: None,
        };
    };
    super::PendingInteractionResolution {
        status: "answered",
        choice_id: Some(choice.id().to_string()),
        reply: Some(choice.reply().to_string()),
    }
}

fn validate_runtime_interaction_custom_reply(
    custom_choice: &crate::session::RuntimeInteractionCustomChoice,
    reply: &str,
) -> Result<(), DaemonError> {
    let reply_len = reply.chars().count();
    if reply_len < custom_choice.min_length() {
        return Err(DaemonError::LocalTransport {
            operation: "resolve runtime interaction",
            message: format!(
                "custom_reply must be at least {} characters",
                custom_choice.min_length()
            ),
        });
    }
    if let Some(max_length) = custom_choice.max_length() {
        if reply_len > max_length {
            return Err(DaemonError::LocalTransport {
                operation: "resolve runtime interaction",
                message: format!("custom_reply must be at most {max_length} characters"),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interaction(default_on_timeout: Option<&str>) -> crate::session::RuntimeInteraction {
        crate::session::RuntimeInteraction::new(
            "interaction-1".to_string(),
            "agent-1".to_string(),
            crate::session::RuntimeInteractionKind::Choice,
            crate::session::RuntimeInteractionLevel::Info,
            Some("Pick".to_string()),
            "Pick one".to_string(),
            vec![
                crate::session::RuntimeInteractionChoice::new(
                    "yes".to_string(),
                    "Yes".to_string(),
                    "User picked yes.".to_string(),
                    None,
                ),
                crate::session::RuntimeInteractionChoice::new(
                    "no".to_string(),
                    "No".to_string(),
                    "User picked no.".to_string(),
                    None,
                ),
            ],
            None,
            Some(1),
            default_on_timeout.map(ToOwned::to_owned),
        )
    }

    #[test]
    fn timeout_without_default_returns_explicit_timeout_result() {
        let resolution = timeout_runtime_interaction_resolution(&interaction(None));

        assert_eq!(resolution.status, "timed_out");
        assert_eq!(resolution.choice_id, None);
        assert_eq!(resolution.reply, None);
    }

    #[test]
    fn timeout_with_default_returns_default_choice_reply() {
        let resolution = timeout_runtime_interaction_resolution(&interaction(Some("no")));

        assert_eq!(resolution.status, "answered");
        assert_eq!(resolution.choice_id.as_deref(), Some("no"));
        assert_eq!(resolution.reply.as_deref(), Some("User picked no."));
    }
}
