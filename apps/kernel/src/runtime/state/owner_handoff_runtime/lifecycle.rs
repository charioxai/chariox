//! MP-08 / MP-10 / MP-11 A07: durable outcomes, restart and source-loss reconciliation.
use super::*;
impl KernelRuntimeState {
    /// Settle the obligation with the safe outcome and wake the agent.
    pub(super) fn finish_handoff(
        &self,
        room: &str,
        handoff: &RuntimeHandoff,
        result: &HandoffOutcome,
        actor: &str,
    ) {
        let id = RuntimeHandoff::interaction_id(&handoff.obligation_id);
        let public = serde_json::to_value(result).ok();
        let audit = self.owned.durable_state_store.append_event(
            "handoff.outcome",
            Some(id.clone()),
            json!({"actor":actor,"agent_id":handoff.agent_id,"task_id":handoff.task_id,
                "binding":{"tab_id":handoff.target.tab_id,"generation":handoff.target.generation,
                    "document_id":handoff.target.document_id,"node_ref":handoff.target.node_ref,
                    "origin":handoff.target.origin},
                "outcome":result,"kind":handoff.kind,"action":result.action,"status":result.status,
                "reason_code":result.reason_code,"resolved_at_ms":crate::session::unix_epoch_ms()}),
        );
        if let Err(error) = audit {
            tracing::warn!(%error, "MP-11 A07: hand-off audit was not recorded");
        }
        let settled = self.owned.durable_state_store.agent_lifecycle(
            crate::durable_state::agent_lifecycle::Operation::SourceOutcome {
                public_answer: public,
                room: room.to_owned(),
                source: id,
                occurrence: format!(
                    "handoff-{}",
                    serde_json::to_value(result.status)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default()
                ),
                success: result.status == HandoffStatus::Completed,
                now: crate::session::unix_epoch_ms(),
            },
        );
        if let Err(error) = settled {
            tracing::warn!(%error, "MP-08 A07: hand-off outcome retained for sweep reconciliation");
        }
        self.schedule_agent_lifecycle_sweep();
    }

    /// Sweep reconciliation for an open `hand_off` obligation: `Some(false)`
    /// when the hand-off is gone; a restored hand-off whose responder was lost
    /// is re-armed for its remaining window (its target is revalidated before
    /// any action), or expires.
    pub(in crate::runtime::state) async fn reconcile_handoff(
        &self,
        session: &crate::session::RuntimeSession,
        handoff_id: &str,
    ) -> Option<bool> {
        // Never race a live owner action with the source-loss sweep.
        if self
            .owned
            .handoff_claims
            .lock()
            .ok()
            .is_some_and(|claims| claims.contains(handoff_id))
        {
            return None;
        }
        // Crash recovery must not re-arm a write-ahead claim or replay input.
        match self.owned.durable_state_store.load_subject_events_by_kind(
            handoff_id,
            "handoff.claimed",
            1,
        ) {
            Ok(events) if !events.is_empty() => {
                let claim = &events[0];
                if claim.payload["room"].as_str() != Some(session.id()) {
                    return Some(false);
                }
                let Ok(handoff) =
                    serde_json::from_value::<RuntimeHandoff>(claim.payload["handoff"].clone())
                else {
                    return Some(false);
                };
                let result = match self.owned.durable_state_store.load_subject_events_by_kind(
                    handoff_id,
                    "handoff.outcome",
                    1,
                ) {
                    Ok(events) => events
                        .last()
                        .and_then(|event| {
                            serde_json::from_value::<HandoffOutcome>(
                                event.payload["outcome"].clone(),
                            )
                            .ok()
                        })
                        .unwrap_or_else(|| {
                            outcome(
                                handoff_id,
                                HandoffStatus::Uncertain,
                                "withdrawn",
                                Some("restart_after_claim"),
                            )
                        }),
                    Err(_) => return None,
                };
                {
                    let mutation = self.owned.begin_managed_activity_mutation();
                    let mut sessions = self.owned.session_store.write();
                    if let Ok(mut latest) = sessions.get_session(session.id()) {
                        if latest.remove_active_interaction(handoff_id).is_some() {
                            sessions.restore_session(latest);
                            mutation.record();
                        }
                    }
                }
                self.finish_handoff(session.id(), &handoff, &result, "kernel");
                return None;
            }
            Err(_) => return None, // Unknown durability never permits replay.
            _ => {}
        }
        let Some(handoff) = session
            .active_interactions()
            .iter()
            .find(|i| i.id() == handoff_id)
            .and_then(|i| i.handoff().cloned())
        else {
            return Some(false);
        };
        if self
            .owned
            .pending_interactions
            .write()
            .get(handoff_id)
            .is_some_and(|p| p.belongs_to(&self.owned.session_store))
        {
            return None;
        }
        let remaining = handoff
            .expires_at_ms
            .saturating_sub(crate::session::unix_epoch_ms())
            / 1000;
        {
            let mutation = self.owned.begin_managed_activity_mutation();
            let mut sessions = self.owned.session_store.write();
            if let Ok(mut latest) = sessions.get_session(session.id()) {
                if latest.remove_active_interaction(handoff_id).is_some() {
                    sessions.restore_session(latest);
                    mutation.record();
                }
            }
        }
        if remaining < 1 {
            let _ = self.owned.session_snapshot(session.id());
            self.owned
                .terminal_stream
                .notify_terminal_projection_change(session.id());
            let result = outcome(
                handoff_id,
                HandoffStatus::Expired,
                "timeout",
                Some("timeout"),
            );
            self.finish_handoff(session.id(), &handoff, &result, "kernel");
            return None;
        }
        if let Err(error) = self
            .register_handoff_interaction(
                session.id(),
                session.owner_user_id(),
                handoff.clone(),
                remaining,
            )
            .await
        {
            tracing::warn!(%error, "MP-08 A07: restored hand-off could not be re-armed");
            let result = outcome(
                handoff_id,
                HandoffStatus::Expired,
                "withdrawn",
                Some("restore_failed"),
            );
            self.finish_handoff(session.id(), &handoff, &result, "kernel");
        }
        None
    }

    /// Cancelling the task withdraws its pending hand-off; false when none
    /// was pending.
    pub(in crate::runtime::state) fn withdraw_handoff(&self, room: &str, handoff_id: &str) -> bool {
        // A claimed browser operation has a current-task cancellation fence.
        // Its own settlement reports the physical result; do not invent a lost source.
        if self
            .owned
            .handoff_claims
            .lock()
            .ok()
            .is_some_and(|claims| claims.contains(handoff_id))
        {
            return true;
        }
        let Some(owner) = self
            .owned
            .pending_interactions
            .write()
            .get(handoff_id)
            .and_then(|p| p.kernel_operation_owner.clone())
        else {
            return false;
        };
        let Ok((handoff, _claim)) = self
            .owned
            .claim_handoff(room, handoff_id, &owner, |_| Ok(()))
        else {
            return false;
        };
        let result = outcome(
            handoff_id,
            HandoffStatus::Cancelled,
            "withdrawn",
            Some("task_cancelled"),
        );
        self.finish_handoff(room, &handoff, &result, "kernel");
        true
    }
}
