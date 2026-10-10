use super::workflow_code_request_support::*;
use super::workflow_request_runtime_state::workflow_response_session;
use super::*;

impl KernelRuntimeState {
    /// MP-11 F7: at most two compiler jobs per kernel process. No app guard is
    /// held during discovery, sandbox setup, evaluation or schema replay.
    pub(super) async fn run_workflow_compiler_operation<R: Send + 'static>(
        &self,
        operation: impl FnOnce() -> Result<R, DaemonError> + Send + 'static,
    ) -> Result<R, DaemonError> {
        static SLOTS: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
        self.authorize_current_external_command()?;
        let permit = SLOTS
            .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(2)))
            .clone()
            .try_acquire_owned()
            .map_err(|_| DaemonError::LocalTransport {
                operation: "workflow_code.compile",
                message: "workflow compiler capacity reached; retry later".into(),
            })?;
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation()
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "workflow_code.compile",
            message: format!("workflow compiler task failed: {error}"),
        })??;
        self.authorize_current_external_command()?;
        Ok(result)
    }

    pub(super) async fn compile_workflow_request_source(
        &self,
        session_id: &str,
        source: &str,
        language: crate::workflow_code::WorkflowCodeLanguage,
    ) -> Result<crate::workflow_code::WorkflowCodeCompileResult, DaemonError> {
        let session_id = session_id.to_owned();
        let (limits, root) = self
            .with_authorized_app_side_effect(move |app| {
                let session = app.sessions().get_session(&session_id)?;
                let root = std::path::PathBuf::from(session.workspace_id());
                Ok((
                    app.config().workflow_code_limits(),
                    root.is_absolute().then_some(root),
                ))
            })
            .await?;
        let source = source.to_owned();
        self.run_workflow_compiler_operation(move || {
            crate::workflow_code::compile_workflow_code_source_with_schema_import_root(
                "node",
                &source,
                language,
                &limits,
                root.as_deref(),
            )
        })
        .await
    }

    pub(super) async fn execute_workflow_code_validate_request(
        &self,
        request: crate::local::ValidateWorkflowCodeRequest,
        caller_metaagent_id: Option<&str>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let compile = self
            .compile_workflow_request_source(
                &request.session_id,
                &request.source,
                request
                    .language
                    .unwrap_or(crate::workflow_code::WorkflowCodeLanguage::JavaScript),
            )
            .await?;
        let caller_metaagent_id = caller_metaagent_id.map(str::to_string);
        self.with_authorized_app_side_effect(move |app| {
            let limits = app.config().workflow_code_limits();
            let result = validate_compiled_request(
                app,
                self,
                &request.session_id,
                compile,
                &limits,
                (&request.provider_rebindings, &request.agent_rebindings),
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
        let compile = match self
            .compile_workflow_request_source(
                &request.session_id,
                &request.source,
                request
                    .language
                    .unwrap_or(crate::workflow_code::WorkflowCodeLanguage::JavaScript),
            )
            .await
        {
            Ok(compile) => compile,
            Err(error) => return (Err(error), None),
        };
        let caller_user_id = caller_user_id.to_string();
        let controlled_by_metaagent_id = caller_metaagent_id.map(str::to_string);
        let session_id = request.session_id.clone();
        let result = self
            .with_authorized_app_side_effect(move |app| {
                let limits = app.config().workflow_code_limits();
                let language = request
                    .language
                    .unwrap_or(crate::workflow_code::WorkflowCodeLanguage::JavaScript);
                let compile = validate_compiled_request(
                    app,
                    self,
                    &request.session_id,
                    compile,
                    &limits,
                    (&request.provider_rebindings, &request.agent_rebindings),
                    controlled_by_metaagent_id.as_deref(),
                )?;
                let apply = crate::app::KernelSessionService::with_authorization(app, &|| {
                    self.authorize_current_external_command()
                })
                .apply_workflow_code_definition(
                    &request.session_id,
                    &compile.definition,
                    &limits,
                    caller_user_id.clone(),
                    controlled_by_metaagent_id.clone(),
                )?;
                let result =
                    crate::workflow_code::WorkflowCodeCompileAndApplyResult { compile, apply };
                self.authorize_current_external_command()?;
                let artifact_name = format!(
                    "workflow-source-{}-{}",
                    request.session_id, result.apply.workflow_id
                );
                let registry = workflow_code_registry_for_session(app, &request.session_id)?;
                let actor = workflow_code_artifact_actor(
                    &caller_user_id,
                    controlled_by_metaagent_id.as_deref(),
                );
                let artifact = if registry.get(&artifact_name)?.is_some() {
                    registry.update(
                        &artifact_name,
                        language,
                        request.source,
                        result.compile.definition.clone(),
                        result.compile.validation.clone(),
                        actor,
                        crate::workflow_code::WorkflowCodeArtifactHistoryAction::Updated,
                    )?
                } else {
                    registry.save(
                        &artifact_name,
                        language,
                        request.source,
                        result.compile.definition.clone(),
                        result.compile.validation.clone(),
                        actor,
                        crate::workflow_code::WorkflowCodeArtifactHistoryAction::Created,
                    )?
                };
                app.sessions_mut().bind_workflow_code_source(
                    &request.session_id,
                    &result.apply.workflow_id,
                    None,
                    crate::session::WorkflowCodeSourceDescriptor {
                        artifact_name: artifact.metadata.name,
                        language: artifact.metadata.language,
                        source_sha256: artifact.metadata.source_sha256,
                        origin: crate::session::WorkflowCodeSourceOrigin::Authored,
                    },
                    result.apply.clone(),
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
            .with_authorized_app_side_effect(move |app| {
                let result = workflow_code_artifact_apply_result(
                    app,
                    &request.name,
                    crate::workflow_code::WorkflowCodeArtifactHistoryAction::Applied,
                    WorkflowApplyContext {
                        session_id: &request.session_id,
                        provider_rebindings: &request.provider_rebindings,
                        agent_rebindings: &request.agent_rebindings,
                        caller_user_id,
                        controlled_by_metaagent_id,
                        operation: "workflow_code_artifact.apply",
                        run_endpoint: None,
                        run_queue: None,
                        authorize: &|| self.authorize_current_external_command(),
                    },
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
        let compile = match self
            .compile_workflow_request_source(
                &request.session_id,
                &request.source,
                request
                    .language
                    .unwrap_or(crate::workflow_code::WorkflowCodeLanguage::JavaScript),
            )
            .await
        {
            Ok(compile) => compile,
            Err(error) => return (Err(error), None),
        };
        let caller_user_id = caller_user_id.to_string();
        let controlled_by_metaagent_id = caller_metaagent_id.map(str::to_string);
        let session_id = request.session_id.clone();
        let apply_result = match self
            .with_authorized_app_side_effect({
                let session_id = session_id.clone();
                let endpoint = request.endpoint.clone();
                let queue_ref = request.queue_ref.clone();
                let provider_rebindings = request.provider_rebindings.clone();
                let agent_rebindings = request.agent_rebindings.clone();
                let caller_user_id = caller_user_id.clone();
                move |app| {
                    let limits = app.config().workflow_code_limits();
                    let compile = validate_compiled_request(
                        app,
                        self,
                        &session_id,
                        compile,
                        &limits,
                        (&provider_rebindings, &agent_rebindings),
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
                    let apply = crate::app::KernelSessionService::with_authorization(app, &|| {
                        self.authorize_current_external_command()
                    })
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
                caller_metaagent_id,
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
                        workflow_run: Box::new(workflow_run),
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
                        queued_prompt: Box::new(queued_prompt),
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
            .with_authorized_app_side_effect({
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
                        &name,
                        crate::workflow_code::WorkflowCodeArtifactHistoryAction::Run,
                        WorkflowApplyContext {
                            session_id: &session_id,
                            provider_rebindings: &provider_rebindings,
                            agent_rebindings: &agent_rebindings,
                            caller_user_id,
                            controlled_by_metaagent_id,
                            operation: "workflow_code_artifact.run",
                            run_endpoint: Some(endpoint.as_deref()),
                            run_queue: queue_ref.as_deref(),
                            authorize: &|| self.authorize_current_external_command(),
                        },
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
                caller_metaagent_id,
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
                        workflow_run: Box::new(workflow_run),
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
                        queued_prompt: Box::new(queued_prompt),
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

fn validate_compiled_request(
    app: &mut crate::DaemonApp,
    state: &KernelRuntimeState,
    session_id: &str,
    mut compile: crate::workflow_code::WorkflowCodeCompileResult,
    limits: &crate::config::WorkflowCodeLimitsConfig,
    bindings: (
        &[crate::workflow_code::WorkflowCodeProviderRebinding],
        &[crate::workflow_code::WorkflowCodeAgentRebinding],
    ),
    actor: Option<&str>,
) -> Result<crate::workflow_code::WorkflowCodeCompileResult, DaemonError> {
    let (definition, mut validation) =
        crate::app::KernelSessionService::with_authorization(app, &|| {
            state.authorize_current_external_command()
        })
        .validate_workflow_code_definition_with_rebindings(
            session_id,
            &compile.definition,
            limits,
            bindings.0,
            bindings.1,
            actor,
        )?;
    crate::workflow_code::attach_workflow_code_diagnostic_spans(
        &mut validation,
        &compile.source_spans,
    );
    compile.definition = definition;
    compile.validation = validation;
    Ok(compile)
}
