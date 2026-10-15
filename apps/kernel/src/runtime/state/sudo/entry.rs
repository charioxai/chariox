use super::*;

impl KernelRuntimeState {
    /// Called only after terminal identity and session ownership are checked by
    /// the router. The waiting future, including the full prompt, is ephemeral.
    pub(crate) async fn submit_sudo_prompt(
        &self,
        request: SubmitPromptRequest,
        owner: &str,
        terminal: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let prompt = policy::parse_sudo_prompt(&request.prompt)
            .unwrap_or_default()
            .to_owned();
        self.submit_sudo_entry(request, &prompt, owner, terminal, None)
            .await
    }

    pub(super) async fn submit_sudo_entry(
        &self,
        mut request: SubmitPromptRequest,
        prompt: &str,
        owner: &str,
        terminal: &str,
        requester: Option<KernelAccessGrant>,
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
        if prompt.trim().is_empty() {
            return Err(error("usage: /sudo <prompt>"));
        }
        self.sweep_sudo();
        let entry = KernelSudoTurn {
            entry_id: format!("sudo:{:016x}", rand::random::<u64>()),
            session_id: session.id().into(),
            agent_id: agent_id.into(),
            owner_user_id: owner.into(),
            terminal_id: terminal.into(),
            requester,
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
            if entry.requester.as_ref().is_some_and(|requester| {
                access.values().any(|turn| {
                    turn.prompt_id.is_none()
                        && turn
                            .requester
                            .as_ref()
                            .is_some_and(|pending| pending.holder_pid == requester.holder_pid)
                })
            }) {
                return Err(error("this requester already has a pending sudo request"));
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
            let ended = self
                .owned
                .sudo_turns
                .lock()
                .expect("access state poisoned")
                .remove(&entry.entry_id)
                .unwrap_or_else(|| entry.clone());
            let _ = self
                .owned
                .timeout_runtime_interaction(&entry.session_id, &entry.entry_id);
            let _ = self.record_sudo_end(&ended, "refused_or_cancelled");
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
        // MP-08 / MP-11: establish display facts in the same kernel operation,
        // before registration publishes the first popup. Never parse display copy.
        let requester = entry.requester.as_ref().and_then(|grant| {
            self.owned
                .kernel_access
                .lock()
                .expect("access state poisoned")
                .grants
                .get(&grant.grant_id)
                .map(|grant| crate::runtime::kernel_access::requester::project(&grant.holder))
        });
        let requester_name = if entry.requester.is_none() {
            "You"
        } else {
            match requester.as_ref().and_then(|r| r.provider_harness) {
                Some(KernelAccessProviderHarness::Claude) => "Claude Code",
                Some(KernelAccessProviderHarness::Codex) => "Codex",
                Some(KernelAccessProviderHarness::Opencode) => "OpenCode",
                None => "An external agent",
            }
        };
        let agent = self.owned.agent_store.get_agent(&entry.agent_id)?;
        let session = self.owned.session_store.get_session(&entry.session_id)?;
        let summary = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut task_summary: String = summary
            .chars()
            .filter(|c| {
                !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            })
            .take(160)
            .collect();
        if summary.chars().count() > 160 {
            task_summary.push('…');
        }
        let presentation = SudoRequestPresentation {
            requester_name: requester_name.into(),
            agent_name: agent.alias().unwrap_or(agent.agent_ref()).into(),
            session_name: session.alias().unwrap_or("Unnamed session").into(),
            task_summary,
            task: prompt.into(),
            grant_id: entry.requester.as_ref().map(|g| g.grant_id.clone()),
            terminal_id: entry.terminal_id.clone(),
        };
        let title = if entry.requester.is_none() {
            "Run one sudo task".into()
        } else {
            format!("{requester_name} wants to run one sudo task")
        };
        let mut interaction = RuntimeInteraction::for_kernel_operation(&entry.entry_id, &entry.entry_id, title,
            "Sudo allows critical approvals across this kernel until this task yields or is revoked.",
            vec![RuntimeInteractionChoice::new("refuse", "Refuse", "refuse", None), RuntimeInteractionChoice::new("approve", "Approve", "approve", None).requiring_passkey()])
            .with_timeout_sec(seconds).with_sudo_request(presentation);
        if let Some(requester) = requester {
            interaction = interaction.with_requester(requester);
        }
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
        if answer.status == "timed_out" {
            return Err(error(
                "sudo request expired; answer the popup in a Chariox terminal",
            ));
        }
        if answer.choice_id.as_deref() != Some("approve") {
            return Err(error("sudo request refused"));
        }
        // The winning terminal answer records its identity before waking us.
        let entry = self
            .owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .get(&entry.entry_id)
            .cloned()
            .ok_or_else(|| error("sudo request revoked"))?;
        let entry = &entry;
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
        let admitted = self.admit_sudo_turn(entry, &turn, bound);
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

    pub(super) fn admit_sudo_turn(
        &self,
        entry: &KernelSudoTurn,
        turn: &KernelSudoTurn,
        bound: bool,
    ) -> Result<(), DaemonError> {
        // Grant writers may read sessions; interaction resolution holds a
        // session writer before sudo_turns. Acquire grants before sudo_turns
        // here to avoid the session -> sudo -> grants -> session cycle.
        // Keep both writers through the queued-to-running transition.
        let grants = self
            .owned
            .kernel_access
            .lock()
            .expect("access state poisoned");
        let mut access = self.owned.sudo_turns.lock().expect("access state poisoned");
        if !bound
            || access.get(&entry.entry_id) != Some(entry)
            || !policy::requester_grant_live(entry, &grants)
        {
            return Err(error("sudo authorization revoked before dispatch"));
        }
        let cutoff = crate::runtime::kernel_access::process::birth_cutoff()
            .map_err(|e| error(e.to_string()))?;
        self.audit_sudo(turn, "started")?;
        self.owned
            .sudo_process_cutoffs
            .lock()
            .expect("sudo process cutoffs poisoned")
            .insert(entry.entry_id.clone(), cutoff);
        access.insert(entry.entry_id.clone(), turn.clone());
        Ok(())
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
            let _ = self.state.revoke_sudo(
                Some(&self.entry.owner_user_id),
                Some(&self.entry.entry_id),
                "request_cancelled",
            );
        }
    }
}
