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
            LocalDaemonRequest::ApplyWorkflowCodeArtifact(request) => {
                self.execute_workflow_code_artifact_apply_request(
                    request,
                    &caller_user_id,
                    caller_metaagent_id.as_deref(),
                )
                .await
            }
            LocalDaemonRequest::RunWorkflowCode(request) => {
                self.execute_workflow_code_run_request(
                    request,
                    &caller_user_id,
                    caller_metaagent_id.as_deref(),
                )
                .await
            }
            LocalDaemonRequest::RunWorkflowCodeArtifact(request) => {
                self.execute_workflow_code_artifact_run_request(
                    request,
                    &caller_user_id,
                    caller_metaagent_id.as_deref(),
                )
                .await
            }
            LocalDaemonRequest::ListWorkflowRegistry(request) => (
                self.execute_workflow_registry_list_request(request).await,
                None,
            ),
            LocalDaemonRequest::GetWorkflowRegistryEntry(request) => (
                self.execute_workflow_registry_get_request(request).await,
                None,
            ),
            LocalDaemonRequest::AddWorkflowRegistryEntry(request) => (
                self.execute_workflow_registry_add_request(request).await,
                None,
            ),
            LocalDaemonRequest::AddWorkflowRegistryEntryFromWorkflow(request) => (
                self.execute_workflow_registry_add_from_workflow_request(request)
                    .await,
                None,
            ),
            LocalDaemonRequest::DeleteWorkflowRegistryEntry(request) => (
                self.execute_workflow_registry_delete_request(request).await,
                None,
            ),
            LocalDaemonRequest::LoadWorkflowRegistryEntry(request) => {
                self.execute_workflow_registry_load_request(
                    request,
                    &caller_user_id,
                    caller_metaagent_id.as_deref(),
                )
                .await
            }
            LocalDaemonRequest::RunWorkflowRegistryEntry(request) => {
                self.execute_workflow_registry_run_request(
                    request,
                    &caller_user_id,
                    caller_metaagent_id.as_deref(),
                )
                .await
            }
            LocalDaemonRequest::CreateWorkflowCodeArtifact(request) => (
                self.execute_workflow_code_artifact_create_request(
                    request,
                    &caller_user_id,
                    caller_metaagent_id.as_deref(),
                )
                .await,
                None,
            ),
            LocalDaemonRequest::UpdateWorkflowCodeArtifact(request) => (
                self.execute_workflow_code_artifact_update_request(
                    request,
                    &caller_user_id,
                    caller_metaagent_id.as_deref(),
                )
                .await,
                None,
            ),
            LocalDaemonRequest::BindWorkflowCodeSource(request) => {
                let result = self
                    .execute_workflow_code_source_bind_request(request)
                    .await;
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
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
                self.execute_workflow_code_artifact_import_request(
                    request,
                    &caller_user_id,
                    caller_metaagent_id.as_deref(),
                )
                .await,
                None,
            ),
            LocalDaemonRequest::ExportWorkflowCodePackage(request) => (
                self.execute_workflow_code_package_export_request(request)
                    .await,
                None,
            ),
            LocalDaemonRequest::ImportWorkflowCodePackage(request) => (
                self.execute_workflow_code_package_import_request(
                    request,
                    &caller_user_id,
                    caller_metaagent_id.as_deref(),
                )
                .await,
                None,
            ),
            LocalDaemonRequest::ExportWorkflowCodeSource(request) => (
                self.execute_workflow_code_source_export_request(request)
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
                    .workflow_apply_design_op(
                        request,
                        &caller_user_id,
                        caller_metaagent_id.as_deref(),
                    )
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
            LocalDaemonRequest::CreateWorkflowEventBinding(request) => {
                let result = owned.workflow_create_event_binding(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListWorkflowEventBindings(request) => {
                (owned.workflow_list_event_bindings(request), None)
            }
            LocalDaemonRequest::SetWorkflowEventBindingStatus(request) => {
                let result = owned.workflow_set_event_binding_status(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::TransferWorkflowEventBinding(request) => {
                let result = owned.workflow_transfer_event_binding(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::TestWorkflowEventBinding(request) => {
                let session_id = request.session_id.clone();
                let result = owned
                    .workflow_test_event_delivery_envelope(request, &caller_user_id)
                    .and_then(|delivery| {
                        self.accept_workflow_event_delivery(delivery)
                            .map(|accepted| LocalDaemonResponse::WorkflowEventBindingTested {
                                delivery_id: accepted.delivery_id,
                                queued_prompt_id: accepted.queued_prompt_id,
                                duplicate: accepted.duplicate,
                                session: accepted.session,
                            })
                    });
                let session = result
                    .as_ref()
                    .ok()
                    .and_then(workflow_response_session)
                    .or_else(|| owned.session_snapshot(&session_id).ok());
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
            LocalDaemonRequest::CreateWorkflowSchedule(request) => {
                let result = owned.workflow_create_schedule(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListWorkflowSchedules(request) => {
                (owned.workflow_list_schedules(request), None)
            }
            LocalDaemonRequest::SetWorkflowScheduleEnabled(request) => {
                let result = owned.workflow_set_schedule_enabled(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::RemoveWorkflowSchedule(request) => {
                let result = owned.workflow_remove_schedule(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::PreviewWorkflowSchedule(request) => {
                (owned.workflow_preview_schedule(request), None)
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
                self.execute_workflow_cancel_run_request(request).await
            }
            LocalDaemonRequest::PauseWorkflowRun(request) => {
                self.execute_workflow_pause_run_request(request).await
            }
            LocalDaemonRequest::ResumeWorkflowRun(request) => {
                self.execute_workflow_resume_run_request(request).await
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
}

pub(super) fn workflow_response_session(
    response: &LocalDaemonResponse,
) -> Option<crate::session::RuntimeSession> {
    match response {
        LocalDaemonResponse::WorkflowCreated { session, .. }
        | LocalDaemonResponse::WorkflowCodeSourceBound { session, .. }
        | LocalDaemonResponse::WorkflowCodeApplied { session, .. }
        | LocalDaemonResponse::WorkflowCodeRun { session, .. }
        | LocalDaemonResponse::WorkflowRegistryEntryLoaded { session, .. }
        | LocalDaemonResponse::WorkflowRegistryEntryRun { session, .. }
        | LocalDaemonResponse::WorkflowDesignOpAccepted { session, .. }
        | LocalDaemonResponse::WorkflowAliased { session, .. }
        | LocalDaemonResponse::WorkflowPublicationCreated { session, .. }
        | LocalDaemonResponse::WorkflowEventBindingCreated { session, .. }
        | LocalDaemonResponse::WorkflowEventBindingUpdated { session, .. }
        | LocalDaemonResponse::WorkflowEventBindingTested { session, .. }
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
        | LocalDaemonResponse::WorkflowRunPaused { session, .. }
        | LocalDaemonResponse::WorkflowRunResumed { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogCreated { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogUpdated { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogRemoved { session, .. }
        | LocalDaemonResponse::WorkflowScheduleCreated { session, .. }
        | LocalDaemonResponse::WorkflowScheduleUpdated { session, .. }
        | LocalDaemonResponse::WorkflowScheduleRemoved { session, .. }
        | LocalDaemonResponse::WorkflowFlushContextUpdated { session, .. }
        | LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowPromptQueueCreated { session, .. }
        | LocalDaemonResponse::WorkflowPromptQueueUpdated { session, .. }
        | LocalDaemonResponse::WorkflowPromptQueueRemoved { session, .. }
        | LocalDaemonResponse::QueuedWorkflowPromptUpdated { session, .. }
        | LocalDaemonResponse::QueuedWorkflowPromptRemoved { session, .. }
        | LocalDaemonResponse::WorkflowPromptQueueCleared { session, .. }
        | LocalDaemonResponse::WorkflowTurnAcknowledged { session, .. } => Some(session.clone()),
        LocalDaemonResponse::WorkflowEventBindingTransferred { .. } => None,
        _ => None,
    }
}
