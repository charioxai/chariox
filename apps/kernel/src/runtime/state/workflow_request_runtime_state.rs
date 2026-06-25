use super::*;

impl KernelRuntimeState {
    pub(crate) async fn execute_workflow_request(
        &self,
        request: LocalDaemonRequest,
        caller_user_id: String,
        caller_metaagent_id: Option<String>,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let owned = &self.owned;

        if let Some(metaagent_id) = caller_metaagent_id.as_deref() {
            if let Err(error) =
                owned.ensure_workflow_request_controlled_by_metaagent(&request, metaagent_id)
            {
                return (Err(error), None);
            }
        }

        let outcome = match request {
            LocalDaemonRequest::CreateWorkflow(request) => {
                let result =
                    owned.workflow_create_workflow(request, caller_metaagent_id.as_deref());
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ValidateWorkflowCode(request) => (
                self.execute_workflow_code_validate_request(
                    request,
                    caller_metaagent_id.as_deref(),
                )
                .await,
                None,
            ),
            LocalDaemonRequest::ApplyWorkflowCode(request) => {
                self.execute_workflow_code_apply_request(
                    request,
                    &caller_user_id,
                    caller_metaagent_id.as_deref(),
                )
                .await
            }
            LocalDaemonRequest::CreateWorkflowCodeArtifact(request) => (
                self.execute_workflow_code_artifact_create_request(request)
                    .await,
                None,
            ),
            LocalDaemonRequest::UpdateWorkflowCodeArtifact(request) => (
                self.execute_workflow_code_artifact_update_request(request)
                    .await,
                None,
            ),
            LocalDaemonRequest::GetWorkflowCodeArtifact(request) => (
                self.execute_workflow_code_artifact_get_request(request)
                    .await,
                None,
            ),
            LocalDaemonRequest::ListWorkflowCodeArtifacts(request) => (
                self.execute_workflow_code_artifact_list_request(request)
                    .await,
                None,
            ),
            LocalDaemonRequest::DeleteWorkflowCodeArtifact(request) => (
                self.execute_workflow_code_artifact_delete_request(request)
                    .await,
                None,
            ),
            LocalDaemonRequest::ExportWorkflowCodeArtifact(request) => (
                self.execute_workflow_code_artifact_export_request(request)
                    .await,
                None,
            ),
            LocalDaemonRequest::ImportWorkflowCodeArtifact(request) => (
                self.execute_workflow_code_artifact_import_request(request)
                    .await,
                None,
            ),
            LocalDaemonRequest::ApplyWorkflowDesignOp(request) => {
                let origin_client_id = request.origin_client_id.clone();
                let op_id = request.op_id.clone();
                let session_id = request.session_id.clone();
                let op = request.op.clone();
                let event_store = owned.workflow_design_events.clone();
                let result = owned
                    .workflow_apply_design_op(request, &caller_user_id)
                    .map(|session| {
                        let event = event_store.append(session_id, origin_client_id, op_id, op);
                        LocalDaemonResponse::WorkflowDesignOpAccepted { session, event }
                    });
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::AliasWorkflow(request) => {
                let result = owned.workflow_alias_workflow(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListWorkflows(request) => (
                owned.workflow_list_workflows(request, caller_metaagent_id.as_deref()),
                None,
            ),
            LocalDaemonRequest::ResolveWorkflow(request) => {
                (owned.workflow_resolve_workflow(request), None)
            }
            LocalDaemonRequest::CreateWorkflowPublication(request) => {
                let result = owned.workflow_create_publication(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListWorkflowPublications(request) => {
                (owned.workflow_list_publications(request), None)
            }
            LocalDaemonRequest::GetWorkflowPublication(request) => {
                (owned.workflow_get_publication(request), None)
            }
            LocalDaemonRequest::ExportWorkflowPublicationPackage(request) => {
                (owned.workflow_export_publication_package(request), None)
            }
            LocalDaemonRequest::DisableWorkflowPublication(request) => {
                let result = owned.workflow_disable_publication(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::MaterializeWorkflowPublication(request) => {
                let result = owned.workflow_materialize_publication(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::CreateWorkflowEndpoint(request) => {
                let result = owned.workflow_create_endpoint(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::AliasWorkflowEndpoint(request) => {
                let result = owned.workflow_alias_endpoint(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::BindWorkflowEndpoint(request) => {
                let result = owned.workflow_bind_endpoint(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::AddWorkflowNode(request) => {
                let result = owned.workflow_add_node(
                    request,
                    &caller_user_id,
                    caller_metaagent_id.as_deref(),
                );
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::RemoveWorkflowNode(request) => {
                let result = owned.workflow_remove_node(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::UpdateWorkflowNodeInstructions(request) => {
                let result = owned.workflow_update_node_instructions(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowNodeCanCompleteRun(request) => {
                let result = owned.workflow_set_node_can_complete_run(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowNodeCanEmitIntermediateOutput(request) => {
                let result =
                    owned.workflow_set_node_can_emit_intermediate_output(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowNodeWaitForAllInputs(request) => {
                let result = owned.workflow_set_node_wait_for_all_inputs(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowNodeIntermediateOutputSchema(request) => {
                let result =
                    owned.workflow_set_node_intermediate_output_schema(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowNodeMaxTurns(request) => {
                let result = owned.workflow_set_node_max_turns(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::AddWorkflowEdge(request) => {
                let result = owned.workflow_add_edge(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::RemoveWorkflowEdge(request) => {
                let result = owned.workflow_remove_edge(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::UpdateWorkflowCanvasLayout(request) => {
                let result = owned.workflow_update_canvas_layout(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowFlushContext(request) => {
                let result = owned.workflow_set_flush_context(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowRunOutputSchema(request) => {
                let result = owned.workflow_set_run_output_schema(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::SetWorkflowIntermediateOutputSchema(request) => {
                let result = owned.workflow_set_intermediate_output_schema(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListWorkflowRuns(request) => (
                owned.workflow_list_runs(request, caller_metaagent_id.as_deref()),
                None,
            ),
            LocalDaemonRequest::GetWorkflowRun(request) => (owned.workflow_get_run(request), None),
            LocalDaemonRequest::CreateWorkflowWatchdog(request) => {
                let result = owned.workflow_create_watchdog(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListWorkflowWatchdogs(request) => {
                (owned.workflow_list_watchdogs(request), None)
            }
            LocalDaemonRequest::SetWorkflowWatchdogEnabled(request) => {
                let result = owned.workflow_set_watchdog_enabled(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::RemoveWorkflowWatchdog(request) => {
                let result = owned.workflow_remove_watchdog(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListWorkflowPromptQueues(request) => {
                (owned.workflow_list_prompt_queues(request), None)
            }
            LocalDaemonRequest::CreateWorkflowPromptQueue(request) => {
                let result = owned.workflow_create_prompt_queue(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::UpdateWorkflowPromptQueue(request) => {
                let result = owned.workflow_update_prompt_queue(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::RemoveWorkflowPromptQueue(request) => {
                let result = owned.workflow_remove_prompt_queue(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListQueuedWorkflowPrompts(request) => {
                (owned.workflow_list_queued_prompts(request), None)
            }
            LocalDaemonRequest::UpdateQueuedWorkflowPrompt(request) => {
                let result = owned.workflow_update_queued_prompt(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::RemoveQueuedWorkflowPrompt(request) => {
                let result = owned.workflow_remove_queued_prompt(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ClearWorkflowPromptQueue(request) => {
                let result = owned.workflow_clear_prompt_queue(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::InvokeWorkflowEndpoint(request) => {
                self.execute_workflow_invoke_endpoint_request(request, &caller_user_id)
                    .await
            }
            LocalDaemonRequest::CancelWorkflowRun(request) => {
                self.execute_workflow_cancel_run_request(request)
            }
            LocalDaemonRequest::ResumeWorkflowRun(request) => {
                self.execute_workflow_resume_run_request(request)
            }
            LocalDaemonRequest::ValidateWorkflowHandoff(request) => {
                let result = owned.workflow_validate_handoff(request);
                (result, None)
            }
            LocalDaemonRequest::AckWorkflowTurn(request) => {
                let result = owned.workflow_ack_turn(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            _ => (
                Err(DaemonError::LocalTransport {
                    operation: "execute workflow request",
                    message: "request is not handled by the workflow runtime".to_string(),
                }),
                None,
            ),
        };
        if outcome.0.is_ok() {
            if let Some(session) = outcome.1.as_ref() {
                if let Err(error) = self
                    .append_session_durable_event("session.updated", session, "workflow")
                    .await
                {
                    return (Err(error), outcome.1);
                }
            }
        }
        outcome
    }

    async fn execute_workflow_code_validate_request(
        &self,
        request: crate::local::ValidateWorkflowCodeRequest,
        caller_metaagent_id: Option<&str>,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        let caller_metaagent_id = caller_metaagent_id.map(str::to_string);
        self.with_app_side_effect(move |app| {
            let limits = app.config().workflow_code_limits();
            let result = crate::app::KernelSessionService::new(app)
                .compile_and_validate_workflow_code_javascript_with_rebindings(
                    &request.session_id,
                    &request.node_path,
                    &request.source,
                    &limits,
                    &request.provider_rebindings,
                    caller_metaagent_id.as_deref(),
                )?;
            Ok(LocalDaemonResponse::WorkflowCodeValidated { result })
        })
        .await
    }

    async fn execute_workflow_code_apply_request(
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
                let result = app.compile_and_apply_workflow_code_javascript_with_rebindings(
                    &request.session_id,
                    &request.node_path,
                    &request.source,
                    &limits,
                    caller_user_id,
                    controlled_by_metaagent_id,
                    &request.provider_rebindings,
                )?;
                let session =
                    crate::app::KernelSessionReadService::new(app).session_snapshot(&session_id)?;
                Ok(LocalDaemonResponse::WorkflowCodeApplied { result, session })
            })
            .await;
        let session = result.as_ref().ok().and_then(workflow_response_session);
        (result, session)
    }

    async fn execute_workflow_code_artifact_create_request(
        &self,
        request: crate::local::CreateWorkflowCodeArtifactRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let limits = app.config().workflow_code_limits();
            let compile = crate::workflow_code::compile_workflow_code_javascript(
                &request.node_path,
                &request.source,
                &limits,
            )?;
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            let artifact = registry.save(
                &request.name,
                request.language,
                request.source,
                compile.definition,
                &limits,
            )?;
            app.durable_state_store().append_event(
                "workflow_code_artifact.created",
                Some(request.session_id),
                serde_json::json!({
                    "artifact": &artifact.metadata,
                }),
            )?;
            Ok(LocalDaemonResponse::WorkflowCodeArtifactCreated { artifact })
        })
        .await
    }

    async fn execute_workflow_code_artifact_update_request(
        &self,
        request: crate::local::UpdateWorkflowCodeArtifactRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let limits = app.config().workflow_code_limits();
            let compile = crate::workflow_code::compile_workflow_code_javascript(
                &request.node_path,
                &request.source,
                &limits,
            )?;
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            let artifact = registry.update(
                &request.name,
                request.language,
                request.source,
                compile.definition,
                &limits,
            )?;
            app.durable_state_store().append_event(
                "workflow_code_artifact.updated",
                Some(request.session_id),
                serde_json::json!({
                    "artifact": &artifact.metadata,
                }),
            )?;
            Ok(LocalDaemonResponse::WorkflowCodeArtifactUpdated { artifact })
        })
        .await
    }

    async fn execute_workflow_code_artifact_get_request(
        &self,
        request: crate::local::GetWorkflowCodeArtifactRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            let artifact =
                registry
                    .get(&request.name)?
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "workflow_code.get",
                        message: format!("workflow-code artifact `{}` is not saved", request.name),
                    })?;
            Ok(LocalDaemonResponse::WorkflowCodeArtifact { artifact })
        })
        .await
    }

    async fn execute_workflow_code_artifact_list_request(
        &self,
        request: crate::local::ListWorkflowCodeArtifactsRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            let artifacts = registry.list()?;
            Ok(LocalDaemonResponse::WorkflowCodeArtifactsListed { artifacts })
        })
        .await
    }

    async fn execute_workflow_code_artifact_delete_request(
        &self,
        request: crate::local::DeleteWorkflowCodeArtifactRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            let path = registry.delete(&request.name)?;
            app.durable_state_store().append_event(
                "workflow_code_artifact.deleted",
                Some(request.session_id),
                serde_json::json!({
                    "name": &request.name,
                    "path": &path,
                }),
            )?;
            Ok(LocalDaemonResponse::WorkflowCodeArtifactDeleted {
                name: request.name,
                path,
            })
        })
        .await
    }

    async fn execute_workflow_code_artifact_export_request(
        &self,
        request: crate::local::ExportWorkflowCodeArtifactRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            let package = registry.export_package(&request.name)?;
            app.durable_state_store().append_event(
                "workflow_code_artifact.exported",
                Some(request.session_id),
                serde_json::json!({
                    "name": &package.name,
                    "source_sha256": &package.source_sha256,
                    "source_bytes": package.source_bytes,
                    "package_version": package.package_version,
                }),
            )?;
            Ok(LocalDaemonResponse::WorkflowCodeArtifactExported { package })
        })
        .await
    }

    async fn execute_workflow_code_artifact_import_request(
        &self,
        request: crate::local::ImportWorkflowCodeArtifactRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            request.package.validate_integrity()?;
            let limits = app.config().workflow_code_limits();
            let compile = crate::workflow_code::compile_workflow_code_javascript(
                &request.node_path,
                &request.package.source,
                &limits,
            )?;
            let registry = workflow_code_registry_for_session(app, &request.session_id)?;
            let artifact = registry.import_package(
                request.name.as_deref(),
                request.package,
                compile.definition,
                &limits,
                request.overwrite,
            )?;
            app.durable_state_store().append_event(
                "workflow_code_artifact.imported",
                Some(request.session_id),
                serde_json::json!({
                    "artifact": &artifact.metadata,
                    "overwrite": request.overwrite,
                }),
            )?;
            Ok(LocalDaemonResponse::WorkflowCodeArtifactImported { artifact })
        })
        .await
    }
}

fn workflow_code_registry_for_session(
    app: &crate::app::DaemonApp,
    session_id: &str,
) -> Result<crate::workflow_code::WorkflowCodeArtifactRegistry, DaemonError> {
    let session = app.sessions().get_session(session_id)?;
    let mut roots = Vec::new();
    if !session.workspace_id().trim().is_empty() {
        roots.push(
            crate::workflow_code::WorkflowCodeArtifactRegistry::project_root(
                session.workspace_id(),
            ),
        );
    }
    if let Some(root) = crate::workflow_code::WorkflowCodeArtifactRegistry::user_root() {
        roots.push(root);
    }
    Ok(crate::workflow_code::WorkflowCodeArtifactRegistry::new(
        roots,
    ))
}

pub(super) fn workflow_response_session(
    response: &LocalDaemonResponse,
) -> Option<crate::session::RuntimeSession> {
    match response {
        LocalDaemonResponse::WorkflowCreated { session, .. }
        | LocalDaemonResponse::WorkflowCodeApplied { session, .. }
        | LocalDaemonResponse::WorkflowDesignOpAccepted { session, .. }
        | LocalDaemonResponse::WorkflowAliased { session, .. }
        | LocalDaemonResponse::WorkflowPublicationCreated { session, .. }
        | LocalDaemonResponse::WorkflowPublicationDisabled { session, .. }
        | LocalDaemonResponse::WorkflowPublicationMaterialized { session, .. }
        | LocalDaemonResponse::WorkflowEndpointCreated { session, .. }
        | LocalDaemonResponse::WorkflowEndpointAliased { session, .. }
        | LocalDaemonResponse::WorkflowEndpointBound { session, .. }
        | LocalDaemonResponse::WorkflowNodeAdded { session, .. }
        | LocalDaemonResponse::WorkflowNodeRemoved { session, .. }
        | LocalDaemonResponse::WorkflowNodeInstructionsUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeWaitForAllInputsUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated { session, .. }
        | LocalDaemonResponse::WorkflowEdgeAdded { session, .. }
        | LocalDaemonResponse::WorkflowEdgeRemoved { session, .. }
        | LocalDaemonResponse::WorkflowCanvasLayoutUpdated { session, .. }
        | LocalDaemonResponse::WorkflowRunInvoked { session, .. }
        | LocalDaemonResponse::WorkflowPromptEnqueued { session, .. }
        | LocalDaemonResponse::WorkflowRunCancelled { session, .. }
        | LocalDaemonResponse::WorkflowRunResumed { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogCreated { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogUpdated { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogRemoved { session, .. }
        | LocalDaemonResponse::WorkflowFlushContextUpdated { session, .. }
        | LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowPromptQueueCreated { session, .. }
        | LocalDaemonResponse::WorkflowPromptQueueUpdated { session, .. }
        | LocalDaemonResponse::WorkflowPromptQueueRemoved { session, .. }
        | LocalDaemonResponse::QueuedWorkflowPromptUpdated { session, .. }
        | LocalDaemonResponse::QueuedWorkflowPromptRemoved { session, .. }
        | LocalDaemonResponse::WorkflowPromptQueueCleared { session, .. }
        | LocalDaemonResponse::WorkflowTurnAcknowledged { session, .. } => Some(session.clone()),
        _ => None,
    }
}
