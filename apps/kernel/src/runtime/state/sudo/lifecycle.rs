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
            if !matches!(
                payload["outcome"].as_str(),
                Some("requested" | "authorized" | "started")
            ) {
                continue;
            }
            let Ok(turn) = serde_json::from_value::<KernelSudoTurn>(payload["turn"].clone()) else {
                continue;
            };
            let _ = self.record_sudo_end(&turn, "restart_dropped");
            self.owned.record_notice(&turn.session_id, None, self.owned.attachment_store.list_session_attachment_ids(&turn.session_id),
                "Kernel restart discarded sudo state whose last durable receipt was pending or running; it may already have ended before restart. Enter /sudo again with a fresh passkey.");
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
            self.owned
                .sudo_process_cutoffs
                .lock()
                .expect("sudo process cutoffs poisoned")
                .remove(&turn.entry_id);
            if let Err(error) = self.record_sudo_end(&turn, reason) {
                receipt_error.get_or_insert(error);
            }
            let _ = self
                .owned
                .timeout_runtime_interaction(&turn.session_id, &turn.entry_id);
            if let Some(prompt_id) = turn.prompt_id.as_deref() {
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
        self.audit_sudo(turn, reason).map_err(|error| {
            crate::logging::warn_with_fields("kernel_access.sudo", "sudo authority removed but its durable receipt failed", serde_json::json!({"entry_id": turn.entry_id, "outcome": reason, "error": error.to_string()}));
            error
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
            .filter(|turn| !self.sudo_live(turn))
            .collect::<Vec<_>>();
        let removed = {
            let mut access = self.owned.sudo_turns.lock().expect("access state poisoned");
            ended
                .into_iter()
                .filter_map(|turn| {
                    (access.get(&turn.entry_id) == Some(&turn))
                        .then(|| access.remove(&turn.entry_id))
                        .flatten()
                })
                .collect::<Vec<_>>()
        };
        for turn in removed {
            let _ = self
                .owned
                .timeout_runtime_interaction(&turn.session_id, &turn.entry_id);
            self.owned
                .sudo_process_cutoffs
                .lock()
                .expect("sudo process cutoffs poisoned")
                .remove(&turn.entry_id);
            let _ = self.record_sudo_end(&turn, "ended");
        }
    }
}
