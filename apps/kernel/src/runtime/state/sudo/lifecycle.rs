use super::*;

impl KernelRuntimeState {
    pub(in crate::runtime::state) fn recover_sudo_notices(&self) {
        let Ok(events) = self
            .owned
            .durable_state_store
            .load_events_by_kind("kernel_access.sudo")
        else {
            return;
        };
        let mut latest = BTreeMap::new();
        for event in events {
            if let Some(id) = event.subject_id.clone() {
                latest.insert(id, event.payload);
            }
        }
        for payload in latest.into_values() {
            if matches!(
                payload["outcome"].as_str(),
                Some("expired" | "restart_dropped")
            ) {
                if let Ok(turn) = serde_json::from_value::<KernelSudoTurn>(payload["turn"].clone())
                {
                    self.queue_sudo_end_wake(&turn, payload["outcome"].as_str().unwrap());
                }
                continue;
            }
            if !matches!(
                payload["outcome"].as_str(),
                Some(
                    "requested"
                        | "authorized"
                        | "started"
                        | "timer_armed"
                        | "warning"
                        | "extension_requested"
                        | "extended"
                        | "timer_arm_missed"
                        | "timer_warning_missed"
                )
            ) {
                continue;
            }
            let Ok(turn) = serde_json::from_value::<KernelSudoTurn>(payload["turn"].clone()) else {
                continue;
            };
            let _ = self.record_sudo_end(&turn, "restart_dropped");
            self.queue_sudo_end_wake(&turn, "restart_dropped");
            self.owned.record_notice(&turn.session_id, None, self.owned.attachment_store.list_session_attachment_ids(&turn.session_id),
                "Kernel restart ended a sudo window (fail closed); it may already have ended before restart. Any open work continues as a regular agent. Enter /sudo again with a fresh passkey to re-elevate.");
        }
    }
    pub(crate) fn revoke_sudo(
        &self,
        owner: Option<&str>,
        id: Option<&str>,
        reason: &str,
    ) -> Result<usize, DaemonError> {
        let revoked = {
            let mut access = self.owned.sudo_turns.lock().expect("access state poisoned");
            let ids = access
                .iter()
                .filter(|(key, turn)| {
                    id.is_none_or(|id| id == key.as_str())
                        && owner.is_none_or(|owner| owner == turn.owner_user_id)
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            ids.into_iter()
                .filter_map(|id| access.remove(&id))
                .collect::<Vec<_>>()
        };
        let count = revoked.len();
        let mut receipt_error = None;
        for turn in revoked {
            // Revocation interrupts the elevated turn; expiry lets it continue.
            let bound = self
                .owned
                .session_store
                .get_session(&turn.session_id)
                .ok()
                .and_then(|session| {
                    self.owned
                        .prompt_state_owner
                        .sudo_bound_prompt(&session, &turn.agent_id)
                })
                .filter(|(entry, _)| *entry == turn.entry_id)
                .map(|(_, prompt)| prompt);
            if let Err(error) = self.finish_sudo_window(&turn, reason) {
                receipt_error.get_or_insert(error);
            }
            if let Some(prompt_id) = bound.as_deref().or(turn.prompt_id.as_deref()) {
                if let Ok(Some(cancelled)) = self.owned.cancel_local_prompt_if_matches(
                    &turn.session_id,
                    &turn.agent_id,
                    "sudo-revoke",
                    Some(prompt_id),
                ) {
                    if let Some(dispatch) = cancelled.dispatch {
                        self.spawn_prompt_abort(dispatch, self.provider_runtime_lanes.clone());
                    }
                }
            }
        }
        if let Some(error) = receipt_error {
            return Err(error);
        }
        Ok(count)
    }

    pub(super) fn record_sudo_end(
        &self,
        turn: &KernelSudoTurn,
        reason: &str,
    ) -> Result<(), DaemonError> {
        self.audit_sudo(turn, reason).inspect_err(|error| {
            crate::logging::warn_with_fields("kernel_access.sudo", "sudo authority removed but its durable receipt failed", serde_json::json!({"entry_id": turn.entry_id, "outcome": reason, "error": error.to_string()}));
        })
    }

    pub(crate) fn list_sudo_turns(&self, owner: &str) -> Vec<KernelSudoTurn> {
        self.sweep_sudo();
        self.owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .values()
            .filter(|turn| turn.owner_user_id == owner)
            .cloned()
            .collect()
    }

    pub(crate) fn sweep_sudo(&self) {
        self.sweep_sudo_from(None);
    }

    pub(super) fn sweep_sudo_from(&self, timer: Option<&str>) {
        let turns = self
            .owned
            .sudo_turns
            .lock()
            .expect("access state poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let ended = turns
            .into_iter()
            .filter_map(|turn| {
                let reason = self.owned.sudo_end_reason(&turn)?;
                Some((turn, reason))
            })
            .collect::<Vec<_>>();
        let removed = {
            let mut access = self.owned.sudo_turns.lock().expect("access state poisoned");
            ended
                .into_iter()
                .filter_map(|(turn, reason)| {
                    (access.get(&turn.entry_id) == Some(&turn))
                        .then(|| access.remove(&turn.entry_id))
                        .flatten()
                        .map(|turn| (turn, reason))
                })
                .collect::<Vec<_>>()
        };
        for (turn, reason) in removed {
            let late = turn.deadline.is_some_and(|deadline| {
                std::time::Instant::now().saturating_duration_since(deadline) > SUDO_TIMER_TOLERANCE
            });
            if reason == "expired" && late && timer != Some(turn.entry_id.as_str()) {
                self.sudo_timer_alert(&turn, "expiry");
            }
            let _ = self.finish_sudo_window(&turn, reason);
        }
    }
}
