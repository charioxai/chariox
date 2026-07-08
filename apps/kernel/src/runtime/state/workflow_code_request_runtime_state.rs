use super::workflow_code_request_support::*;
use super::workflow_request_runtime_state::workflow_response_session;
use super::*;

impl KernelRuntimeState {
    pub(super) async fn execute_workflow_code_validate_request(
        &self,
        request: crate::local::ValidateWorkflowCodeRequest,
        caller_metaagent_id: Option<&str>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let caller_metaagent_id = caller_metaagent_id.map(str::to_string);
        self.with_app_side_effect(move |app| {
            let limits = app.config().workflow_code_limits();
            let result = crate::app::KernelSessionService::new(app)
                .compile_and_validate_workflow_code_source_with_rebindings(
                    &request.session_id,
                    &request.node_path,
                    &request.source,
                    request
                        .language
                        .unwrap_or(crate::workflow_code::WorkflowCodeLanguage::JavaScript),
                    &limits,
                    &request.provider_rebindings,
                    &request.agent_rebindings,
                    caller_metaagent_id.as_deref(),
                )?;
            Ok(LocalDaemonResponse::WorkflowCodeValidated { result })
        })
        .await
    }

    pub(super) async fn execute_workflow_code_apply_request(
        &self,
        request: crate::local::ApplyWorkflowCodeRequest,
        caller_user_id: &str,
        caller_metaagent_id: Option<&str>,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let caller_user_id = caller_user_id.to_string();
        let controlled_by_metaagent_id = caller_metaagent_id.map(str::to_string);
        let session_id = request.session_id.clone();
        let result = self
            .with_app_side_effect(move |app| {
                let limits = app.config().workflow_code_limits();
                let result = app.compile_and_apply_workflow_code_source_with_rebindings(
                    &request.session_id,
                    &request.node_path,
                    &request.source,
                    request
                        .language
                        .unwrap_or(crate::workflow_code::WorkflowCodeLanguage::JavaScript),
                    &limits,
                    caller_user_id,
                    controlled_by_metaagent_id,
                    &request.provider_rebindings,
                    &request.agent_rebindings,
                )?;
                let session =
                    crate::app::KernelSessionReadService::new(app).session_snapshot(&session_id)?;
                Ok(LocalDaemonResponse::WorkflowCodeApplied { result, session })
            })
            .await;
        let session = result.as_ref().ok().and_then(workflow_response_session);
        (result, session)
    }

    pub(super) async fn execute_workflow_code_artifact_apply_request(
        &self,
        request: crate::local::ApplyWorkflowCodeArtifactRequest,
        caller_user_id: &str,
        caller_metaagent_id: Option<&str>,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let caller_user_id = caller_user_id.to_string();
        let controlled_by_metaagent_id = caller_metaagent_id.map(str::to_string);
        let session_id = request.session_id.clone();
        let result = self
            .with_app_side_effect(move |app| {
                let result = workflow_code_artifact_apply_result(
                    app,
                    &request.session_id,
                    &request.name,
                    &request.provider_rebindings,
                    &request.agent_rebindings,
                    caller_user_id,
                    controlled_by_metaagent_id,
                    crate::workflow_code::WorkflowCodeArtifactHistoryAction::Applied,
                    "workflow_code_artifact.apply",
                    None,
                    None,
                )?;
                let session =
                    crate::app::KernelSessionReadService::new(app).session_snapshot(&session_id)?;
                Ok(LocalDaemonResponse::WorkflowCodeApplied { result, session })
            })
            .await;
        let session = result.as_ref().ok().and_then(workflow_response_session);
        (result, session)
    }

    pub(super) async fn execute_workflow_code_run_request(
        &self,
        request: crate::local::RunWorkflowCodeRequest,
        caller_user_id: &str,
        caller_metaagent_id: Option<&str>,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let caller_user_id = caller_user_id.to_string();
        let controlled_by_metaagent_id = caller_metaagent_id.map(str::to_string);
        let session_id = request.session_id.clone();
        let apply_result = match self
            .with_app_side_effect({
                let session_id = session_id.clone();
                let node_path = request.node_path.clone();
                let source = request.source.clone();
                let endpoint = request.endpoint.clone();
                let queue_ref = request.queue_ref.clone();
                let language = request
                    .language
                    .unwrap_or(crate::workflow_code::WorkflowCodeLanguage::JavaScript);
                let provider_rebindings = request.provider_rebindings.clone();
                let agent_rebindings = request.agent_rebindings.clone();
                let caller_user_id = caller_user_id.clone();
                move |app| {
                    let limits = app.config().workflow_code_limits();
                    let compile = crate::app::KernelSessionService::new(app)
                        .compile_and_validate_workflow_code_source_with_rebindings(
                            &session_id,
                            &node_path,
                            &source,
                            language,
                            &limits,
                            &provider_rebindings,
                            &agent_rebindings,
                            controlled_by_metaagent_id.as_deref(),
                        )?;
                    reject_invalid_workflow_code_run_compile(
                        "workflow_code.run",
                        &compile.validation,
                    )?;
                    workflow_code_run_endpoint_preflight(
                        &compile.definition,
                        endpoint.as_deref(),
                        "workflow_code.run",
                    )?;
                    workflow_code_run_queue_preflight(
                        &compile.definition,
                        queue_ref.as_deref(),
                        "workflow_code.run",
                    )?;
                    let apply = crate::app::KernelSessionService::new(app)
                        .apply_workflow_code_definition(
                            &session_id,
                            &compile.definition,
                            &limits,
                            caller_user_id,
                            controlled_by_metaagent_id,
                        )?;
                    Ok(crate::workflow_code::WorkflowCodeCompileAndApplyResult { compile, apply })
                }
            })
            .await
        {
            Ok(result) => result,
            Err(error) => return (Err(error), None),
        };
        let endpoint_ref = match workflow_code_endpoint_ref(&apply_result.apply, request.endpoint) {
            Ok(endpoint_ref) => endpoint_ref,
            Err(error) => return (Err(error), self.owned.session_snapshot(&session_id).ok()),
        };
        let queue_ref = workflow_code_queue_ref(&apply_result.apply, request.queue_ref);
        let invocation_prompt = workflow_code_invocation_prompt(
            &request.prompt,
            apply_result.compile.definition.workflow.prompt.as_deref(),
        );
        let (invoke_response, session) = self
            .execute_workflow_invoke_endpoint_request(
                crate::local::InvokeWorkflowEndpointRequest {
                    session_id: session_id.clone(),
                    workflow_ref: apply_result.apply.workflow_id.clone(),
                    endpoint_ref,
                    queue_ref,
                    prompt: Some(invocation_prompt),
                    publication_invocation: None,
                },
                &caller_user_id,
            )
            .await;
        let result = match invoke_response {
            Ok(crate::local::LocalDaemonResponse::WorkflowRunInvoked {
                workflow_run,
                workflow,
                endpoint,
                session,
            }) => Ok(crate::local::LocalDaemonResponse::WorkflowCodeRun {
                result: crate::workflow_code::WorkflowCodeRunResult {
                    apply: apply_result,
                    invocation: crate::workflow_code::WorkflowCodeRunInvocation::Started {
                        workflow_run,
                        workflow,
                        endpoint,
                    },
                },
                session,
            }),
            Ok(crate::local::LocalDaemonResponse::WorkflowPromptEnqueued {
                queued_prompt,
                workflow,
                endpoint,
                session,
            }) => Ok(crate::local::LocalDaemonResponse::WorkflowCodeRun {
                result: crate::workflow_code::WorkflowCodeRunResult {
                    apply: apply_result,
                    invocation: crate::workflow_code::WorkflowCodeRunInvocation::Enqueued {
                        queued_prompt,
                        workflow,
                        endpoint,
                    },
                },
                session,
            }),
            Ok(_) => Err(DaemonError::LocalTransport {
                operation: "workflow_code.run",
                message: "workflow endpoint invocation returned an unexpected response".to_string(),
            }),
            Err(error) => Err(error),
        };
        if let Ok(crate::local::LocalDaemonResponse::WorkflowCodeRun { result, .. }) = &result {
            self.persist_workflow_code_run_event(
                &session_id,
                &caller_user_id,
                caller_metaagent_id,
                result,
            );
        }
        let session = result
            .as_ref()
            .ok()
            .and_then(workflow_response_session)
            .or(session);
        (result, session)
    }

    pub(super) async fn execute_workflow_code_artifact_run_request(
        &self,
        request: crate::local::RunWorkflowCodeArtifactRequest,
        caller_user_id: &str,
        caller_metaagent_id: Option<&str>,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let caller_user_id = caller_user_id.to_string();
        let controlled_by_metaagent_id = caller_metaagent_id.map(str::to_string);
        let session_id = request.session_id.clone();
        let apply_result = match self
            .with_app_side_effect({
                let session_id = session_id.clone();
                let name = request.name.clone();
                let provider_rebindings = request.provider_rebindings.clone();
                let agent_rebindings = request.agent_rebindings.clone();
                let endpoint = request.endpoint.clone();
                let queue_ref = request.queue_ref.clone();
                let caller_user_id = caller_user_id.clone();
                move |app| {
                    workflow_code_artifact_apply_result(
                        app,
                        &session_id,
                        &name,
                        &provider_rebindings,
                        &agent_rebindings,
                        caller_user_id,
                        controlled_by_metaagent_id,
                        crate::workflow_code::WorkflowCodeArtifactHistoryAction::Run,
                        "workflow_code_artifact.run",
                        Some(endpoint.as_deref()),
                        queue_ref.as_deref(),
                    )
                }
            })
            .await
        {
            Ok(result) => result,
            Err(error) => return (Err(error), None),
        };
        let endpoint_ref = match workflow_code_endpoint_ref(&apply_result.apply, request.endpoint) {
            Ok(endpoint_ref) => endpoint_ref,
            Err(error) => return (Err(error), self.owned.session_snapshot(&session_id).ok()),
        };
        let queue_ref = workflow_code_queue_ref(&apply_result.apply, request.queue_ref);
        let invocation_prompt = workflow_code_invocation_prompt(
            &request.prompt,
            apply_result.compile.definition.workflow.prompt.as_deref(),
        );
        let (invoke_response, session) = self
            .execute_workflow_invoke_endpoint_request(
                crate::local::InvokeWorkflowEndpointRequest {
                    session_id: session_id.clone(),
                    workflow_ref: apply_result.apply.workflow_id.clone(),
                    endpoint_ref,
                    queue_ref,
                    prompt: Some(invocation_prompt),
                    publication_invocation: None,
                },
                &caller_user_id,
            )
            .await;
        let result = match invoke_response {
            Ok(crate::local::LocalDaemonResponse::WorkflowRunInvoked {
                workflow_run,
                workflow,
                endpoint,
                session,
            }) => Ok(crate::local::LocalDaemonResponse::WorkflowCodeRun {
                result: crate::workflow_code::WorkflowCodeRunResult {
                    apply: apply_result,
                    invocation: crate::workflow_code::WorkflowCodeRunInvocation::Started {
                        workflow_run,
                        workflow,
                        endpoint,
                    },
                },
                session,
            }),
            Ok(crate::local::LocalDaemonResponse::WorkflowPromptEnqueued {
                queued_prompt,
                workflow,
                endpoint,
                session,
            }) => Ok(crate::local::LocalDaemonResponse::WorkflowCodeRun {
                result: crate::workflow_code::WorkflowCodeRunResult {
                    apply: apply_result,
                    invocation: crate::workflow_code::WorkflowCodeRunInvocation::Enqueued {
                        queued_prompt,
                        workflow,
                        endpoint,
                    },
                },
                session,
            }),
            Ok(_) => Err(DaemonError::LocalTransport {
                operation: "workflow_code_artifact.run",
                message: "workflow endpoint invocation returned an unexpected response".to_string(),
            }),
            Err(error) => Err(error),
        };
        if let Ok(crate::local::LocalDaemonResponse::WorkflowCodeRun { result, .. }) = &result {
            self.persist_workflow_code_run_event(
                &session_id,
                &caller_user_id,
                caller_metaagent_id,
                result,
            );
        }
        let session = result
            .as_ref()
            .ok()
            .and_then(workflow_response_session)
            .or(session);
        (result, session)
    }
}
