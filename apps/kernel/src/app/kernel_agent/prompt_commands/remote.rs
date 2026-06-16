use crate::agent::RemoteAgentBinding;
use crate::app::{DaemonApp, KernelRemotePromptDispatch};
use crate::error::DaemonError;
use crate::session::{PromptCancellation, PromptCompletion, PromptQueueItem};
use crate::transport::relay_client::{
    send_peer_request_via_temporary_connection,
    send_peer_request_via_temporary_connection_with_timeout,
};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use arroba_relay::protocol::ClientTarget;

use super::super::KernelAgentService;
use super::completion::{KernelPromptCompletionAdmission, KernelPromptOwnerCompletion};

fn remote_workspace_live_sync_mode_for_agent(
    app: &DaemonApp,
    session_id: &str,
    agent_id: &str,
) -> Option<crate::config::WorkspaceLiveSyncMode> {
    let session = app.sessions().get_session(session_id).ok()?;
    let agent = app.agents().get_agent(agent_id).ok()?;
    Some(
        crate::provider::provider_workspace_live_sync_mode_for_session(
            agent.provider(),
            app.config(),
            Some(&session),
        ),
    )
}

fn remote_git_turn_context(
    dispatch: &KernelRemotePromptDispatch,
) -> crate::transport::relay_peer::RemoteGitTurnContext {
    crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: dispatch.session_id.clone(),
        home_agent_id: dispatch.agent_id.clone(),
        home_prompt_id: dispatch.prompt_id.clone(),
        home_turn_id: dispatch.prompt_id.clone(),
        workspace_live_sync_mode: dispatch.workspace_live_sync_mode,
        prompt_summary: crate::prompt_transcript::render_prompt_transcript(
            &dispatch.prompt,
            &dispatch.attachments,
        ),
    }
}

fn remote_dispatch_relay_config(
    app: &DaemonApp,
    dispatch: &KernelRemotePromptDispatch,
) -> crate::config::DaemonConfig {
    let mut config = app.config().clone();
    if let (Some(relay_url), Some(relay_token)) =
        (dispatch.relay_url.clone(), dispatch.relay_token.clone())
    {
        config.apply_remote_relay_override(relay_url, relay_token);
    }
    config
}

fn remote_git_turn_context_for_prompt(
    app: &DaemonApp,
    session_id: &str,
    agent_id: &str,
    prompt: &PromptQueueItem,
) -> crate::transport::relay_peer::RemoteGitTurnContext {
    crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: session_id.to_string(),
        home_agent_id: agent_id.to_string(),
        home_prompt_id: prompt.id().to_string(),
        home_turn_id: prompt.id().to_string(),
        workspace_live_sync_mode: remote_workspace_live_sync_mode_for_agent(
            app, session_id, agent_id,
        ),
        prompt_summary: crate::prompt_transcript::render_prompt_transcript(
            prompt.prompt(),
            prompt.attachments(),
        ),
    }
}

impl<'a> KernelAgentService<'a> {
    pub(super) fn cancel_remote_active_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
        attachment_id: Option<&str>,
        active_prompt: &PromptQueueItem,
        remote_execution: RemoteAgentBinding,
    ) -> Result<PromptCancellation, DaemonError> {
        let relay_config = self
            .app
            .relay_config_for_remote_execution(&remote_execution);
        match self
            .app
            .block_on_relay_future(send_peer_request_via_temporary_connection(
                &relay_config,
                ClientTarget {
                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                    daemon_alias: None,
                },
                RelayPeerRequest::CancelLeasedPrompt {
                    leased_agent_id: remote_execution.leased_agent_id.clone(),
                },
            ))? {
            RelayPeerResponse::LeasedPromptCancelled { .. } => {}
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "cancel remote prompt",
                    message: format!("unexpected remote prompt cancellation response: {other:?}"),
                });
            }
        }
        let prompt = self
            .app
            .prompt_owner_begin_cancelling_active_prompt(session_id, agent_id)?;
        let recipients = match attachment_id {
            Some(attachment_id) => self.app.other_attachment_ids(session_id, attachment_id),
            None => self.app.attachments.list_session_attachment_ids(session_id),
        };
        let message = match attachment_id {
            Some(attachment_id) => format!(
                "Attachment `{attachment_id}` requested cancellation of active remote prompt `{}` on worker kernel `{}`.",
                active_prompt.id(),
                remote_execution.worker_kernel_id
            ),
            None => format!(
                "Arroba requested cancellation of active remote prompt `{}` on worker kernel `{}`.",
                active_prompt.id(),
                remote_execution.worker_kernel_id
            ),
        };
        self.app
            .record_notice(session_id, None, recipients, message);
        crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id)?;
        Ok(PromptCancellation {
            prompt,
            started_next: None,
        })
    }

    pub(super) fn finish_compat_remote_prompt_dispatch(
        &mut self,
        dispatch: Option<KernelRemotePromptDispatch>,
    ) -> Result<(), DaemonError> {
        let Some(dispatch) = dispatch else {
            return Ok(());
        };
        let attachments = self
            .app
            .serialize_remote_prompt_attachments(&dispatch.attachments)?;
        let agent = self.app.agents().get_agent(&dispatch.agent_id)?;
        let remote_extension_manifest = self.app.remote_extension_manifest_for_agent(&agent)?;
        let relay_config = remote_dispatch_relay_config(self.app, &dispatch);
        let result = match self.app.block_on_relay_future(
            send_peer_request_via_temporary_connection_with_timeout(
                &relay_config,
                ClientTarget {
                    daemon_id: Some(dispatch.worker_kernel_id.clone()),
                    daemon_alias: None,
                },
                RelayPeerRequest::SubmitLeasedPrompt {
                    leased_agent_id: dispatch.leased_agent_id.clone(),
                    prompt: dispatch.prompt.clone(),
                    attachments,
                    workflow_context: dispatch.workflow_context.clone(),
                    git_context: Some(remote_git_turn_context(&dispatch)),
                    required_mcps: Vec::new(),
                    remote_extension_manifest,
                },
                crate::transport::relay_client::LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
            ),
        ) {
            Ok(RelayPeerResponse::LeasedPromptSubmitted {
                provider_run_id, ..
            }) => Ok(provider_run_id),
            Ok(other) => Err(DaemonError::LocalTransport {
                operation: "submit remote prepared prompt",
                message: format!("unexpected remote prompt response: {other:?}"),
            }),
            Err(error) => Err(error),
        };
        self.app
            .finish_kernel_remote_prompt_dispatch(dispatch, result)
    }

    pub(super) fn complete_remote_prompt_from_admission(
        &mut self,
        admission: KernelPromptCompletionAdmission,
    ) -> Result<KernelPromptOwnerCompletion, DaemonError> {
        let KernelPromptCompletionAdmission::Remote {
            session_id,
            agent_id,
            remote_execution,
            next_queued_prompt,
        } = admission
        else {
            return Err(DaemonError::LocalTransport {
                operation: "complete prompt admission",
                message: "expected remote prompt completion admission".to_string(),
            });
        };

        let relay_config = self
            .app
            .relay_config_for_remote_execution(&remote_execution);
        let remote_provider_run_id =
            match self
                .app
                .block_on_relay_future(send_peer_request_via_temporary_connection(
                    &relay_config,
                    ClientTarget {
                        daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::CompleteLeasedPrompt {
                        leased_agent_id: remote_execution.leased_agent_id.clone(),
                    },
                ))? {
                RelayPeerResponse::LeasedPromptCompleted {
                    provider_run_id,
                    git_observations,
                    workspace_live_sync_change,
                    ..
                } => {
                    let _ = crate::git_observer::append_observations(
                        &self.app.operational_history_store(),
                        git_observations,
                    )?;
                    if let Some(change) = workspace_live_sync_change {
                        self.app.fanout_remote_workspace_live_sync_change(
                            change,
                            Some(&remote_execution.worker_kernel_id),
                        );
                    }
                    provider_run_id
                }
                other => {
                    return Err(DaemonError::LocalTransport {
                        operation: "complete remote prompt",
                        message: format!("unexpected remote prompt completion response: {other:?}"),
                    });
                }
            };
        let completed = self
            .app
            .prompt_owner_complete_active_prompt_only(&session_id, &agent_id)?;
        let _ = self
            .app
            .agents()
            .set_remote_execution_active_worker_provider_run_id(&agent_id, None)?;
        Ok(KernelPromptOwnerCompletion {
            session_id,
            agent_id,
            completed,
            provider_run_id: None,
            remote_execution: Some(remote_execution),
            remote_provider_run_id,
            next_queued_prompt,
        })
    }

    pub(super) fn finish_remote_prompt_completion(
        &mut self,
        completion: KernelPromptOwnerCompletion,
    ) -> Result<PromptCompletion, DaemonError> {
        let remote_provider_run_id = completion
            .remote_provider_run_id
            .as_deref()
            .unwrap_or("remote-provider-run-completed");
        let recipient_attachment_ids = self
            .app
            .attachments
            .list_session_attachment_ids(&completion.session_id);
        self.record_assistant_message_completion(
            &completion.session_id,
            remote_provider_run_id,
            recipient_attachment_ids,
            &format!("prompt-complete:{}", completion.completed.id()),
            crate::session::unix_epoch_ms(),
        );
        let started_next = if self
            .app
            .prompt_owner_active_prompt_for_agent(&completion.session_id, &completion.agent_id)?
            .is_none()
        {
            let remote_execution = completion.remote_execution.as_ref().ok_or_else(|| {
                DaemonError::LocalTransport {
                    operation: "complete remote prompt",
                    message: "missing remote execution binding".to_string(),
                }
            })?;
            self.advance_next_queued_prompt_remote(
                &completion.session_id,
                &completion.agent_id,
                &remote_execution.worker_kernel_id,
                &remote_execution.leased_agent_id,
                remote_execution.relay_url.as_deref(),
                remote_execution.relay_token.as_deref(),
                completion.next_queued_prompt.as_ref(),
            )?
        } else {
            None
        };
        if started_next.is_none() {
            self.app
                .sync_focused_provider_run_if_idle(&completion.session_id)?;
        }
        crate::app::KernelSessionReadService::new(self.app)
            .session_snapshot(&completion.session_id)?;

        let prompt_completion = PromptCompletion {
            completed: completion.completed,
            started_next,
        };
        self.inject_orphaned_metaagent_task_event_after_turn(
            &completion.agent_id,
            &prompt_completion,
        )?;
        Ok(prompt_completion)
    }

    pub(crate) fn advance_next_queued_prompt_remote(
        &mut self,
        session_id: &str,
        agent_id: &str,
        worker_kernel_id: &str,
        leased_agent_id: &str,
        relay_url: Option<&str>,
        relay_token: Option<&str>,
        expected_next: Option<&PromptQueueItem>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        let mut relay_config = self.app.config().clone();
        if let (Some(relay_url), Some(relay_token)) = (relay_url, relay_token) {
            relay_config
                .apply_remote_relay_override(relay_url.to_string(), relay_token.to_string());
        }
        loop {
            let next_candidate =
                self.next_queued_prompt_candidate(session_id, agent_id, expected_next)?;
            let Some(peeked) = next_candidate else {
                return Ok(None);
            };
            let is_workflow_prompt = crate::app::workflow_runtime::is_workflow_prompt_source(
                peeked.source_attachment_id(),
            );
            if let Err(error) = crate::app::KernelSessionReadService::new(self.app)
                .ensure_attachment_in_session(session_id, peeked.source_attachment_id())
            {
                if !is_workflow_prompt {
                    self.app.record_notice(
                        session_id,
                        None,
                        self.app.attachments.list_session_attachment_ids(session_id),
                        format!(
                            "Skipped queued prompt `{}` because its source attachment is no longer active: {}",
                            peeked.id(),
                            error
                        ),
                    );
                    let _ = self.activate_next_queued_prompt_for_mirror(
                        session_id,
                        agent_id,
                        expected_next,
                    )?;
                    continue;
                }
            }
            let agent = self.app.agents().get_agent(agent_id)?;
            let remote_extension_manifest = self.app.remote_extension_manifest_for_agent(&agent)?;
            let response = self.app.block_on_relay_future(
                send_peer_request_via_temporary_connection_with_timeout(
                    &relay_config,
                    ClientTarget {
                        daemon_id: Some(worker_kernel_id.to_string()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::SubmitLeasedPrompt {
                        leased_agent_id: leased_agent_id.to_string(),
                        prompt: peeked.prompt().to_string(),
                        attachments: self
                            .app
                            .serialize_remote_prompt_attachments(peeked.attachments())?,
                        workflow_context: if is_workflow_prompt {
                            Some(
                                crate::app::RemoteWorkflowTurnContextResolver::new(self.app)
                                    .remote_workflow_turn_context_for_prompt(
                                        session_id, agent_id, &peeked,
                                    )?,
                            )
                        } else {
                            None
                        },
                        git_context: Some(remote_git_turn_context_for_prompt(
                            self.app, session_id, agent_id, &peeked,
                        )),
                        required_mcps: Vec::new(),
                        remote_extension_manifest,
                    },
                    crate::transport::relay_client::LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
                ),
            );
            let remote_provider_run_id = match response {
                Ok(RelayPeerResponse::LeasedPromptSubmitted {
                    provider_run_id, ..
                }) => provider_run_id,
                Ok(other) => {
                    return Err(DaemonError::LocalTransport {
                        operation: "advance remote queued prompt",
                        message: format!("unexpected remote prompt response: {other:?}"),
                    });
                }
                Err(error) => return Err(error),
            };
            let (_session, next_candidate) =
                self.activate_next_queued_prompt_for_mirror(session_id, agent_id, expected_next)?;
            let Some(active) = next_candidate else {
                continue;
            };
            self.app.echo_prompt_to_other_attachments(
                session_id,
                &remote_provider_run_id,
                active.source_attachment_id(),
                active.prompt(),
                active.attachments(),
            );
            if let (Some(workflow_run_id), Some(workflow_node_run_id)) =
                (active.workflow_run_id(), active.workflow_node_run_id())
            {
                self.app.sessions_mut().mark_workflow_turn_dispatched(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                )?;
            }
            crate::app::workflow_runtime::start_workflow_prompt_from_runtime(
                self.app, session_id, &active,
            )?;
            crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id)?;
            return Ok(Some(active));
        }
    }
}
