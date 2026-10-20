//! MP-08 / MP-10 / MP-11 A04: window clock, warnings, extension and release.
//! Authority always compares the monotonic deadline at use, so a late timer
//! can never extend a window; timers only make expiry and warnings visible.
use super::*;
use crate::durable_state::agent_lifecycle::{self as ledger, Operation};
use std::time::Instant;

/// The duration chosen in the passkey popup: one hour unless the owner picked
/// 2, 4 or 8 hours. Anything else, including more than 8 hours, is refused.
pub(crate) fn sudo_window_minutes(reply: Option<&str>) -> Result<u32, DaemonError> {
    match reply.map(str::trim).filter(|reply| *reply != "approve") {
        None | Some("") => Ok(SUDO_DEFAULT_MINUTES),
        Some(reply) => reply
            .parse::<u32>()
            .ok()
            .filter(|minutes| SUDO_WINDOW_MINUTES.contains(minutes))
            .ok_or_else(|| error("a sudo window lasts 1, 2, 4 or 8 hours")),
    }
}

/// Starts the window at verification time; extension restarts it, never banks.
#[cfg(test)]
pub(super) fn open_window(turn: &mut KernelSudoTurn, minutes: u32) {
    open_window_at(
        turn,
        minutes,
        (Instant::now(), crate::session::unix_epoch_ms()),
    );
}

/// Regression-only short windows, keyed by session so parallel tests keep the
/// real durations. Real elapsed-time proof stays with the live drills.
#[cfg(test)]
pub(crate) static SUDO_WINDOW_LENGTH_FOR_TEST: std::sync::Mutex<BTreeMap<String, Duration>> =
    std::sync::Mutex::new(BTreeMap::new());

pub(super) fn open_window_at(turn: &mut KernelSudoTurn, minutes: u32, verified: (Instant, u64)) {
    let length = Duration::from_secs(u64::from(minutes) * 60);
    #[cfg(test)]
    let length = SUDO_WINDOW_LENGTH_FOR_TEST
        .lock()
        .unwrap()
        .get(&turn.session_id)
        .copied()
        .unwrap_or(length);
    turn.duration_minutes = minutes;
    turn.deadline = Some(verified.0 + length);
    turn.expires_at_ms = Some(verified.1 + length.as_millis() as u64);
    turn.revision += 1;
    turn.warning_sent = false;
}

impl KernelRuntimeOwnedState {
    pub(in crate::runtime::state) fn sudo_windows_for_session(
        &self,
        session_id: &str,
    ) -> Vec<KernelSudoTurn> {
        self.sudo_turns
            .lock()
            .expect("access state poisoned")
            .values()
            .filter(|turn| turn.session_id == session_id && turn.deadline.is_some())
            .cloned()
            .collect()
    }

    fn notify_session(&self, session_id: &str, agent_id: &str, message: String) {
        self.record_notice_for_agent(
            session_id,
            None,
            Some(agent_id),
            self.attachment_store
                .list_session_attachment_ids(session_id),
            message,
        );
        let _ = self.session_snapshot(session_id);
    }

    /// Kernel-correlated continuations of the window's task may start; any
    /// other prompt for a held agent stays queued and is shown as deferred.
    pub(in crate::runtime::state) fn admit_sudo_work_prompt(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &str,
        prompt: &str,
    ) -> bool {
        let window = self
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .values()
            .find(|turn| {
                turn.session_id == session.id() && turn.agent_id == agent && turn.task_id.is_some()
            })
            .cloned();
        let Some(window) = window else {
            return false;
        };
        let correlated = self
            .durable_state_store
            .agent_tasks(Some(session.id()), Some(agent))
            .is_ok_and(|tasks| {
                tasks.iter().any(|task| {
                    Some(task.task_id.as_str()) == window.task_id.as_deref()
                        && (task.prompt_id == prompt
                            || task.pending_prompt_id.as_deref() == Some(prompt))
                })
            });
        correlated
            && self.prompt_state_owner.admit_sudo_work_prompt(
                session,
                agent,
                &window.entry_id,
                prompt,
            )
    }

    /// The work task a held agent may still receive inbox events for.
    pub(in crate::runtime::state) fn sudo_work_task(
        &self,
        session_id: &str,
        agent: &str,
    ) -> Option<String> {
        self.sudo_turns
            .lock()
            .expect("access state poisoned")
            .values()
            .find(|turn| turn.session_id == session_id && turn.agent_id == agent)
            .and_then(|turn| turn.task_id.clone())
    }
}

impl KernelRuntimeState {
    /// Proof of life: the scheduled task itself records that it runs, then
    /// wakes for the warning and the expiry. The sweep supervises both.
    pub(super) fn arm_sudo_timer(&self, entry_id: &str, revision: u64) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let state = self.clone();
        let id = entry_id.to_owned();
        self.owned.sudo_timer_changes.record_change();
        runtime.spawn(async move {
            // Exercise the same scheduled-wake path before claiming liveness.
            tokio::time::sleep(Duration::from_millis(25)).await;
            let armed = state
                .owned
                .sudo_turns
                .lock()
                .expect("access state poisoned")
                .get(&id)
                .filter(|turn| turn.revision == revision)
                .cloned();
            let Some(turn) = armed else { return };
            if state.audit_sudo(&turn, "timer_armed").is_err() {
                return;
            }
            state
                .owned
                .sudo_timers
                .lock()
                .expect("sudo timers poisoned")
                .insert(id.clone(), (revision, 0));
            loop {
                let sequence = state.owned.sudo_timer_changes.sequence();
                let next = state
                    .owned
                    .sudo_turns
                    .lock()
                    .expect("access state poisoned")
                    .get(&id)
                    .filter(|turn| turn.revision == revision)
                    .and_then(|turn| {
                        let deadline = turn.deadline?;
                        Some(if turn.warning_sent {
                            deadline
                        } else {
                            deadline.checked_sub(SUDO_WARNING).unwrap_or(deadline)
                        })
                    });
                let Some(next) = next else {
                    break;
                };
                tokio::select! {
                    _ = tokio::time::sleep_until(next.into()) => {},
                    _ = state.owned.sudo_timer_changes.wait_for_change_after(sequence) => continue,
                }
                state.pump_sudo_windows(Some(&id));
                if Instant::now() >= next {
                    // A transition that cannot be recorded is retried, not spun.
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        });
    }

    /// Called by each window's timer and by the kernel pump (the dead-man):
    /// ends expired windows, sends the one 10-minute warning per revision and
    /// alerts when a timer is missing or late instead of staying silent.
    pub(crate) fn pump_sudo_windows(&self, timer: Option<&str>) {
        self.retry_sudo_end_wakes();
        self.sweep_sudo_from(timer);
        let now = Instant::now();
        let due = {
            let mut access = self.owned.sudo_turns.lock().expect("access state poisoned");
            access
                .values_mut()
                .filter_map(|turn| {
                    let deadline = turn.deadline?;
                    if turn.warning_sent || deadline.saturating_duration_since(now) > SUDO_WARNING
                    {
                        return None;
                    }
                    let late = now.saturating_duration_since(deadline.checked_sub(SUDO_WARNING)?);
                    let mut warning = turn.clone();
                    warning.warning_sent = true;
                    if let Some(task) = turn.task_id.as_deref() {
                        if self.owned.durable_state_store.agent_lifecycle(Operation::Occur(ledger::occurrence(
                            &turn.session_id, &turn.agent_id, &turn.entry_id,
                            &format!("{}:{}:warning", turn.entry_id, turn.revision), "sudo_warning",
                            serde_json::json!({"task_id": task, "expires_at_ms": turn.expires_at_ms, "message": "Your sudo window is nearing expiry; only the owner can extend it with a fresh passkey."}),
                        ))).is_err() { return None; }
                    }
                    // A failed receipt leaves the warning due for retry.
                    if self.audit_sudo(&warning, "warning").is_err() { return None; }
                    turn.warning_sent = true;
                    Some((turn.clone(), late))
                })
                .collect::<Vec<_>>()
        };
        for (turn, late) in due {
            let remaining = turn.deadline.map_or(0, |d| {
                d.saturating_duration_since(now).as_secs().div_ceil(60)
            });
            self.owned.notify_session(&turn.session_id, &turn.agent_id, format!(
                "Sudo window {} for agent {} expires in {remaining} minutes. Use /sudo extend to renew it with a fresh passkey; regular work continues after expiry.",
                turn.entry_id, turn.agent_id));
            if timer != Some(turn.entry_id.as_str()) && late > SUDO_TIMER_TOLERANCE {
                self.sudo_timer_alert(&turn, "warning");
            }
        }
        let unarmed = {
            let access = self.owned.sudo_turns.lock().expect("access state poisoned");
            let mut timers = self.owned.sudo_timers.lock().expect("sudo timers poisoned");
            access
                .values()
                .filter(|turn| {
                    turn.deadline.is_some_and(|deadline| {
                        let opened = deadline
                            .checked_sub(Duration::from_secs(u64::from(turn.duration_minutes) * 60))
                            .unwrap_or(deadline);
                        now.saturating_duration_since(opened) > SUDO_TIMER_TOLERANCE
                    })
                })
                .filter_map(|turn| {
                    let (armed, alerted) = timers.entry(turn.entry_id.clone()).or_insert((0, 0));
                    (*armed != turn.revision).then(|| {
                        let first = *alerted != turn.revision;
                        *alerted = turn.revision;
                        (turn.clone(), first)
                    })
                })
                .collect::<Vec<_>>()
        };
        for (turn, first) in unarmed {
            if first {
                self.sudo_timer_alert(&turn, "arm");
            }
            self.arm_sudo_timer(&turn.entry_id, turn.revision);
        }
    }

    pub(super) fn sudo_timer_alert(&self, turn: &KernelSudoTurn, stage: &str) {
        let _ = self.audit_sudo(turn, &format!("timer_{stage}_missed"));
        self.owned.notify_session(&turn.session_id, &turn.agent_id, format!(
            "Alert: the sudo window {} timer missed its {stage}; the kernel sweep enforced it. Authority always uses the kernel deadline.",
            turn.entry_id));
        if let Some(task) = turn.task_id.as_deref() {
            let _ = self.owned.durable_state_store.agent_lifecycle(Operation::Occur(ledger::occurrence(
                &turn.session_id, &turn.agent_id, &turn.entry_id,
                &format!("{}:{}:timer_{stage}_missed", turn.entry_id, turn.revision), "sudo_timer_alert",
                serde_json::json!({"task_id": task, "stage": stage, "message": "The sudo timer missed a scheduled wake. The kernel sweep enforced the deadline and retried the scheduler."}),
            )));
        }
    }

    /// Releases the causal fence and makes the end visible. Deferred prompts
    /// and inbox events then run as distinct regular turns, and waiting work
    /// re-evaluates through the ordinary wake path.
    pub(super) fn finish_sudo_window(
        &self,
        turn: &KernelSudoTurn,
        reason: &str,
    ) -> Result<(), DaemonError> {
        self.spawn_leased_sudo_update(turn, true);
        self.owned
            .sudo_timers
            .lock()
            .expect("sudo timers poisoned")
            .remove(&turn.entry_id);
        self.owned
            .sudo_scopes
            .lock()
            .expect("sudo scopes poisoned")
            .remove(&turn.entry_id);
        self.owned
            .sudo_verified_at
            .lock()
            .expect("sudo verification clocks poisoned")
            .retain(|id, _| {
                id != &turn.entry_id && !id.starts_with(&format!("{}:", turn.entry_id))
            });
        self.owned.sudo_timer_changes.record_change();
        let mut interactions = vec![turn.entry_id.clone()];
        interactions.extend(
            self.owned
                .pending_interactions
                .write()
                .iter()
                .filter(|(id, pending)| {
                    pending.session_id == turn.session_id
                        && id.starts_with(&format!("{}:", turn.entry_id))
                })
                .map(|(id, _)| id.clone()),
        );
        for id in interactions {
            let _ = self
                .owned
                .timeout_runtime_interaction(&turn.session_id, &id);
        }
        if let Ok(session) = self.owned.session_store.get_session(&turn.session_id) {
            self.owned.prompt_state_owner.release_sudo_work(
                &session,
                &turn.agent_id,
                &turn.entry_id,
            );
        }
        if let Some(run) = self
            .owned
            .provider_store
            .get_run_for_agent(&turn.session_id, &turn.agent_id)
        {
            self.owned
                .provider_run_projection
                .catalog_changes()
                .invalidate(run.id());
        }
        let receipt = self.record_sudo_end(turn, reason);
        if turn.deadline.is_none() {
            return receipt;
        }
        self.owned.notify_session(&turn.session_id, &turn.agent_id, format!(
            "Sudo window {} for agent {} ended ({reason}). Its regular work continues without elevation; enter /sudo again with a fresh passkey to re-elevate.",
            turn.entry_id, turn.agent_id));
        self.queue_sudo_end_wake(turn, reason);
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let state = self.clone();
            let (room, agent) = (turn.session_id.clone(), turn.agent_id.clone());
            runtime.spawn(async move {
                Box::pin(state.advance_project_queued_prompt_after_settlement(&room, &agent)).await;
                let _ = Box::pin(state.deliver_agent_inbox(&room, &agent)).await;
            });
        }
        receipt
    }

    /// Opens one fresh-passkey interaction for the host. On verification the
    /// expiry becomes now plus the chosen duration if the window, its revision
    /// and its work are unchanged; concurrent or stale responses fail.
    pub(crate) async fn extend_sudo_window(
        &self,
        request: ExtendKernelSudoRequest,
        owner: &str,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.sweep_sudo();
        let attachment = self
            .owned
            .ensure_attachment_in_session(&request.session_id, &request.attachment_id)?;
        let turn = self
            .owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .get(&request.entry_id)
            .cloned()
            .filter(|turn| turn.session_id == request.session_id && turn.deadline.is_some())
            .ok_or_else(|| error("no live sudo window with that id; enter /sudo again"))?;
        if attachment.owner_user_id() != owner || turn.owner_user_id != owner {
            return Err(error("only the session host can extend sudo"));
        }
        if turn.revision != request.revision {
            return Err(error(
                "sudo window changed; reload its status and extend again",
            ));
        }
        let decision_timeout = u64::from(
            self.owned
                .config_projection
                .snapshot()
                .user_config
                .kernel_access
                .request_timeout_minutes,
        ) * 60;
        let remaining = turn
            .deadline
            .map_or(0, |deadline| {
                deadline.saturating_duration_since(Instant::now()).as_secs()
            })
            .clamp(1, decision_timeout.max(1));
        let id = format!("{}:extend:{}", turn.entry_id, turn.revision);
        let interaction = RuntimeInteraction::for_kernel_operation(&id, &id, "Extend sudo window",
            format!("Extend agent {}'s sudo window {} in session {} for the same owner-authorized work. The new expiry is now plus the selected duration (at most 8 hours); unrelated prompts and messages stay outside it, and it never answers approvals.",
                turn.agent_id, turn.entry_id, turn.session_id),
            vec![RuntimeInteractionChoice::new("refuse", "Refuse", "refuse", None), RuntimeInteractionChoice::new("approve", "Extend", "approve", None).requiring_passkey()])
            .with_timeout_sec(remaining);
        let rx = self
            .create_kernel_operation_interaction(&turn.session_id, owner, interaction)
            .await?;
        self.audit_sudo(&turn, "extension_requested")?;
        let answer = tokio::time::timeout(Duration::from_secs(remaining), rx)
            .await
            .map_err(|_| error("sudo window expired before the extension was approved"))?
            .map_err(|_| error("sudo extension cancelled"))?;
        if answer.choice_id.as_deref() != Some("approve") {
            return Err(error("sudo extension refused or expired"));
        }
        let minutes = sudo_window_minutes(answer.reply.as_deref())?;
        let verified = self
            .owned
            .sudo_verified_at
            .lock()
            .expect("sudo verification clocks poisoned")
            .remove(&id)
            .ok_or_else(|| error("sudo extension has no fresh verification clock"))?;
        let extended = {
            let mut access = self.owned.sudo_turns.lock().expect("access state poisoned");
            let current = access
                .get_mut(&turn.entry_id)
                .filter(|current| {
                    current.revision == turn.revision
                        && current.deadline.is_some_and(|d| Instant::now() < d)
                })
                .ok_or_else(|| error("sudo window ended or changed before the extension"))?;
            open_window_at(current, minutes, verified);
            current.clone()
        };
        if !self.sudo_live(&extended) {
            return Err(error("sudo window ended before the extension"));
        }
        self.audit_sudo(&extended, "extended")?;
        self.arm_sudo_timer(&extended.entry_id, extended.revision);
        self.spawn_leased_sudo_update(&extended, false);
        self.owned.notify_session(
            &extended.session_id,
            &extended.agent_id,
            format!(
                "Sudo window {} for agent {} extended by fresh passkey: {} minutes from now.",
                extended.entry_id, extended.agent_id, minutes
            ),
        );
        Ok(LocalDaemonResponse::KernelSudoExtended { turn: extended })
    }
}
