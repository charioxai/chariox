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
            // A saved uncertain send is reconciled before inspecting current runs.
            // Its old turn ending is never evidence that the notification did not land.
            if self.reconcile_notification_injection(&session, &id).await {
                continue;
            }
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
            let remote = self
                .owned
                .agent_store
                .get_agent(&dispatch.agent_id)
                .ok()
                .and_then(|agent| agent.remote_execution().cloned());
            let structured = remote.is_none()
                && self
                    .owned
                    .provider_store
                    .get_run(&dispatch.provider_run_id)
                    .is_ok_and(|run| {
                        self.owned
                            .provider_store
                            .run_uses_structured_prompt_io(&run)
                    });
            if self
                .owned
                .mark_notification_submit(&session, &injection, remote.as_ref())
                .is_err()
            {
                continue;
            }
            let accepted = if remote.is_some() {
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
                    remote.as_ref(),
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
                Ok(true) if structured => {
                    // try_send accepted the command, not the provider prompt. The
                    // finished-submit reapers settle the exact durable injection.
                }
                Ok(accepted) => {
                    let _ = self
                        .owned
                        .settle_notification_injection(&session, &injection, accepted);
                }
                Err(_) if structured => {
                    let _ = self
                        .owned
                        .settle_notification_injection(&session, &injection, false);
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
                        "notification_inject_uncertain: held until acknowledgement or expiry",
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
            if queued.publication_invocation().and_then(|i| i.caller.get("notification_steer"))
                .is_some_and(|s| s.get("remote_uncertain").is_some()
                    || s.get("submit_epoch").is_some()) {
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
        settle_injection(
            &self.durable_state_store,
            &self.session_store,
            session_id,
            injection.queued.id(),
            &injection.run_id,
            accepted,
        )?;
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
    }
    fn mark_notification_submit(
        &self,
        session_id: &str,
        injection: &Injection,
        remote: Option<&crate::agent::RemoteAgentBinding>,
    ) -> Result<(), DaemonError> {
        self.durable_state_store
            .with_workflow_runtime_transition_lock(|| {
                let mut sessions = self.session_store.write();
                let mut after = sessions.get_session(session_id)?;
                let item = after
                    .notification_prompt_mut(injection.queued.id())
                    .ok_or_else(|| notification_error("injection disappeared"))?;
                if item.status() != WorkflowQueuedPromptStatus::Running
                    || item.workflow_run_id() != Some(injection.run_id.as_str())
                {
                    return Err(notification_error("injection changed"));
                }
                let saved =
                    &mut item.notification_invocation_mut().unwrap().caller["notification_steer"];
                saved["agent_id"] = serde_json::json!(injection.dispatch.agent_id);
                saved["source_attachment_id"] =
                    serde_json::json!(injection.dispatch.source_attachment_id);
                if let Some(remote) = remote {
                    if remote.active_worker_provider_run_id.as_deref()
                        != Some(injection.dispatch.provider_run_id.as_str())
                    {
                        return Err(notification_error(
                            "original worker run changed before send intent",
                        ));
                    }
                    saved["remote_uncertain"] = serde_json::to_value(RemoteInjectionIdentity {
                        agent_id: injection.dispatch.agent_id.clone(),
                        worker_kernel_id: remote.worker_kernel_id.clone(),
                        worker_machine_id: remote.worker_machine_id.clone(),
                        leased_agent_id: remote.leased_agent_id.clone(),
                        execution_lease_id: remote.execution_lease_id.clone(),
                        target_home_prompt_id: injection
                            .dispatch
                            .target_active_prompt_id
                            .clone()
                            .unwrap(),
                        worker_provider_run_id: injection.dispatch.provider_run_id.clone(),
                    })
                    .map_err(|_| notification_error("cannot encode injection identity"))?;
                } else {
                    // Every local path persists the same intent before I/O, including
                    // native bridges and PTY writers that can finish after an error.
                    // Neither a restart nor an ended turn proves non-acceptance.
                    saved["submit_epoch"] =
                        serde_json::json!(self.provider_store.structured_submit_epoch());
                }
                self.durable_state_store
                    .persist_workflow_runtime_transition(
                        &after,
                        "notification_inject_submitting",
                    )?;
                sessions.restore_session(after);
                Ok(())
            })
    }
}

// These records are kernel-owned invocation context, never client-supplied authority.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct RemoteInjectionIdentity {
    agent_id: String,
    worker_kernel_id: String,
    worker_machine_id: String,
    leased_agent_id: String,
    execution_lease_id: String,
    target_home_prompt_id: String,
    worker_provider_run_id: String,
}
fn notification_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "notification injection",
        message: message.into(),
    }
}

impl KernelRuntimeState {
    // True means this item is held or settled; ordinary selection must not touch it.
    async fn reconcile_notification_injection(&self, session_id: &str, id: &str) -> bool {
        let Ok(snapshot) = self.owned.session_store.get_session(session_id) else {
            return true;
        };
        let Some(item) = snapshot
            .workflow_queued_prompts()
            .iter()
            .find(|q| q.id() == id)
        else {
            return true;
        };
        let Some(value) = item
            .publication_invocation()
            .and_then(|i| i.caller.get("notification_steer"))
            .and_then(|s| s.get("remote_uncertain"))
        else {
            return false;
        };
        if item.notification_expired_at(crate::session::unix_epoch_ms()) {
            let _ = self.owned.prepare_notification_injection(session_id, id);
            return true;
        }
        if !self
            .owned
            .durable_state_store
            .notification_queue_is_owned(&snapshot, item)
            .unwrap_or(false)
        {
            return true;
        }
        let Ok(identity) = serde_json::from_value::<RemoteInjectionIdentity>(value.clone()) else {
            return true;
        };
        let result =
            super::remote_prompt_worker_submission_runtime::query_remote_queued_steer_receipt(
                self,
                &identity.agent_id,
                id,
                &identity.worker_kernel_id,
                &identity.worker_machine_id,
                &identity.leased_agent_id,
                &identity.target_home_prompt_id,
                &identity.worker_provider_run_id,
                &identity.execution_lease_id,
            )
            .await;
        if let Ok(Some(receipt)) = result {
            let _ = self
                .owned
                .settle_notification_remote_receipt(session_id, id, &receipt);
        }
        true
    }
}

impl KernelRuntimeOwnedState {
    fn settle_notification_remote_receipt(
        &self,
        session_id: &str,
        id: &str,
        receipt: &crate::transport::relay_peer::LeasedPromptReceipt,
    ) -> Result<(), DaemonError> {
        let snapshot = self.session_store.get_session(session_id)?;
        let item = snapshot
            .workflow_queued_prompts()
            .iter()
            .find(|q| q.id() == id)
            .ok_or_else(|| notification_error("injection missing"))?;
        let saved = item
            .publication_invocation()
            .and_then(|i| i.caller.get("notification_steer"))
            .and_then(|s| s.get("remote_uncertain"))
            .ok_or_else(|| notification_error("uncertainty missing"))?;
        let identity: RemoteInjectionIdentity = serde_json::from_value(saved.clone())
            .map_err(|_| notification_error("identity invalid"))?;
        if !self
            .durable_state_store
            .notification_queue_is_owned(&snapshot, item)?
        {
            return Err(notification_error("receipt ownership changed"));
        }
        let agent = self.agent_store.get_agent(&identity.agent_id)?;
        let binding = agent
            .remote_execution()
            .ok_or_else(|| notification_error("remote binding missing"))?;
        if binding.worker_kernel_id != identity.worker_kernel_id
            || binding.worker_machine_id != identity.worker_machine_id
            || binding.leased_agent_id != identity.leased_agent_id
            || binding.execution_lease_id != identity.execution_lease_id
            || receipt.home_prompt_id != id
            || receipt.target_home_prompt_id.as_deref()
                != Some(identity.target_home_prompt_id.as_str())
            || receipt.worker_provider_run_id != identity.worker_provider_run_id
            || receipt.execution_lease_id.as_deref() != Some(identity.execution_lease_id.as_str())
        {
            return Err(notification_error(
                "receipt conflicts with original worker/lease/turn",
            ));
        }
        use crate::transport::relay_peer::LeasedPromptReceiptPhase;
        let accepted = match receipt.phase {
            LeasedPromptReceiptPhase::SteerAccepted => true,
            LeasedPromptReceiptPhase::SteerRejected => false,
            LeasedPromptReceiptPhase::SteerDispatching => return Ok(()),
            _ => return Err(notification_error("receipt has no steer outcome")),
        };
        if accepted {
            let source_attachment = item
                .publication_invocation()
                .and_then(|i| i.caller.get("notification_steer"))
                .and_then(|s| s.get("source_attachment_id"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("workflow_notification");
            self.append_steering_prompt_history(
                session_id,
                &identity.worker_provider_run_id,
                &identity.target_home_prompt_id,
                source_attachment,
                &identity.agent_id,
                id,
                item.prompt().unwrap_or_default(),
                &[],
            )?;
        }
        settle_injection(
            &self.durable_state_store,
            &self.session_store,
            session_id,
            id,
            item.workflow_run_id()
                .ok_or_else(|| notification_error("injection run missing"))?,
            accepted,
        )
    }
}

/// Both finished-submit reapers use this correlation before touching an active prompt.
pub(crate) fn finish_structured_notification_submit(
    store: &DurableKernelStateStore,
    sessions: &SessionStateStore,
    finished: &crate::provider::FinishedProviderPromptSubmitJob,
) -> Result<bool, DaemonError> {
    let Ok(snapshot) = sessions.get_session(&finished.session_id) else {
        return Ok(false);
    };
    let Some(item) = snapshot
        .workflow_queued_prompts()
        .iter()
        .find(|q| q.id() == finished.prompt_id)
    else {
        return Ok(false);
    };
    let Some(saved) = item
        .publication_invocation()
        .and_then(|i| i.caller.get("notification_steer"))
    else {
        return Ok(false);
    };
    if saved.get("submit_epoch").is_none() {
        return Ok(false);
    }
    if saved
        .get("provider_run_id")
        .and_then(serde_json::Value::as_str)
        != Some(finished.provider_run_id.as_str())
        || saved.get("agent_id").and_then(serde_json::Value::as_str)
            != Some(finished.agent_id.as_str())
    {
        return Err(notification_error("submit result conflicts with injection"));
    }
    if item.status() == WorkflowQueuedPromptStatus::Running && item.notification_injection_pending()
    {
        if !store.notification_queue_is_owned(&snapshot, item)? {
            return Ok(false);
        }
        let accepted = match &finished.result {
            Ok(_) => true,
            Err(DaemonError::ProviderPromptSteerRejected { .. }) => false,
            Err(_) => {
                // A write/read/disconnect error is not a negative acknowledgement.
                // The persisted submit intent holds the exact original provider/turn
                // until expiry; neither changed epochs nor ended turns permit replay.
                crate::logging::warn_with_fields(
                    "daemon.notification_delivery",
                    "notification_inject_local_uncertain: held until acknowledgement or expiry",
                    serde_json::json!({"session_id": finished.session_id, "prompt_id": finished.prompt_id, "provider_run_id": finished.provider_run_id}),
                );
                return Ok(true);
            }
        };
        settle_injection(
            store,
            sessions,
            &finished.session_id,
            &finished.prompt_id,
            item.workflow_run_id()
                .ok_or_else(|| notification_error("injection run missing"))?,
            accepted,
        )?;
    }
    Ok(true)
}

fn settle_injection(
    store: &DurableKernelStateStore,
    sessions: &SessionStateStore,
    session_id: &str,
    id: &str,
    run_id: &str,
    accepted: bool,
) -> Result<(), DaemonError> {
    store.with_workflow_runtime_transition_lock(|| {
        let mut sessions = sessions.write();
        let mut after = sessions.get_session(session_id)?;
        let Some(item) = after.notification_prompt_mut(id) else {
            return Ok(());
        };
        if item.status() != WorkflowQueuedPromptStatus::Running
            || item.workflow_run_id() != Some(run_id)
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
        store.persist_workflow_runtime_transition(
            &after,
            if accepted {
                "notification_injected"
            } else {
                "notification_inject_turn_ended_queued"
            },
        )?;
        sessions.restore_session(after);
        Ok(())
    })
}

#[cfg(test)]
mod tests;
