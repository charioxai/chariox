use super::*;

impl KernelRuntimeOwnedState {
    fn restore_session_and_publish_projection(
        &self,
        session: crate::session::RuntimeSession,
    ) -> Result<crate::session::RuntimeSession, DaemonError> {
        let session_id = session.id().to_string();
        self.session_store.restore_session(session);
        self.session_snapshot(&session_id)
    }

    pub(super) fn register_runtime_interaction(
        &self,
        session_id: &str,
        interaction: crate::session::RuntimeInteraction,
        responder: tokio::sync::oneshot::Sender<super::PendingInteractionResolution>,
    ) -> Result<(), DaemonError> {
        crate::logging::debug_with_fields(
            "runtime.interaction",
            "register runtime interaction requested",
            serde_json::json!({
                "session_id": session_id,
                "interaction_id": interaction.id(),
                "agent_id": interaction.agent_id(),
                "kind": format!("{:?}", interaction.kind()),
                "pending_store_ptr": format!("{:p}", std::sync::Arc::as_ptr(&self.pending_interactions.inner)),
                "active_interaction_count_before": self
                    .session_store
                    .get_session(session_id)
                    .ok()
                    .map(|session| session.active_interactions().len()),
                "pending_interaction_count_before": self.pending_interactions.write().len(),
            }),
        );
        let mut session = self.session_store.get_session(session_id)?;
        if session
            .active_interaction_for_agent(interaction.agent_id())
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation: "register runtime interaction",
                message: format!(
                    "agent {} already has an active interaction",
                    interaction.agent_id()
                ),
            });
        }
        session.add_active_interaction(interaction.clone());
        self.restore_session_and_publish_projection(session)?;
        self.pending_interactions.write().insert(
            interaction.id().to_string(),
            super::PendingInteraction {
                session_id: session_id.to_string(),
                responder: std::sync::Arc::new(std::sync::Mutex::new(Some(responder))),
            },
        );
        crate::logging::debug_with_fields(
            "runtime.interaction",
            "registered runtime interaction",
            serde_json::json!({
                "session_id": session_id,
                "interaction_id": interaction.id(),
                "agent_id": interaction.agent_id(),
                "pending_store_ptr": format!("{:p}", std::sync::Arc::as_ptr(&self.pending_interactions.inner)),
                "pending_interaction_count_after": self.pending_interactions.write().len(),
            }),
        );
        Ok(())
    }

    pub(super) fn resolve_runtime_interaction(
        &self,
        session_id: &str,
        interaction_id: &str,
        choice_id: &str,
        custom_reply: Option<&str>,
    ) -> Result<(), DaemonError> {
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
        if pending.session_id != session_id {
            return Err(DaemonError::LocalTransport {
                operation: "resolve runtime interaction",
                message: "interaction does not belong to the requested session".to_string(),
            });
        }
        let mut session = self.session_store.get_session(session_id)?;
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
        let pending = self
            .pending_interactions
            .write()
            .remove(interaction_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "resolve runtime interaction",
                message: format!("interaction {interaction_id} was not pending"),
            })?;
        let _ = session.remove_active_interaction(interaction_id);
        self.restore_session_and_publish_projection(session)?;
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
        crate::logging::debug_with_fields(
            "runtime.interaction",
            "timeout runtime interaction requested",
            serde_json::json!({
                "session_id": session_id,
                "interaction_id": interaction_id,
                "pending_interaction_count_before": self.pending_interactions.write().len(),
            }),
        );
        let pending = match self.pending_interactions.write().remove(interaction_id) {
            Some(pending) => pending,
            None => return Ok(()),
        };
        if pending.session_id != session_id {
            return Ok(());
        }
        let mut session = self.session_store.get_session(session_id)?;
        let Some(interaction) = session.remove_active_interaction(interaction_id) else {
            return Ok(());
        };
        self.restore_session_and_publish_projection(session)?;
        let resolution = if let Some(default_choice_id) = interaction.default_on_timeout() {
            if let Some(choice) = interaction.choice(default_choice_id) {
                super::PendingInteractionResolution {
                    status: "answered",
                    choice_id: Some(choice.id().to_string()),
                    reply: Some(choice.reply().to_string()),
                }
            } else {
                super::PendingInteractionResolution {
                    status: "timed_out",
                    choice_id: None,
                    reply: None,
                }
            }
        } else {
            super::PendingInteractionResolution {
                status: "timed_out",
                choice_id: None,
                reply: None,
            }
        };
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
