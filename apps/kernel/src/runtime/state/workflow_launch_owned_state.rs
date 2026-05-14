//! Workflow launch, queued-start, and resume state transitions.
//!
//! This module owns entry-node scheduling and manual/queued workflow-run admission. Completion,
//! failure, and node fan-out remain in `workflow_dispatch`.

use super::*;

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_schedule_entry_node(
        &self,
        session_id: &str,
        workflow_run: &crate::session::WorkflowRun,
    ) -> Result<WorkflowPromptDispatches, DaemonError> {
        let endpoint_prompt = workflow_run
            .invocation_prompt()
            .map(str::trim)
            .unwrap_or("");
        let node_run = workflow_run.node_runs().first().ok_or_else(|| {
            DaemonError::InvalidWorkflowGraphReference {
                session_id: session_id.to_string(),
                workflow_id: workflow_run.workflow_id().to_string(),
                reference: workflow_run.id().to_string(),
                message: "workflow run has no entry node run",
            }
        })?;
        let prompt_text = self.workflow_turn_prompt_text(
            session_id,
            workflow_run.id(),
            node_run.id(),
            node_run.node_id(),
            endpoint_prompt,
            None,
            None,
        )?;
        let _ = self.session_store.write().prepare_workflow_turn(
            session_id,
            workflow_run.id(),
            node_run.id(),
            format!("workflow-ack:{}", node_run.id()),
            prompt_text.clone(),
            None,
            None,
        )?;
        let claim_id = self.workflow_dispatch_claim_id(session_id, node_run.agent_id())?;
        match self.acquire_workflow_node_workspace_claim(
            session_id,
            &claim_id,
            node_run.agent_id(),
            workflow_run.id(),
            node_run.id(),
        ) {
            Ok(()) => {}
            Err(error @ DaemonError::WorkspaceClaimConflict { .. }) => {
                let _ = self
                    .session_store
                    .write()
                    .block_workflow_node_on_workspace_claim(
                        session_id,
                        workflow_run.id(),
                        node_run.id(),
                    );
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Workflow run `{}` blocked node `{}` on a workspace claim: {error}",
                        workflow_run.id(),
                        node_run.node_id()
                    ),
                );
                let _ = self.session_snapshot(session_id)?;
                return Ok(WorkflowPromptDispatches::default());
            }
            Err(error) => return Err(error),
        }
        let _ = self
            .session_store
            .write()
            .ready_workflow_node_after_workspace_claim(
                session_id,
                workflow_run.id(),
                node_run.id(),
            );
        let prompt = crate::session::PromptQueueItem::new(
            self.session_store.reserve_prompt_id(),
            crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
            node_run.agent_id(),
            prompt_text,
            crate::session::PromptStatus::Queued,
        )
        .with_workflow_context(workflow_run.id(), node_run.id());
        self.workflow_submit_prepared_prompt(
            crate::app::KernelPreparedPromptSubmission {
                session_id: session_id.to_string(),
                prompt,
                force_queue: false,
            },
            workflow_run.id(),
            node_run.id(),
        )
    }

    pub(super) fn workflow_invoke_queued_launch(
        &self,
        session_id: &str,
        queued_launch: crate::session::QueuedWorkflowLaunch,
    ) -> Result<
        (
            crate::app::workflow_runtime::WorkflowLaunchOutcome,
            WorkflowPromptDispatches,
        ),
        DaemonError,
    > {
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, queued_launch.workflow_id())?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            session_id,
            queued_launch.workflow_id(),
            queued_launch.endpoint_id(),
        )?;
        self.workflow_validate_agents(session_id, &workflow)?;
        self.workflow_flush_agent_context_if_needed(session_id, &workflow)?;
        let workflow_run = self.session_store.write().invoke_workflow_endpoint(
            session_id,
            workflow.id(),
            endpoint.id(),
            queued_launch.invocation_prompt().map(str::to_string),
        )?;
        let dispatches = self.workflow_schedule_entry_node(session_id, &workflow_run)?;
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        if let Some(watchdog_id) = queued_launch.watchdog_id() {
            let _ = self.session_store.write().mark_workflow_watchdog_invoked(
                session_id,
                watchdog_id,
                workflow_run.id(),
            );
        }
        Ok((
            crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                workflow_run,
                workflow,
                endpoint,
            },
            dispatches,
        ))
    }

    pub(super) fn workflow_invoke_endpoint_with_admission(
        &self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        prompt: Option<String>,
    ) -> Result<
        (
            crate::app::workflow_runtime::WorkflowLaunchOutcome,
            WorkflowPromptDispatches,
        ),
        DaemonError,
    > {
        let workflow = self
            .session_store
            .read()
            .resolve_workflow_ref(session_id, workflow_ref)?;
        let endpoint = self.session_store.read().resolve_workflow_endpoint_ref(
            session_id,
            workflow_ref,
            endpoint_ref,
        )?;
        self.workflow_validate_agents(session_id, &workflow)?;
        let admission = self.session_store.write().admit_manual_workflow_launch(
            session_id,
            workflow.id(),
            endpoint.id(),
            prompt.clone(),
        )?;
        match admission {
            crate::session::WorkflowLaunchAdmission::StartNow => {
                self.workflow_flush_agent_context_if_needed(session_id, &workflow)?;
                let workflow_run = self.session_store.write().invoke_workflow_endpoint(
                    session_id,
                    workflow.id(),
                    endpoint.id(),
                    prompt,
                )?;
                let dispatches = self.workflow_schedule_entry_node(session_id, &workflow_run)?;
                let workflow_run = self
                    .session_store
                    .read()
                    .resolve_workflow_run_ref(session_id, workflow_run.id())?;
                Ok((
                    crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                        workflow_run,
                        workflow,
                        endpoint,
                    },
                    dispatches,
                ))
            }
            crate::session::WorkflowLaunchAdmission::Queued(queued_launch) => Ok((
                crate::app::workflow_runtime::WorkflowLaunchOutcome::Queued {
                    queued_launch,
                    workflow,
                    endpoint,
                },
                WorkflowPromptDispatches::default(),
            )),
        }
    }

    pub(super) fn workflow_maybe_start_next_queued_launch(&self, session_id: &str) {
        let queued_launch = match self
            .session_store
            .write()
            .dequeue_next_workflow_launch(session_id)
        {
            Ok(Some(queued_launch)) => queued_launch,
            Ok(None) => return,
            Err(error) => {
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!("Failed to start queued workflow launch: {error}"),
                );
                return;
            }
        };
        if let Some(watchdog_id) = queued_launch.watchdog_id() {
            let _ = self
                .session_store
                .write()
                .mark_workflow_watchdog_pending_started(session_id, watchdog_id);
        }
        match self.workflow_invoke_queued_launch(session_id, queued_launch.clone()) {
            Ok((
                crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                    workflow_run,
                    workflow,
                    endpoint,
                },
                _dispatches,
            )) => {
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Started queued workflow run `{}` for workflow `{}` endpoint `{}`.",
                        workflow_run.id(),
                        workflow.id(),
                        endpoint.id()
                    ),
                );
            }
            Ok((crate::app::workflow_runtime::WorkflowLaunchOutcome::Queued { .. }, _)) => {}
            Err(error) => {
                if let Some(watchdog_id) = queued_launch.watchdog_id() {
                    let _ = self.session_store.write().mark_workflow_watchdog_failed(
                        session_id,
                        watchdog_id,
                        error.to_string(),
                    );
                }
                self.record_notice(
                    session_id,
                    None,
                    self.attachment_store
                        .list_session_attachment_ids(session_id),
                    format!(
                        "Queued workflow launch `{}` failed: {error}",
                        queued_launch.id()
                    ),
                );
            }
        }
    }

    pub(super) fn workflow_resume_run(
        &self,
        session_id: &str,
        workflow_run_ref: &str,
    ) -> Result<(crate::session::WorkflowRun, WorkflowPromptDispatches), DaemonError> {
        let workflow_run = self
            .session_store
            .write()
            .resume_workflow_run(session_id, workflow_run_ref)?;
        let resumable = workflow_run
            .node_runs()
            .iter()
            .filter_map(|node_run| {
                let prompt = node_run.turn_envelope()?.rendered_prompt()?.to_string();
                Some((
                    node_run.id().to_string(),
                    node_run.agent_id().to_string(),
                    prompt,
                ))
            })
            .collect::<Vec<_>>();
        let mut dispatches = WorkflowPromptDispatches::default();
        for (workflow_node_run_id, agent_id, prompt_text) in resumable {
            let prompt = crate::session::PromptQueueItem::new(
                self.session_store.reserve_prompt_id(),
                crate::scheduler::runtime::workflow_prompt_source_attachment_id(workflow_run.id()),
                agent_id,
                prompt_text,
                crate::session::PromptStatus::Queued,
            )
            .with_workflow_context(workflow_run.id(), &workflow_node_run_id);
            match self.workflow_submit_prepared_prompt(
                crate::app::KernelPreparedPromptSubmission {
                    session_id: session_id.to_string(),
                    prompt,
                    force_queue: false,
                },
                workflow_run.id(),
                &workflow_node_run_id,
            ) {
                Ok(prepared) => dispatches.extend(prepared),
                Err(error) => {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachment_store
                            .list_session_attachment_ids(session_id),
                        format!(
                            "Workflow run `{}` could not resume node prompt: {}",
                            workflow_run.id(),
                            error
                        ),
                    );
                }
            }
        }
        let workflow_run = self
            .session_store
            .read()
            .resolve_workflow_run_ref(session_id, workflow_run.id())?;
        Ok((workflow_run, dispatches))
    }
}
