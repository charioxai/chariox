use super::*;

impl KernelRuntimeState {
    /// Called only after terminal identity and session ownership are checked by
    /// the router. The waiting future, including the full prompt, is ephemeral.
    pub(crate) async fn submit_sudo_prompt(
        &self,
        mut request: SubmitPromptRequest,
        owner: &str,
        terminal: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        if request.target_agent_id.is_none() {
            request.target_agent_id = self
                .owned
                .session_store
                .get_session(&request.session_id)?
                .focused_agent_id()
                .map(str::to_owned);
        }
        let agent_id = request
            .target_agent_id
            .as_deref()
            .ok_or_else(|| error("sudo needs a focused agent"))?;
        self.owned.require_publication_activation()?;
        let attachment = self
            .owned
            .ensure_attachment_in_session(&request.session_id, &request.attachment_id)?;
        if attachment.owner_user_id() != owner {
            return Err(error("sudo needs the host's own terminal attachment"));
        }
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let session = self.owned.session_store.get_session(&request.session_id)?;
        if session.owner_user_id() != owner
            || session.status() == SessionStatus::Ended
            || agent.session_id() != session.id()
        {
            return Err(error("only the session host can authorize sudo"));
        }
        if agent.remote_execution().is_some() || agent.is_metaagent() {
            return Err(error(
                "sudo requires a local regular agent; finish Meta mode first",
            ));
        }
        let prompt = request
            .prompt
            .trim_start()
            .strip_prefix("/sudo")
            .unwrap_or_default()
            .trim();
        if prompt.is_empty() {
            return Err(error("usage: /sudo <prompt>"));
        }
        self.sweep_sudo();
        let entry = KernelSudoTurn {
            entry_id: format!("sudo:{:016x}", rand::random::<u64>()),
            session_id: session.id().into(),
            agent_id: agent_id.into(),
            owner_user_id: owner.into(),
            terminal_id: terminal.into(),
            prompt_id: None,
            provider_run_id: None,
        };
        {
            let mut access = self.owned.sudo_turns.lock().expect("access state poisoned");
            if access.values().any(|turn| turn.agent_id == agent_id) {
                return Err(error(
                    "this agent already has a pending or running sudo turn",
                ));
            }
            self.audit_sudo(&entry, "requested")?;
            access.insert(entry.entry_id.clone(), entry.clone());
        }
        let mut guard = PendingSudoGuard {
            state: self.clone(),
            entry: entry.clone(),
            armed: true,
        };
        let result = self
            .authorize_and_start_sudo(&entry, &request, prompt)
            .await;
        if result.is_ok() {
            guard.armed = false;
        }
        if result.is_err() {
            self.owned
                .sudo_turns
                .lock()
                .expect("access state poisoned")
                .remove(&entry.entry_id);
            let _ = self
                .owned
                .timeout_runtime_interaction(&entry.session_id, &entry.entry_id);
            let _ = self.audit_sudo(&entry, "refused_or_cancelled");
        }
        result
    }

    async fn authorize_and_start_sudo(
        &self,
        entry: &KernelSudoTurn,
        request: &SubmitPromptRequest,
        prompt: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let seconds = u64::from(
            self.owned
                .config_projection
                .snapshot()
                .user_config
                .kernel_access
                .request_timeout_minutes,
        ) * 60;
        let interaction = RuntimeInteraction::for_kernel_operation(&entry.entry_id, &entry.entry_id, "Authorize one sudo turn",
            format!("Terminal {} requests one sudo turn for agent {} in session {}. It may answer critical approvals across this kernel until it yields. Prompt:\n{}", entry.terminal_id, entry.agent_id, entry.session_id, prompt),
            vec![RuntimeInteractionChoice::new("refuse", "Refuse", "refuse", None), RuntimeInteractionChoice::new("approve", "Approve", "approve", None).requiring_passkey()]).with_timeout_sec(seconds);
        let rx = self
            .create_kernel_operation_interaction(
                &entry.session_id,
                &entry.owner_user_id,
                interaction,
            )
            .await?;
        if !self
            .owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .contains_key(&entry.entry_id)
        {
            self.owned
                .timeout_runtime_interaction(&entry.session_id, &entry.entry_id)?;
            return Err(error("sudo request revoked"));
        }
        let answer = tokio::time::timeout(Duration::from_secs(seconds), rx)
            .await
            .map_err(|_| error("sudo request expired; answer the popup in a Chariox terminal"))?
            .map_err(|_| error("sudo request cancelled"))?;
        if answer.choice_id.as_deref() != Some("approve") {
            return Err(error("sudo request refused"));
        }
        self.audit_sudo(entry, "authorized")?;
        let attachments = crate::runtime::agent_actor::prompt_attachment_materialization::materialize_inline_prompt_attachments(&entry.session_id, &entry.agent_id, request.attachments.clone())?;
        // A cold launch uses the normal provider path. No elevated authority is
        // usable until admission installs the exact prompt and run identity.
        self.with_app_side_effect(|app| {
            if !self
                .owned
                .sudo_turns
                .lock()
                .expect("access state poisoned")
                .contains_key(&entry.entry_id)
            {
                return Err(error("queued sudo was revoked"));
            }
            app.ensure_prompt_provider_run_for_agent(&entry.session_id, &entry.agent_id)
        })
        .await?;
        loop {
            let submission = self.try_start_sudo(entry, request, prompt, &attachments)?;
            if let Some(submission) = submission {
                if let Some(dispatch) = submission.dispatch {
                    self.start_active_turn_with_trace_id(
                        &dispatch.session_id,
                        &dispatch.agent_id,
                        &dispatch.prompt_id,
                        &dispatch.provider_run_id,
                        &entry.entry_id,
                    );
                    self.spawn_prompt_dispatch(dispatch, self.provider_runtime_lanes.clone());
                }
                return Ok(LocalDaemonResponse::PromptSubmitted {
                    outcome: submission.outcome,
                    agent_activity: self.agent_activity_for_session(&submission.session),
                    session: submission.session,
                    agent_activity_revision: self.owned.session_projection.change_sequence(),
                });
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    fn try_start_sudo(
        &self,
        entry: &KernelSudoTurn,
        request: &SubmitPromptRequest,
        text: &str,
        attachments: &[crate::session::PromptAttachment],
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        // Release sudo_turns before reading session/prompt state: interaction
        // resolution holds session_store, then prompt state, then sudo_turns.
        let present = self
            .owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .contains_key(&entry.entry_id);
        if !present || !self.sudo_live(entry) {
            return Err(error("queued sudo was revoked"));
        }
        let session = self.owned.session_store.get_session(&entry.session_id)?;
        if self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &entry.agent_id)
            .is_some()
        {
            return Ok(None);
        }
        let prepared = crate::app::KernelPreparedPromptSubmission {
            session_id: entry.session_id.clone(),
            prompt: PromptQueueItem::new(&entry.entry_id, &request.attachment_id, &entry.agent_id, text, PromptStatus::Queued)
                .with_hidden_system_context("This is a human-authorized sudo turn. Use chariox_kernel_request for kernel operations; sudo ends when this turn yields. Never ask for or accept a passkey in agent output.")
                .with_attachments(attachments.to_vec()), force_queue: false, refresh_projection: true,
        };
        // Provider launch and concurrent ordinary submissions can leave the
        // target busy. Retry without ever admitting a durable queued prompt.
        let submission = match self
            .owned
            .submit_local_prepared_prompt_with_queue_policy(&prepared, false)
        {
            Err(DaemonError::LocalTransport { message, .. })
                if message == "target agent is busy; retry when its provider is ready" =>
            {
                return Ok(None)
            }
            result => result?.ok_or_else(|| error("provider unavailable for sudo"))?,
        };
        let PromptSubmissionOutcome::Started { prompt } = &submission.outcome else {
            return Err(error("sudo must start a fresh turn"));
        };
        let dispatch = submission
            .dispatch
            .as_ref()
            .ok_or_else(|| error("sudo provider dispatch unavailable"))?;
        let mut turn = entry.clone();
        turn.prompt_id = Some(prompt.id().into());
        turn.provider_run_id = Some(dispatch.provider_run_id.clone());
        let bound = self.owned.prompt_state_owner.bind_sudo_turn(
            &session,
            &entry.agent_id,
            prompt.id(),
            &entry.entry_id,
        );
        let admitted = {
            let mut access = self.owned.sudo_turns.lock().expect("access state poisoned");
            if !bound || access.get(&entry.entry_id) != Some(entry) {
                Err(error("sudo authorization revoked before dispatch"))
            } else {
                self.audit_sudo(&turn, "started").map(|()| {
                    access.insert(entry.entry_id.clone(), turn.clone());
                })
            }
        };
        if let Err(error) = admitted {
            if let Ok(Some(cancelled)) = self.owned.cancel_local_prompt_if_matches(
                &turn.session_id,
                &turn.agent_id,
                "sudo-admission-rollback",
                turn.prompt_id.as_deref(),
            ) {
                if let Some(dispatch) = cancelled.dispatch {
                    self.spawn_prompt_abort(dispatch, self.provider_runtime_lanes.clone());
                }
            }
            return Err(error);
        }
        Ok(Some(submission))
    }
}

// A disconnected/cancelled request cannot leave an orphaned authorization.
struct PendingSudoGuard {
    state: KernelRuntimeState,
    entry: KernelSudoTurn,
    armed: bool,
}
impl Drop for PendingSudoGuard {
    fn drop(&mut self) {
        if self.armed {
            self.state.revoke_sudo(
                Some(&self.entry.owner_user_id),
                Some(&self.entry.entry_id),
                "request_cancelled",
            );
        }
    }
}
