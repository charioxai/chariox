use super::*;

mod maintenance;
mod registration;
#[cfg(test)]
mod regression_tests;

fn interaction_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "runtime interaction",
        message: message.into(),
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn resolve_runtime_interaction(
        &self,
        session_id: &str,
        interaction_id: &str,
        choice_id: &str,
        custom_reply: Option<&str>,
        caller_user_id: Option<&str>,
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
            pending
                .get(interaction_id)
                .cloned()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "resolve runtime interaction",
                    message: format!("interaction {interaction_id} was not pending"),
                })?
        };
        if pending.session_id != session_id || !pending.belongs_to(&self.session_store) {
            return Err(DaemonError::LocalTransport {
                operation: "resolve runtime interaction",
                message: "interaction does not belong to the requested session".to_string(),
            });
        }
        if pending
            .kernel_operation_owner
            .as_deref()
            .is_some_and(|owner| Some(owner) != caller_user_id)
        {
            return Err(interaction_error(
                "Only the operation owner can answer this decision",
            ));
        }
        if pending
            .kernel_operation_deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            return Err(interaction_error("Kernel operation decision expired"));
        }
        let mut sessions = self.session_store.write();
        let mut session = sessions.get_session(session_id)?.clone();
        let interaction = session
            .active_interactions()
            .iter()
            .find(|interaction| interaction.id() == interaction_id)
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "resolve runtime interaction",
                message: format!("interaction {interaction_id} is not active in session"),
            })?;
        let resolved_reply = if let Some(choice) = interaction.choice(choice_id) {
            if let Some(reply) = custom_reply {
                let custom_choice =
                    interaction
                        .custom_choice()
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "resolve runtime interaction",
                            message:
                                "custom_reply is only valid for interactions with a custom choice"
                                    .to_string(),
                        })?;
                validate_runtime_interaction_custom_reply(custom_choice, reply)?;
                reply.to_string()
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
        let pending = self
            .pending_interactions
            .write()
            .remove(interaction_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "resolve runtime interaction",
                message: format!("interaction {interaction_id} was not pending"),
            })?;
        let _ = session.remove_active_interaction(interaction_id);
        sessions.restore_session(session);
        drop(sessions);
        self.session_snapshot(session_id)?;
        self.terminal_stream
            .notify_terminal_projection_change(session_id);
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
            let mut pending = self.pending_interactions.write();
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
                .remove(interaction_id)
                .expect("checked pending interaction")
        };
        let mut sessions = self.session_store.write();
        let mut session = sessions.get_session(session_id)?.clone();
        let Some(interaction) = session.remove_active_interaction(interaction_id) else {
            return Ok(());
        };
        sessions.restore_session(session);
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
