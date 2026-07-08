use super::workflow_code_request_support::*;
use super::workflow_request_runtime_state::workflow_response_session;
use super::*;

impl KernelRuntimeState {
    pub(super) async fn execute_workflow_registry_list_request(
        &self,
        request: crate::local::ListWorkflowRegistryRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let registry = workflow_registry_for_session(app, &request.session_id)?;
            let limits = app.config().workflow_code_limits();
            let node_path = crate::workflow_code::discover_workflow_code_node_path()?;
            let entries = registry
                .list()?
                .into_iter()
                .map(|entry| {
                    if entry.summary.is_some() {
                        return entry;
                    }
                    match registry.resolve(&entry.name) {
                        Ok(resolved) => {
                            crate::workflow_code::enrich_workflow_registry_entry_summary(
                                resolved, &node_path, &limits,
                            )
                        }
                        Err(error) => {
                            crate::workflow_code::workflow_registry_metadata_with_summary_failure(
                                entry, error,
                            )
                        }
                    }
                })
                .collect();
            Ok(LocalDaemonResponse::WorkflowRegistryListed { entries })
        })
        .await
    }

    pub(super) async fn execute_workflow_registry_get_request(
        &self,
        request: crate::local::GetWorkflowRegistryEntryRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let registry = workflow_registry_for_session(app, &request.session_id)?;
            let limits = app.config().workflow_code_limits();
            let node_path = crate::workflow_code::discover_workflow_code_node_path()?;
            let resolved = registry.resolve(&request.name)?;
            let entry = crate::workflow_code::enrich_workflow_registry_entry_summary(
                resolved, &node_path, &limits,
            );
            Ok(LocalDaemonResponse::WorkflowRegistryEntry { entry })
        })
        .await
    }

    pub(super) async fn execute_workflow_registry_add_request(
        &self,
        request: crate::local::AddWorkflowRegistryEntryRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let limits = app.config().workflow_code_limits();
            let registry = workflow_registry_for_session(app, &request.session_id)?;
            let scope = workflow_registry_write_scope(app, &request.session_id, request.scope)?;
            let entry = registry.add(
                &request.name,
                scope,
                request.source,
                &request.node_path,
                &limits,
            )?;
            app.durable_state_store().append_event(
                "workflow_registry.added",
                Some(request.session_id),
                serde_json::json!({ "entry": &entry }),
            )?;
            Ok(LocalDaemonResponse::WorkflowRegistryEntryAdded { entry })
        })
        .await
    }

    pub(super) async fn execute_workflow_registry_add_from_workflow_request(
        &self,
        request: crate::local::AddWorkflowRegistryEntryFromWorkflowRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let limits = app.config().workflow_code_limits();
            let session = crate::app::KernelSessionReadService::new(app)
                .session_snapshot(&request.session_id)?;
            let export = crate::workflow_code::export_workflow_code_source_from_session_workflow(
                &session,
                &request.workflow_ref,
                crate::workflow_code::WorkflowCodeSourceExportFormat::Inline,
                request.agent_mode,
            )?;
            let registry = workflow_registry_for_session(app, &request.session_id)?;
            let scope = workflow_registry_write_scope(app, &request.session_id, request.scope)?;
            let node_path = crate::workflow_code::discover_workflow_code_node_path()?;
            let entry = registry.add_from_export(
                &request.name,
                scope,
                export,
                node_path.to_string_lossy().as_ref(),
                &limits,
            )?;
            app.durable_state_store().append_event(
                "workflow_registry.added_from_workflow",
                Some(request.session_id),
                serde_json::json!({
                    "entry": &entry,
                    "workflow_ref": &request.workflow_ref,
                    "agent_mode": request.agent_mode,
                }),
            )?;
            Ok(LocalDaemonResponse::WorkflowRegistryEntryAdded { entry })
        })
        .await
    }

    pub(super) async fn execute_workflow_registry_delete_request(
        &self,
        request: crate::local::DeleteWorkflowRegistryEntryRequest,
    ) -> Result<LocalDaemonResponse, DaemonError> {
        self.with_app_side_effect(move |app| {
            let registry = workflow_registry_for_session(app, &request.session_id)?;
            let path = registry.delete(&request.name, request.scope)?;
            app.durable_state_store().append_event(
                "workflow_registry.deleted",
                Some(request.session_id),
                serde_json::json!({ "name": &request.name, "path": &path }),
            )?;
            Ok(LocalDaemonResponse::WorkflowRegistryEntryDeleted {
                name: request.name,
                path,
            })
        })
        .await
    }

    pub(super) async fn execute_workflow_registry_load_request(
        &self,
        request: crate::local::LoadWorkflowRegistryEntryRequest,
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
                let (entry, result) = workflow_registry_apply_result(
                    app,
                    &request.session_id,
                    &request.name,
                    &request.parameters,
                    &request.provider_rebindings,
                    &request.agent_rebindings,
                    caller_user_id,
                    controlled_by_metaagent_id,
                    "workflow_registry.load",
                    None,
                    None,
                )?;
                let session =
                    crate::app::KernelSessionReadService::new(app).session_snapshot(&session_id)?;
                Ok(LocalDaemonResponse::WorkflowRegistryEntryLoaded {
                    entry,
                    result,
                    session,
                })
            })
            .await;
        let session = result.as_ref().ok().and_then(workflow_response_session);
        (result, session)
    }

    pub(super) async fn execute_workflow_registry_run_request(
        &self,
        request: crate::local::RunWorkflowRegistryEntryRequest,
        caller_user_id: &str,
        caller_metaagent_id: Option<&str>,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let caller_user_id = caller_user_id.to_string();
        let controlled_by_metaagent_id = caller_metaagent_id.map(str::to_string);
        let session_id = request.session_id.clone();
        let (entry, apply_result) = match self
            .with_app_side_effect({
                let session_id = session_id.clone();
                let name = request.name.clone();
                let parameters = request.parameters.clone();
                let provider_rebindings = request.provider_rebindings.clone();
                let agent_rebindings = request.agent_rebindings.clone();
                let endpoint = request.endpoint.clone();
                let queue_ref = request.queue_ref.clone();
                let caller_user_id = caller_user_id.clone();
                move |app| {
                    workflow_registry_apply_result(
                        app,
                        &session_id,
                        &name,
                        &parameters,
                        &provider_rebindings,
                        &agent_rebindings,
                        caller_user_id,
                        controlled_by_metaagent_id,
                        "workflow_registry.run",
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
            }) => Ok(
                crate::local::LocalDaemonResponse::WorkflowRegistryEntryRun {
                    entry,
                    result: crate::workflow_code::WorkflowCodeRunResult {
                        apply: apply_result,
                        invocation: crate::workflow_code::WorkflowCodeRunInvocation::Started {
                            workflow_run,
                            workflow,
                            endpoint,
                        },
                    },
                    session,
                },
            ),
            Ok(crate::local::LocalDaemonResponse::WorkflowPromptEnqueued {
                queued_prompt,
                workflow,
                endpoint,
                session,
            }) => Ok(
                crate::local::LocalDaemonResponse::WorkflowRegistryEntryRun {
                    entry,
                    result: crate::workflow_code::WorkflowCodeRunResult {
                        apply: apply_result,
                        invocation: crate::workflow_code::WorkflowCodeRunInvocation::Enqueued {
                            queued_prompt,
                            workflow,
                            endpoint,
                        },
                    },
                    session,
                },
            ),
            Ok(_) => Err(DaemonError::LocalTransport {
                operation: "workflow_registry.run",
                message: "workflow endpoint invocation returned an unexpected response".to_string(),
            }),
            Err(error) => Err(error),
        };
        if let Ok(crate::local::LocalDaemonResponse::WorkflowRegistryEntryRun { result, .. }) =
            &result
        {
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
