use super::*;

pub fn on_workflow_prompt_started(
    app: &mut DaemonApp,
    session_id: &str,
    prompt: &PromptQueueItem,
) -> Result<(), DaemonError> {
    let (Some(workflow_run_id), Some(workflow_node_run_id)) =
        (prompt.workflow_run_id(), prompt.workflow_node_run_id())
    else {
        return Ok(());
    };
    let workflow_run = app.sessions_mut().start_workflow_node_run(
        session_id,
        workflow_run_id,
        workflow_node_run_id,
    )?;
    app.record_notice(
        session_id,
        app.sessions()
            .get_session(session_id)?
            .active_provider_run_id(),
        app.attachments().list_session_attachment_ids(session_id),
        format!(
            "Workflow run `{}` started on agent `{}`.",
            workflow_run.id(),
            prompt.target_agent_id()
        ),
    );
    Ok(())
}

pub fn on_workflow_prompt_completed(
    app: &mut DaemonApp,
    session_id: &str,
    prompt: &PromptQueueItem,
    provider_run_id: Option<&str>,
) -> Result<(), DaemonError> {
    let (Some(workflow_run_id), Some(workflow_node_run_id)) =
        (prompt.workflow_run_id(), prompt.workflow_node_run_id())
    else {
        return Ok(());
    };
    let completion_snapshot = build_workflow_completion_snapshot(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        provider_run_id,
    );
    let has_valid_pending_final_output = workflow_node_run_has_valid_pending_final_output(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
    );
    if completion_snapshot.is_none() && !has_valid_pending_final_output {
        let provider_diagnostic =
            provider_run_id.and_then(|run_id| provider_run_terminal_diagnostic(app, run_id));
        if let Some(diagnostic) = provider_diagnostic {
            app.sessions_mut().fail_workflow_node_run(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            )?;
            record_and_route_workflow_failure(
                app,
                session_id,
                workflow_run_id,
                &WorkflowFailureEvent::new(
                    WorkflowFailureKind::ProviderFailure,
                    workflow_node_run_id,
                    Vec::new(),
                    diagnostic.clone(),
                ),
            );
            app.record_notice(
                session_id,
                provider_run_id,
                app.attachments().list_session_attachment_ids(session_id),
                format!(
                    "Workflow run `{workflow_run_id}` failed after provider turn failure: {diagnostic}"
                ),
            );
            maybe_start_next_queued_workflow_prompt(app, session_id);
            let _ = crate::app::KernelSessionReadService::new(app).session_snapshot(session_id);
            return Ok(());
        }
    }
    let max_turns = workflow_max_turns(app, session_id);
    let completion_result = {
        app.sessions_mut()
            .complete_workflow_node_run_after_provider_turn(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
                completion_snapshot.clone(),
                max_turns,
            )
    };
    let WorkflowCompletionUpdate {
        workflow_run,
        dispatches,
        validation_warnings,
        missing_output_failure,
        run_output_validation_failure,
    } = match completion_result {
        Ok(update) => update,
        Err(crate::error::DaemonError::WorkflowHandoffValidationFailed {
            edge_id,
            message,
            ..
        }) => {
            record_and_route_workflow_failure(
                app,
                session_id,
                workflow_run_id,
                &WorkflowFailureEvent::new(
                    WorkflowFailureKind::OutputValidationFailed,
                    workflow_node_run_id,
                    vec![edge_id.clone()],
                    message.clone(),
                ),
            );
            app.sessions_mut().stop_workflow_node_run(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            )?;
            app.record_notice(
                session_id,
                None,
                app.attachments().list_session_attachment_ids(session_id),
                format!(
                    "Workflow run `{workflow_run_id}` stopped after validation failed on edge `{edge_id}`: {message}"
                ),
            );
            maybe_start_next_queued_workflow_prompt(app, session_id);
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    if !validation_warnings.is_empty() {
        for warning in &validation_warnings {
            let failure = WorkflowFailureEvent::new(
                crate::session::classify_workflow_failure_kind(
                    &completion_snapshot,
                    &warning.message,
                ),
                workflow_node_run_id,
                vec![warning.edge_id.clone()],
                warning.message.clone(),
            );
            record_and_route_workflow_failure(app, session_id, workflow_run_id, &failure);
            app.record_notice(
                session_id,
                None,
                app.attachments().list_session_attachment_ids(session_id),
                format!(
                    "Workflow handoff validation warning on edge `{}`: {}",
                    warning.edge_id, warning.message
                ),
            );
        }
    }
    if workflow_run.status() == WorkflowRunStatus::Stopped
        && workflow_run.final_output().is_none()
        && workflow_run
            .failure_events()
            .iter()
            .all(|event| event.kind() != WorkflowFailureKind::NodeTurnBudgetExhausted)
    {
        record_and_route_workflow_failure(
            app,
            session_id,
            workflow_run_id,
            &WorkflowFailureEvent::new(
                WorkflowFailureKind::NodeTurnBudgetExhausted,
                workflow_node_run_id,
                Vec::new(),
                "workflow run stopped after a node exhausted its turn budget",
            ),
        );
    }
    if let Some(failure) = run_output_validation_failure.as_ref() {
        record_and_route_workflow_failure(
            app,
            session_id,
            workflow_run_id,
            &WorkflowFailureEvent::new(
                WorkflowFailureKind::WorkflowRunOutputValidationFailed,
                workflow_node_run_id,
                Vec::new(),
                failure.message.clone(),
            ),
        );
        app.record_notice(
            session_id,
            provider_run_id,
            app.attachments().list_session_attachment_ids(session_id),
            if failure.retry_scheduled {
                format!(
                    "Workflow run `{workflow_run_id}` final output failed validation on attempt {}/{}; a corrective turn was scheduled: {}",
                    failure.attempt, failure.max_attempts, failure.message
                )
            } else {
                format!(
                    "Workflow run `{workflow_run_id}` failed final output validation after attempt {}/{}: {}",
                    failure.attempt, failure.max_attempts, failure.message
                )
            },
        );
    }
    if let Some(failure) = missing_output_failure.as_ref() {
        record_and_route_workflow_failure(
            app,
            session_id,
            workflow_run_id,
            &WorkflowFailureEvent::new(
                WorkflowFailureKind::MissingStructuredOutput,
                workflow_node_run_id,
                Vec::new(),
                failure.message.clone(),
            ),
        );
        app.record_notice(
            session_id,
            provider_run_id,
            app.attachments().list_session_attachment_ids(session_id),
            if failure.retry_scheduled {
                format!(
                    "Workflow run `{workflow_run_id}` produced no structured output on attempt {}/{}; a corrective turn was scheduled.",
                    failure.attempt, failure.max_attempts
                )
            } else {
                format!(
                    "Workflow run `{workflow_run_id}` failed after producing no structured output on attempt {}/{}.",
                    failure.attempt, failure.max_attempts
                )
            },
        );
    }
    if validation_warnings.is_empty()
        && missing_output_failure.is_none()
        && run_output_validation_failure.is_none()
    {
        let updated = app.sessions_mut().mark_workflow_turn_validated_completed(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        if updated
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .and_then(|node_run| node_run.turn_envelope())
            .is_some_and(|envelope| {
                envelope.state() == crate::session::WorkflowTurnRuntimeState::ValidatedCompleted
            })
        {
            clear_workflow_control_mailbox(
                app,
                session_id,
                workflow_run_id,
                workflow_node_run_id,
                &updated,
            );
        }
    }
    let claim_provider_run_id = provider_run_id.map(str::to_string).or_else(|| {
        app.providers()
            .get_run_for_agent(session_id, prompt.target_agent_id())
            .map(|run| run.id().to_string())
    });
    let released_claim = claim_provider_run_id
        .as_deref()
        .map(|provider_run_id| app.release_prompt_workspace_claim(provider_run_id))
        .unwrap_or(false);
    let released_workflow_claim = app.release_workflow_node_workspace_claim(
        session_id,
        workflow_run_id,
        workflow_node_run_id,
    );
    schedule_workflow_dispatches(app, session_id, workflow_run.id(), &dispatches);
    let workflow_run = current_workflow_run_for_notice(app, session_id, workflow_run);
    if released_claim || released_workflow_claim {
        let _ = retry_blocked_workflow_claims(app);
    }
    let state_suffix = workflow_run_status_notice_suffix(workflow_run.status());
    app.record_notice(
        session_id,
        None,
        app.attachments().list_session_attachment_ids(session_id),
        format!("Workflow run `{}` {state_suffix}.", workflow_run.id()),
    );
    if matches!(
        workflow_run.status(),
        WorkflowRunStatus::Completed | WorkflowRunStatus::Failed | WorkflowRunStatus::Stopped
    ) {
        maybe_start_next_queued_workflow_prompt(app, session_id);
    }
    Ok(())
}

pub(super) fn current_workflow_run_for_notice(
    app: &DaemonApp,
    session_id: &str,
    workflow_run: WorkflowRun,
) -> WorkflowRun {
    app.sessions()
        .resolve_workflow_run_ref(session_id, workflow_run.id())
        .unwrap_or(workflow_run)
}

pub(super) fn workflow_run_status_notice_suffix(status: WorkflowRunStatus) -> &'static str {
    match status {
        WorkflowRunStatus::Waiting => "waiting for downstream handoffs",
        WorkflowRunStatus::Completing => "is completing",
        WorkflowRunStatus::Completed => "completed",
        WorkflowRunStatus::Failed => "failed",
        WorkflowRunStatus::Stopped => "stopped",
        _ => "updated",
    }
}

pub fn on_workflow_provider_failure(
    app: &mut DaemonApp,
    session_id: &str,
    prompt: &PromptQueueItem,
    provider_run_id: Option<&str>,
    message: &str,
) -> Result<(), DaemonError> {
    let (Some(workflow_run_id), Some(workflow_node_run_id)) =
        (prompt.workflow_run_id(), prompt.workflow_node_run_id())
    else {
        return Ok(());
    };
    record_and_route_workflow_failure(
        app,
        session_id,
        workflow_run_id,
        &WorkflowFailureEvent::new(
            WorkflowFailureKind::ProviderFailure,
            workflow_node_run_id,
            Vec::new(),
            message,
        ),
    );
    app.sessions_mut()
        .fail_workflow_node_run(session_id, workflow_run_id, workflow_node_run_id)?;
    app.record_notice(
        session_id,
        provider_run_id,
        app.attachments().list_session_attachment_ids(session_id),
        format!("Workflow run `{workflow_run_id}` failed after provider turn failure: {message}"),
    );
    maybe_start_next_queued_workflow_prompt(app, session_id);
    let _ = crate::app::KernelSessionReadService::new(app).session_snapshot(session_id);
    Ok(())
}

pub fn on_workflow_prompt_cancelled(
    app: &mut DaemonApp,
    session_id: &str,
    prompt: &PromptQueueItem,
) -> Result<(), DaemonError> {
    let (Some(workflow_run_id), Some(workflow_node_run_id)) =
        (prompt.workflow_run_id(), prompt.workflow_node_run_id())
    else {
        return Ok(());
    };
    let workflow_run = app.sessions_mut().stop_workflow_node_run(
        session_id,
        workflow_run_id,
        workflow_node_run_id,
    )?;
    record_and_route_workflow_failure(
        app,
        session_id,
        workflow_run_id,
        &WorkflowFailureEvent::new(
            WorkflowFailureKind::RunStopped,
            workflow_node_run_id,
            Vec::new(),
            "workflow node run was stopped before validated completion",
        ),
    );
    app.record_notice(
        session_id,
        None,
        app.attachments().list_session_attachment_ids(session_id),
        format!("Workflow run `{}` was stopped.", workflow_run.id()),
    );
    maybe_start_next_queued_workflow_prompt(app, session_id);
    Ok(())
}

fn workflow_node_run_has_valid_pending_final_output(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
) -> bool {
    app.sessions()
        .get_session(session_id)
        .ok()
        .and_then(|session| {
            session
                .workflow_run(workflow_run_id)
                .and_then(|workflow_run| {
                    workflow_run
                        .node_runs()
                        .iter()
                        .find(|node_run| node_run.id() == workflow_node_run_id)
                })
                .map(|node_run| node_run.has_valid_pending_final_output())
        })
        .unwrap_or(false)
}

fn maybe_start_next_queued_workflow_prompt(app: &mut DaemonApp, session_id: &str) {
    match app.start_next_queued_workflow_prompt(session_id) {
        Ok(Some(crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
            workflow_run,
            workflow,
            endpoint,
        })) => {
            app.record_notice(
                session_id,
                None,
                app.attachments().list_session_attachment_ids(session_id),
                format!(
                    "Started queued workflow run `{}` for workflow `{}` endpoint `{}`.",
                    workflow_run.id(),
                    workflow.id(),
                    endpoint.id()
                ),
            );
        }
        Ok(Some(crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued { .. })) => {}
        Ok(None) => {}
        Err(error) => {
            app.record_notice(
                session_id,
                None,
                app.attachments().list_session_attachment_ids(session_id),
                format!("Failed to start queued workflow prompt: {error}"),
            );
        }
    }
}
