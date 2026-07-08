//! Remote-agent prompt ownership transitions.
//!
//! This module owns prompt queue state for agents leased to remote kernels. Local provider prompt
//! lifecycle remains in `prompt`.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn submit_remote_prepared_prompt(
        &self,
        prepared: &crate::app::KernelPreparedPromptSubmission,
    ) -> Result<Option<crate::app::KernelPromptSubmission>, DaemonError> {
        let session_id = prepared.session_id.clone();
        let attachment_id = prepared.prompt.source_attachment_id().to_string();
        if !crate::scheduler::runtime::is_workflow_prompt_attachment(&attachment_id) {
            let _ = self.ensure_attachment_in_session(&session_id, &attachment_id)?;
        }
        let target_agent_id = prepared.prompt.target_agent_id().to_string();
        let target_agent = self.agent_store.get_agent(&target_agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id,
                agent_id: target_agent_id,
            });
        }
        let Some(remote_execution) = target_agent.remote_execution().cloned() else {
            return Ok(None);
        };
        let session = self.session_store.get_session(&session_id)?;
        let queued_while_active = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, &target_agent_id)
            .is_some();
        let will_queue = prepared.force_queue || queued_while_active;
        let prompt = if will_queue {
            prepared.prompt.clone()
        } else {
            prepared
                .prompt
                .clone()
                .with_id(self.session_store.reserve_prompt_id())
        };
        let outcome = self.prompt_state_owner.submit_prepared_prompt(
            &session,
            prompt,
            prepared.force_queue,
        )?;
        let outcome_agent_id = match &outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt }
            | crate::session::PromptSubmissionOutcome::Queued { prompt } => {
                prompt.target_agent_id().to_string()
            }
        };
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, &outcome_agent_id);
        self.mirror_prompt_owner_agent_state(
            &session_id,
            &outcome_agent_id,
            active_prompt,
            queued_prompts,
        )?;
        let remote_dispatch = if let crate::session::PromptSubmissionOutcome::Started { prompt } =
            &outcome
        {
            let _ = self.record_started_user_prompt(
                &session_id,
                prompt.source_attachment_id(),
                prompt,
            )?;
            Some(crate::app::KernelRemotePromptDispatch {
                session_id: session_id.clone(),
                agent_id: target_agent_id,
                prompt_id: prompt.id().to_string(),
                worker_kernel_id: remote_execution.worker_kernel_id,
                leased_agent_id: remote_execution.leased_agent_id,
                relay_url: remote_execution.relay_url,
                relay_token: remote_execution.relay_token,
                source_attachment_id: prompt.source_attachment_id().to_string(),
                prompt: prompt.prompt().to_string(),
                attachments: prompt.attachments().to_vec(),
                workspace_live_sync_mode: Some(
                    crate::provider::provider_workspace_live_sync_mode_for_session(
                        target_agent.provider(),
                        &self.config_projection.snapshot(),
                        Some(&session),
                    ),
                ),
                prompt_origin: prompt.prompt_origin(),
                external_provider: prompt.external_provider().map(str::to_string),
                external_provider_session_id: prompt
                    .external_provider_session_id()
                    .map(str::to_string),
                external_provider_turn_id: prompt.external_provider_turn_id().map(str::to_string),
                workflow_context: None,
            })
        } else {
            None
        };
        let session = if prepared.refresh_projection {
            self.session_snapshot(&session_id)?
        } else {
            self.session_snapshot_without_projection_update(&session_id)?
        };
        Ok(Some(crate::app::KernelPromptSubmission {
            outcome,
            session,
            dispatch: None,
            remote_dispatch,
        }))
    }

    pub(super) fn complete_remote_prompt_owner(
        &self,
        session_id: &str,
        agent_id: &str,
        remote_provider_run_id: &str,
        next_queued_prompt: Option<&crate::session::PromptQueueItem>,
    ) -> Result<crate::session::PromptCompletion, DaemonError> {
        let agent = self.agent_store.get_agent(agent_id)?;
        if agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
            });
        }
        let _ = self
            .agent_store
            .set_remote_execution_active_worker_provider_run_id(agent_id, None)?;
        let session = self.session_store.get_session(session_id)?;
        let completed = self
            .prompt_state_owner
            .complete_active_prompt_only(&session, agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let recipient_attachment_ids = self
            .attachment_store
            .list_session_attachment_ids(session_id);
        self.record_assistant_message_completion(
            session_id,
            remote_provider_run_id,
            recipient_attachment_ids,
            &format!("prompt-complete:{}", completed.id()),
            crate::session::unix_epoch_ms(),
        );
        let started_next = if let Some(expected_next) = next_queued_prompt {
            let active = self
                .prompt_state_owner
                .activate_next_queued_prompt_with_prompt_id(
                    &session,
                    agent_id,
                    Some(expected_next.id()),
                    self.session_store.reserve_prompt_id(),
                )?;
            if let Some(active_prompt) = active.as_ref() {
                let _ = self.record_started_user_prompt(
                    session_id,
                    active_prompt.source_attachment_id(),
                    active_prompt,
                )?;
            }
            active
        } else {
            None
        };
        let (active_prompt, queued_prompts) =
            self.prompt_state_owner.state_parts(&session, agent_id);
        self.mirror_prompt_owner_agent_state(session_id, agent_id, active_prompt, queued_prompts)?;
        let _ = self.session_snapshot(session_id)?;
        Ok(crate::session::PromptCompletion {
            completed,
            started_next,
        })
    }

    pub(super) fn begin_remote_prompt_cancellation(
        &self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<crate::app::KernelPromptCancellation, DaemonError> {
        let _ = self.ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent = self.agent_store.get_agent(target_agent_id)?;
        if target_agent.session_id() != session_id {
            return Err(DaemonError::AgentNotInSession {
                session_id: session_id.to_string(),
                agent_id: target_agent_id.to_string(),
            });
        }
        let session = self.session_store.get_session(session_id)?;
        let active_prompt = self
            .prompt_state_owner
            .active_prompt_for_agent(&session, target_agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        if active_prompt.status() == crate::session::PromptStatus::Cancelling {
            let session = self.session_snapshot(session_id)?;
            return Ok(crate::app::KernelPromptCancellation {
                cancellation: crate::session::PromptCancellation {
                    prompt: active_prompt,
                    started_next: None,
                },
                session,
                dispatch: None,
            });
        }
        let prompt = self
            .prompt_state_owner
            .begin_cancelling_active_prompt(&session, target_agent_id)
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let (active_prompt, queued_prompts) = self
            .prompt_state_owner
            .state_parts(&session, target_agent_id);
        self.mirror_prompt_owner_agent_state(
            session_id,
            target_agent_id,
            active_prompt,
            queued_prompts,
        )?;
        let worker_kernel_id = target_agent
            .remote_execution()
            .map(|remote| remote.worker_kernel_id.clone())
            .unwrap_or_else(|| "remote".to_string());
        self.record_notice(
            session_id,
            None,
            self.other_attachment_ids(session_id, attachment_id),
            format!(
                "Attachment `{attachment_id}` requested cancellation of active remote prompt `{}` on worker kernel `{}`.",
                prompt.id(),
                worker_kernel_id
            ),
        );
        let session = self.session_snapshot(session_id)?;
        Ok(crate::app::KernelPromptCancellation {
            cancellation: crate::session::PromptCancellation {
                prompt,
                started_next: None,
            },
            session,
            dispatch: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use super::*;
    use crate::agent::RemoteAgentBinding;
    use crate::app::{DaemonApp, KernelPreparedPromptSubmission, KernelSessionService};
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::session::{CreateSessionRequest, PromptQueueItem, PromptStatus};

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let (
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            history_store,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        ) = {
            let app_locked = app.lock().await;
            (
                app_locked.config_projection_store(),
                app_locked.session_state_store(),
                app_locked.agents().clone(),
                app_locked.attachments().clone(),
                app_locked.providers().clone(),
                app_locked.provider_process_tracking_store(),
                app_locked.slices(),
                app_locked.session_state_projection_store(),
                app_locked.provider_run_projection_store(),
                app_locked.history_store(),
                app_locked.operational_history_store(),
                app_locked.durable_state_store(),
                app_locked.prompt_state_owner(),
                app_locked.active_turn_store(),
                app_locked.prompt_activity_store(),
                app_locked.prompt_workspace_claim_store(),
                app_locked.structured_output_record_store(),
                app_locked.terminal_stream_store(),
                app_locked.workflow_design_event_store(),
                app_locked.metaagent_event_store(),
                app_locked.workspace_coordinator(),
            )
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            history_store,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }

    #[tokio::test]
    async fn remote_completion_with_queued_prompt_projects_combined_transition() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let attachment = KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "client-remote-queue",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        app.agents
            .bind_remote_execution(
                agent.id(),
                RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".to_string(),
                    worker_machine_id: "worker-machine-1".to_string(),
                    execution_lease_id: "lease-1".to_string(),
                    leased_agent_id: "leased-agent-1".to_string(),
                    active_worker_provider_run_id: Some("worker-run-1".to_string()),
                    relay_url: None,
                    relay_token: None,
                },
            )
            .expect("agent should bind to remote execution");
        let session_id = session.id().to_string();
        let agent_id = agent.id().to_string();
        let attachment_id = attachment.id().to_string();
        let projection_store = app.session_state_projection_store();
        let app = Arc::new(Mutex::new(app));
        let runtime = owned_runtime_state(&app).await;

        let first = PromptQueueItem::new(
            "pending:first",
            &attachment_id,
            &agent_id,
            "first remote prompt",
            PromptStatus::Queued,
        );
        let first_submission = runtime
            .owned
            .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: first,
                force_queue: false,
                refresh_projection: true,
            })
            .expect("first remote prompt should submit")
            .expect("remote prompt should be handled");
        let active_prompt_id = match first_submission.outcome {
            crate::session::PromptSubmissionOutcome::Started { prompt } => prompt.id().to_string(),
            crate::session::PromptSubmissionOutcome::Queued { .. } => {
                panic!("first remote prompt should start")
            }
        };
        let queued = PromptQueueItem::new(
            "queued:second",
            &attachment_id,
            &agent_id,
            "second remote prompt",
            PromptStatus::Queued,
        );
        let queued_submission = runtime
            .owned
            .submit_remote_prepared_prompt(&KernelPreparedPromptSubmission {
                session_id: session_id.clone(),
                prompt: queued,
                force_queue: false,
                refresh_projection: true,
            })
            .expect("second remote prompt should submit")
            .expect("remote prompt should be handled");
        let queued_prompt = match queued_submission.outcome {
            crate::session::PromptSubmissionOutcome::Queued { prompt } => prompt,
            crate::session::PromptSubmissionOutcome::Started { .. } => {
                panic!("second remote prompt should queue")
            }
        };
        let before_completion_sequence = projection_store.session_change_sequence(&session_id);

        let completion = runtime
            .owned
            .complete_remote_prompt_owner(
                &session_id,
                &agent_id,
                "worker-run-1",
                Some(&queued_prompt),
            )
            .expect("remote prompt completion should advance queue");

        assert_eq!(completion.completed.id(), active_prompt_id);
        let started_next = completion
            .started_next
            .expect("queued remote prompt should start");
        assert_ne!(started_next.id(), queued_prompt.id());
        assert_eq!(started_next.prompt(), queued_prompt.prompt());
        assert_eq!(
            projection_store.session_change_sequence(&session_id),
            before_completion_sequence + 2,
            "remote queued advancement should mirror once, then refresh the snapshot"
        );
        let projected = projection_store
            .get(&session_id)
            .expect("session projection should refresh");
        let active = projected
            .active_prompt_for_agent(&agent_id)
            .expect("next remote prompt should project as active");
        assert_eq!(active.id(), started_next.id());
        assert_eq!(active.prompt(), "second remote prompt");
        assert!(projected
            .queued_prompts_for_agent(&agent_id)
            .is_some_and(|queue| queue.is_empty()));
    }
}
