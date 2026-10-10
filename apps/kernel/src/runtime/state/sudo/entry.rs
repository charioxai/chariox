use super::*;

// MP-08/MP-10/MP-11: includes the policy relaunch delay and provider startup.
const SUDO_PROVIDER_RELAUNCH_TIMEOUT: Duration = Duration::from_secs(60);

fn not_ready() -> String {
    format!(
        "not ready within {} seconds",
        SUDO_PROVIDER_RELAUNCH_TIMEOUT.as_secs()
    )
}

fn sudo_ended(what: &str, detail: &str) -> DaemonError {
    error(format!(
        "{what}: {detail}; sudo window ended; retry /sudo when the provider is available"
    ))
}

fn catalog_refresh_failed(cause: DaemonError) -> DaemonError {
    let detail = match cause {
        DaemonError::LocalTransport { message, .. } => message,
        other => other.to_string(),
    };
    sudo_ended("provider catalog refresh failed", &detail)
}

fn catalog_not_ready() -> DaemonError {
    sudo_ended("provider catalog refresh failed", &not_ready())
}

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
        if policy::is_sudo_control(&request.prompt) {
            return Err(error(
                "use the terminal's sudo controls for status, extend or revoke",
            ));
        }
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
        if agent.is_metaagent() {
            return Err(error(
                "sudo requires a regular agent; finish Meta mode first",
            ));
        }
        // A10: a leased agent is elevated by its home kernel over the existing
        // lease, and only on a worker that enforces the window as well.
        if agent.remote_execution().is_some_and(|remote| {
            remote.relay_peer_protocol_version.unwrap_or(0)
                < super::leased::LEASED_SUDO_PEER_PROTOCOL_VERSION
        }) {
            return Err(error(
                "this agent's worker kernel predates leased sudo windows; update the worker or run the agent locally",
            ));
        }
        if self
            .owned
            .provider_store
            .get_run_for_agent(session.id(), agent_id)
            .is_some_and(|run| {
                self.owned
                    .provider_run_projection
                    .is_leased_provider_run(run.id())
            })
        {
            return Err(error(
                "a leased agent's sudo window is authorized only by its home kernel",
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
            task_id: None,
            duration_minutes: 0,
            expires_at_ms: None,
            revision: 0,
            warning_sent: false,
            deadline: None,
            placement: agent
                .remote_execution()
                .map(|remote| remote.execution_lease_id.clone()),
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
            // Drop the writer before projecting the end to clients: snapshots
            // read this store, including when provider dispatch has failed.
            let ended = self
                .owned
                .sudo_turns
                .lock()
                .expect("access state poisoned")
                .remove(&entry.entry_id);
            if let Some(ended) = ended {
                let _ = self.finish_sudo_window(&ended, "refused_or_cancelled");
            }
            // A window that never started leaves no supervised task behind.
            let _ = self.owned.withdraw_agent_task(&entry.entry_id);
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
        let requester = match &entry.requester {
            Some(grant) => format!(
                "External agent {} (OS pid {}, grant {})",
                grant.holder_executable.escape_debug(),
                grant.holder_pid,
                grant.grant_id
            ),
            None => format!("Terminal {}", entry.terminal_id),
        };
        let interaction = RuntimeInteraction::for_kernel_operation(&entry.entry_id, &entry.entry_id, "Authorize sudo window",
            format!("{} requests a sudo window for agent {} in session {}: one hour by default, up to 8 hours. It covers only this owner-authorized work and its kernel-correlated continuations, never unrelated prompts or messages, and it never answers approvals. Session inventory is included; each additional typed host operation requires fresh passkey authorization of its exact parameters. Restart, revoke or passkey rotation ends it. Requester-supplied prompt:\n{}", requester, entry.agent_id, entry.session_id, prompt),
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
        if answer.status == "timed_out" {
            return Err(error(
                "sudo request expired; answer the popup in a Chariox terminal",
            ));
        }
        if answer.choice_id.as_deref() != Some("approve") {
            return Err(error("sudo request refused"));
        }
        let minutes = super::sudo_window_minutes(answer.reply.as_deref())?;
        let verified = self
            .owned
            .sudo_verified_at
            .lock()
            .expect("sudo verification clocks poisoned")
            .remove(&entry.entry_id)
            .ok_or_else(|| error("sudo authorization has no fresh verification clock"))?;
        // The winning terminal answer records its identity before waking us;
        // the window starts at that fresh verification.
        let entry = self
            .owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .get_mut(&entry.entry_id)
            .map(|current| {
                super::window::open_window_at(current, minutes, verified);
                current.clone()
            })
            .ok_or_else(|| error("sudo request revoked"))?;
        let entry = &entry;
        self.audit_sudo(entry, "authorized")?;
        self.arm_sudo_timer(&entry.entry_id, entry.revision);
        let leased = entry.placement.is_some();
        // A provider that caches its tool catalog and already runs listed it
        // before this window: reload it once, idle, before the first turn. A
        // leased provider's catalog is the worker's; home never reloads it.
        let mut reload = !leased
            && self
                .owned
                .provider_store
                .get_run_for_agent(&entry.session_id, &entry.agent_id)
                .is_some_and(|run| {
                    crate::provider::provider_runtime_catalog_requires_reload(run.provider())
                });
        let attachments = crate::runtime::agent_actor::prompt_attachment_materialization::materialize_inline_prompt_attachments(&entry.session_id, &entry.agent_id, request.attachments.clone())?;
        // A cold launch uses the normal provider path. No elevated authority is
        // usable until admission installs the exact prompt and run identity.
        // A leased agent launches on its worker through the leased dispatch.
        if !leased {
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
        }
        let mut relaunch_deadline = None;
        // MP-08/MP-10/MP-11: the catalog budget counts only idle refresh
        // attempts. Busy work restarts it; human vault popups precede it.
        let mut catalog_deadline = None;
        let mut vault: Option<super::super::runtime_vault_unlock_state::VaultUnlockGuard> = None;
        loop {
            if reload && self.sudo_agent_busy(entry)? {
                catalog_deadline = None;
                // Ordinary work must not keep an operation unlock open.
                vault = None;
            } else if reload {
                use super::super::provider_reload::ProviderReloadOutcome;
                self.live_queued_sudo(entry)?;
                // A relock (lease expiry, another operation's guard) prompts
                // again; the budget pauses while the popups are open. Release
                // the stale guard first so its drop cannot relock the fresh
                // unlock.
                if !vault.as_ref().is_some_and(|guard| guard.still_unlocked()) {
                    drop(vault.take());
                    let prompted = tokio::time::Instant::now();
                    let unlock =
                        self.unlock_vault_for_agent_reload(&entry.session_id, &entry.agent_id);
                    vault = Some(
                        self.while_sudo_queued(entry, unlock)
                            .await?
                            .map_err(catalog_refresh_failed)?,
                    );
                    if let Some(deadline) = catalog_deadline.as_mut() {
                        *deadline += prompted.elapsed();
                    }
                }
                let deadline = *catalog_deadline
                    .get_or_insert(tokio::time::Instant::now() + SUDO_PROVIDER_RELAUNCH_TIMEOUT);
                let refresh = self.reload_agent_provider_if_idle(
                    &entry.session_id,
                    &entry.agent_id,
                    &super::super::provider_reload::ProviderReloadReason::RuntimeToolCatalog,
                    vault.as_ref(),
                );
                let outcome = tokio::time::timeout_at(deadline, refresh)
                    .await
                    .map_err(|_| catalog_not_ready())?
                    .map_err(catalog_refresh_failed)?;
                reload = matches!(outcome, ProviderReloadOutcome::Deferred);
                if reload && tokio::time::Instant::now() >= deadline {
                    return Err(catalog_not_ready());
                }
                if !reload {
                    catalog_deadline = None;
                    vault = None;
                }
                if matches!(outcome, ProviderReloadOutcome::Reloaded) {
                    relaunch_deadline =
                        Some(tokio::time::Instant::now() + SUDO_PROVIDER_RELAUNCH_TIMEOUT);
                }
            }
            // A detached relaunch can fail without leaving a run. Bound every
            // readiness state, including absent, Starting and Parked, so the
            // normal request error cleanup releases the window and work hold.
            if let Some(deadline) = relaunch_deadline {
                self.live_queued_sudo(entry)?;
                if tokio::time::Instant::now() >= deadline {
                    return Err(sudo_ended("provider relaunch failed", &not_ready()));
                }
                if !self.sudo_provider_running(entry) {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
                relaunch_deadline = None;
            }
            let submission = self.try_start_sudo(entry, request, prompt, &attachments, !reload)?;
            if let Some(mut submission) = submission {
                if let Some(dispatch) = submission.remote_dispatch.take() {
                    self.spawn_remote_prompt_dispatch(dispatch);
                }
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
    /// Human waits are bounded by their own popup expiry; a window that ends
    /// meanwhile drops `wait`, which closes its popups.
    async fn while_sudo_queued<T>(
        &self,
        entry: &KernelSudoTurn,
        wait: impl std::future::Future<Output = T>,
    ) -> Result<T, DaemonError> {
        tokio::pin!(wait);
        loop {
            tokio::select! {
                done = &mut wait => return Ok(done),
                _ = tokio::time::sleep(Duration::from_millis(250)) => {
                    self.live_queued_sudo(entry)?;
                }
            }
        }
    }
    fn sudo_provider_running(&self, entry: &KernelSudoTurn) -> bool {
        self.owned
            .provider_store
            .get_run_for_agent(&entry.session_id, &entry.agent_id)
            .is_some_and(|run| run.state() == crate::provider::ProviderRunState::Running)
    }
    /// The current window while its first turn has not started; Extend may
    /// have renewed it since the caller's read.
    fn live_queued_sudo(&self, entry: &KernelSudoTurn) -> Result<KernelSudoTurn, DaemonError> {
        let current = self
            .owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .get(&entry.entry_id)
            .filter(|current| current.prompt_id.is_none())
            .cloned();
        current
            .filter(|current| self.sudo_live(current))
            .ok_or_else(|| error("queued sudo was revoked"))
    }
    fn sudo_agent_busy(&self, entry: &KernelSudoTurn) -> Result<bool, DaemonError> {
        let session = self.owned.session_store.get_session(&entry.session_id)?;
        Ok(self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, &entry.agent_id)
            .is_some())
    }
    fn try_start_sudo(
        &self,
        entry: &KernelSudoTurn,
        request: &SubmitPromptRequest,
        text: &str,
        attachments: &[crate::session::PromptAttachment],
        catalog_ready: bool,
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        // Release sudo_turns before reading session/prompt state: interaction
        // resolution holds session_store, then prompt state, then sudo_turns.
        let entry = &self.live_queued_sudo(entry)?;
        let session = self.owned.session_store.get_session(&entry.session_id)?;
        let durable_work = self.owned.config_projection.snapshot().room_agent_tools;
        if durable_work {
            // Hold the agent so no other prompt takes the turn the owner
            // authorized; only this work's correlated prompts start until the
            // window ends.
            self.owned.prompt_state_owner.hold_sudo_work(
                &session,
                &entry.agent_id,
                &entry.entry_id,
                &entry.entry_id,
            );
        }
        // MP-08/MP-10/MP-11: native refresh can be Deferred even while idle.
        // Install the work hold above, but never admit a stale first turn,
        // including when ordinary work is cancelled during the busy check.
        if !catalog_ready
            || self
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
                .with_hidden_system_context("This is a human-authorized sudo window for this task only. Use chariox_kernel_request for kernel operations while it is live; it continues across your waits and correlated continuations of this task until it expires, the task ends or the owner revokes it. It never covers unrelated prompts or messages and cannot answer approvals. Never ask for or accept a passkey in agent output.")
                .with_attachments(attachments.to_vec()), force_queue: false, refresh_projection: true,
        };
        if durable_work {
            self.owned.admit_agent_task(&prepared)?;
        }
        // Provider launch and concurrent ordinary submissions can leave the
        // target busy. Retry without ever admitting a durable queued prompt.
        let leased = entry.placement.is_some();
        let submitted = if leased {
            self.owned
                .submit_remote_prepared_prompt_with_queue_policy(&prepared, false)
        } else {
            self.owned
                .submit_local_prepared_prompt_with_queue_policy(&prepared, false)
        };
        let submission = match submitted {
            Err(DaemonError::LocalTransport { message, .. })
                if message == "target agent is busy; retry when its provider is ready" =>
            {
                if durable_work {
                    let _ = self.owned.withdraw_agent_task(&entry.entry_id);
                }
                return Ok(None);
            }
            Ok(Some(submission)) => submission,
            result => {
                if durable_work {
                    let _ = self.owned.withdraw_agent_task(&entry.entry_id);
                }
                result?;
                return Err(error("provider unavailable for sudo"));
            }
        };
        let PromptSubmissionOutcome::Started { prompt } = &submission.outcome else {
            return Err(error("sudo must start a fresh turn"));
        };
        // A leased turn's worker run is bound when the worker accepts it.
        let run = match (&submission.dispatch, &submission.remote_dispatch) {
            (Some(dispatch), _) => Some(dispatch.provider_run_id.clone()),
            (None, Some(_)) => None,
            (None, None) => return Err(error("sudo provider dispatch unavailable")),
        };
        let mut turn = entry.clone();
        turn.prompt_id = Some(prompt.id().into());
        turn.provider_run_id = run;
        turn.task_id = durable_work.then(|| prompt.id().into());
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
        // The turn binds to the current window, so an Extend that landed
        // after the caller's read keeps its fresh deadline and revision.
        let Some(mut started) = access
            .get(&entry.entry_id)
            .filter(|current| bound && current.prompt_id.is_none())
            .filter(|current| policy::requester_grant_live(current, &grants))
            .cloned()
        else {
            return Err(error("sudo authorization revoked before dispatch"));
        };
        started.prompt_id = turn.prompt_id.clone();
        started.provider_run_id = turn.provider_run_id.clone();
        started.task_id = turn.task_id.clone();
        self.audit_sudo(&started, "started")?;
        access.insert(entry.entry_id.clone(), started);
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
