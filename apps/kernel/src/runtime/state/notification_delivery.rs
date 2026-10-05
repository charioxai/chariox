//! MP-08 / MP-10: both source adapters use the ordinary durable queue for
//! admission, restart and fallback. Provider input uses the existing steer seam.
use super::*;
use crate::session::{WorkflowQueuedPrompt, WorkflowQueuedPromptStatus};

struct Injection {
    queued: WorkflowQueuedPrompt,
    dispatch: crate::app::KernelPromptDispatch,
    run_id: String,
}

impl KernelRuntimeState {
    pub(super) async fn deliver_pending_notification_injections(&self) {
        let candidates: Vec<_> = self
            .owned
            .session_store
            .read()
            .list_sessions()
            .into_iter()
            .flat_map(|session| {
                session
                    .workflow_queued_prompts()
                    .iter()
                    .filter(|item| item.notification_injection_pending())
                    .map(|item| (session.id().to_owned(), item.id().to_owned()))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (session, id) in self
            .app_control()
            .event_pump()
            .notification_batch(candidates, 8)
        {
            let Ok(Some(injection)) = self.owned.prepare_notification_injection(&session, &id)
            else {
                continue;
            };
            let dispatch = &injection.dispatch;
            let _permit = self
                .provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            if injection
                .queued
                .notification_expired_at(crate::session::unix_epoch_ms())
            {
                let _ = self.owned.prepare_notification_injection(&session, &id);
                continue;
            }
            // The exact active prompt is checked again by the provider seam.
            // A turn that ended while waiting for the lane cannot consume this item.
            let accepted = if self
                .owned
                .agent_store
                .get_agent(&dispatch.agent_id)
                .is_ok_and(|agent| agent.remote_execution().is_some())
            {
                let prompt = crate::session::PromptQueueItem::new(
                    &dispatch.prompt_id,
                    &dispatch.source_attachment_id,
                    &dispatch.agent_id,
                    &dispatch.prompt,
                    crate::session::PromptStatus::Queued,
                );
                self.steer_remote_agent_message_for_turn(
                    &session,
                    &prompt,
                    dispatch.target_active_prompt_id.as_deref(),
                )
                .await
                .map(|run| run.is_some())
            } else {
                if let Err(error) = self.owned.append_steering_prompt_history(
                    &session,
                    &dispatch.provider_run_id,
                    dispatch.target_active_prompt_id.as_deref().unwrap(),
                    &dispatch.source_attachment_id,
                    &dispatch.agent_id,
                    &dispatch.prompt_id,
                    &dispatch.prompt,
                    &dispatch.attachments,
                ) {
                    Err(error)
                } else {
                    self.enqueue_prompt_dispatch_with_acceptance(dispatch).await
                }
            };
            match accepted {
                Ok(accepted) => {
                    let _ = self
                        .owned
                        .settle_notification_injection(&session, &injection, accepted);
                }
                Err(_) => {
                    // An error can follow a provider write. Retain the exact admitted
                    // item rather than discard it or guess an acknowledged delivery.
                    self.owned.record_notice(
                        &session,
                        None,
                        self.owned
                            .attachment_store
                            .list_session_attachment_ids(&session),
                        "notification_inject_retry: notification remains durably pending",
                    );
                }
            }
        }
    }
}

impl KernelRuntimeOwnedState {
    fn prepare_notification_injection(
        &self,
        session_id: &str,
        id: &str,
    ) -> Result<Option<Injection>, DaemonError> {
        self.durable_state_store.with_workflow_runtime_transition_lock(|| {
            let mut sessions = self.session_store.write();
            let before = sessions.get_session(session_id)?;
            let Some(queued) = before.workflow_queued_prompts().iter().find(|q| q.id() == id && q.notification_injection_pending()).cloned() else { return Ok(None) };
            // Receipt sweep may already have marked it expired. Expiry must
            // retire hot work even when the original target/receipt is gone.
            if queued.notification_expired_at(crate::session::unix_epoch_ms()) {
                let mut after = before.clone();
                after.notification_prompt_mut(id).unwrap().mark_cancelled();
                self.durable_state_store.persist_workflow_runtime_transition(&after, "notification_inject_expired")?;
                sessions.restore_session(after);
                drop(sessions);
                self.record_notice(session_id, None, self.attachment_store.list_session_attachment_ids(session_id), "notification_inject_expired");
                return Ok(None);
            }
            // Arbitrary publication envelopes cannot request a kernel injection:
            // the original durable receipt must corroborate this exact queue item.
            if !self.durable_state_store.notification_queue_is_owned(&before, &queued)? { return Ok(None) }
            let inv = queued.publication_invocation().unwrap();
            let owner = inv.caller.get("owner_id").and_then(serde_json::Value::as_str).unwrap_or("");
            let target = match crate::durable_state::notification_target::WorkflowNotificationTarget::resolve(&sessions, owner, session_id, &inv.publication_id, Some(queued.queue_id())) {
                Ok(target) => target,
                Err(_) => {
                    let mut after = before.clone();
                    after.notification_prompt_mut(id).unwrap().mark_cancelled();
                    self.durable_state_store.persist_workflow_runtime_transition(&after, "notification_inject_target_unavailable")?;
                    sessions.restore_session(after);
                    drop(sessions);
                    self.record_notice(session_id, None, self.attachment_store.list_session_attachment_ids(session_id), "notification_inject_target_unavailable");
                    return Ok(None);
                }
            };
            if target.target().endpoint_id != queued.endpoint_id() { return Ok(None) }
            if !queued.notification_expired_at(crate::session::unix_epoch_ms()) && before.workflow_prompt_queues().iter().any(|q| q.id() == queued.queue_id() && !q.enabled()) {
                return Ok(None);
            }
            let mut after = before.clone();
            let reason;
            let result;
            if queued.notification_expired_at(crate::session::unix_epoch_ms()) {
                after.notification_prompt_mut(id).unwrap().mark_cancelled();
                reason = "notification_inject_expired";
                result = None;
            } else {
                let runs: Vec<_> = before.workflow_runs().iter().filter(|r|
                    r.workflow_id() == queued.workflow_id()
                        && !r.status().is_terminal()).collect();
                let target = (runs.len() == 1).then(|| runs[0]).and_then(|run| {
                    if run.endpoint_id() != queued.endpoint_id() || run.status() != crate::session::WorkflowRunStatus::Running { return None }
                    let entry = run.node_runs().first()?;
                    let active = self.prompt_state_owner.active_prompt_for_agent(&before, entry.agent_id())?;
                    if active.workflow_run_id() != Some(run.id()) || active.status() != crate::session::PromptStatus::Running || active.is_external()
                        || self.prompt_state_owner.prompt_is_sudo_bound(&before, entry.agent_id(), active.id()) { return None }
                    // A restart retries only the originally selected turn.
                    let saved = queued.publication_invocation()?.caller.get("notification_steer");
                    if saved.is_some_and(|s| s.get("prompt_id").and_then(serde_json::Value::as_str) != Some(active.id())) { return None }
                    let agent = self.agent_store.get_agent(entry.agent_id()).ok()?;
                    let provider_run_id = if let Some(remote) = agent.remote_execution() {
                        remote.active_worker_provider_run_id.clone()?
                    } else { self.provider_store.get_run_for_agent(session_id, entry.agent_id())?.id().to_owned() };
                    if saved.is_some_and(|s| s.get("provider_run_id").and_then(serde_json::Value::as_str) != Some(provider_run_id.as_str())) { return None }
                    Some((run.id().to_owned(), active, provider_run_id))
                });
                if let Some((run_id, active, provider_run_id)) = target {
                    let item = after.notification_prompt_mut(id).unwrap();
                    item.mark_running(&run_id);
                    item.notification_invocation_mut().unwrap().caller["notification_steer"] = serde_json::json!({"prompt_id":active.id(),"provider_run_id":provider_run_id});
                    let dispatch = crate::app::KernelPromptDispatch {
                        session_id: session_id.into(), provider_run_id, agent_id: active.target_agent_id().into(),
                        prompt_id: id.into(), target_active_prompt_id: Some(active.id().into()),
                        source_attachment_id: active.source_attachment_id().into(), prompt: queued.prompt().unwrap_or_default().into(),
                        hidden_system_context: String::new(), attachments: vec![], prompt_origin: active.prompt_origin(),
                        external_provider: None, external_provider_session_id: None, external_provider_turn_id: None, steering: true,
                    };
                    reason = "notification_inject_admitted";
                    result = Some(Injection { queued, dispatch, run_id });
                } else {
                    let item = after.notification_prompt_mut(id).unwrap();
                    item.mark_queued_for_retry();
                    item.notification_invocation_mut().unwrap().caller["delivery_mode"] = serde_json::json!("queue");
                    reason = if runs.len() > 1 { "notification_inject_multiple_runs_queued" } else { "notification_inject_idle_or_ended_queued" };
                    result = None;
                }
            }
            self.durable_state_store.persist_workflow_runtime_transition(&after, reason)?;
            sessions.restore_session(after);
            drop(sessions);
            if reason != "notification_inject_admitted" {
                self.record_notice(session_id, None, self.attachment_store.list_session_attachment_ids(session_id), reason);
            }
            Ok(result)
        })
    }

    fn settle_notification_injection(
        &self,
        session_id: &str,
        injection: &Injection,
        accepted: bool,
    ) -> Result<(), DaemonError> {
        self.durable_state_store
            .with_workflow_runtime_transition_lock(|| {
                let mut sessions = self.session_store.write();
                let mut after = sessions.get_session(session_id)?;
                let Some(item) = after.notification_prompt_mut(injection.queued.id()) else {
                    return Ok(());
                };
                if item.status() != WorkflowQueuedPromptStatus::Running
                    || item.workflow_run_id() != Some(injection.run_id.as_str())
                {
                    return Ok(());
                }
                if accepted {
                    item.mark_completed();
                } else {
                    item.mark_queued_for_retry();
                    item.notification_invocation_mut().unwrap().caller["delivery_mode"] =
                        serde_json::json!("queue");
                }
                self.durable_state_store
                    .persist_workflow_runtime_transition(
                        &after,
                        if accepted {
                            "notification_injected"
                        } else {
                            "notification_inject_turn_ended_queued"
                        },
                    )?;
                sessions.restore_session(after);
                drop(sessions);
                if !accepted {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store
                            .list_session_attachment_ids(session_id),
                        "notification_inject_turn_ended_queued",
                    );
                }
                Ok(())
            })
    }
}

#[cfg(test)]
mod tests;
