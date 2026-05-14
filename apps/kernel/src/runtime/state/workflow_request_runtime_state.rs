use super::*;

impl KernelRuntimeState {
    pub(crate) async fn execute_workflow_request(
        &self,
        request: LocalDaemonRequest,
        caller_user_id: String,
    ) -> (
        Result<LocalDaemonResponse, DaemonError>,
        Option<crate::session::RuntimeSession>,
    ) {
        let owned = &self.owned;

        let outcome = match request {
            LocalDaemonRequest::CreateWorkflow(request) => {
                let result = owned.workflow_create_workflow(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ApplyWorkflowDesignOp(request) => {
                let origin_client_id = request.origin_client_id.clone();
                let op_id = request.op_id.clone();
                let session_id = request.session_id.clone();
                let op = request.op.clone();
                let event_store = {
                    let app = self.app.lock().await;
                    app.workflow_design_event_store()
                };
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
            LocalDaemonRequest::ListWorkflows(request) => {
                (owned.workflow_list_workflows(request), None)
            }
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
            LocalDaemonRequest::DisableWorkflowPublication(request) => {
                let result = owned.workflow_disable_publication(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::CreateWorkflowPublicationPairCode(request) => {
                let result = owned.workflow_create_publication_pair_code(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::RedeemWorkflowPublicationPairCode(request) => {
                let result = owned.workflow_redeem_publication_pair_code(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListWorkflowPublicationSenders(request) => (
                owned.workflow_list_publication_senders(request, &caller_user_id),
                None,
            ),
            LocalDaemonRequest::RevokeWorkflowPublicationSender(request) => {
                let result = owned.workflow_revoke_publication_sender(request, &caller_user_id);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::AuthenticateWorkflowPublicationSender(request) => (
                owned.workflow_authenticate_publication_sender(request),
                None,
            ),
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
                let result = owned.workflow_add_node(request, &caller_user_id);
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
            LocalDaemonRequest::SetWorkflowLaunchPolicy(request) => {
                let result = owned.workflow_set_launch_policy(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ListWorkflowRuns(request) => {
                (owned.workflow_list_runs(request), None)
            }
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
            LocalDaemonRequest::ListQueuedWorkflowLaunches(request) => {
                (owned.workflow_list_queued_launches(request), None)
            }
            LocalDaemonRequest::RemoveQueuedWorkflowLaunch(request) => {
                let result = owned.workflow_remove_queued_launch(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::ClearQueuedWorkflowLaunches(request) => {
                let result = owned.workflow_clear_queued_launches(request);
                let session = result.as_ref().ok().and_then(workflow_response_session);
                (result, session)
            }
            LocalDaemonRequest::InvokeWorkflowEndpoint(request) => {
                let session_id = request.session_id.clone();
                let result = match owned
                    .ensure_workflow_endpoint_owner(
                        &request.session_id,
                        &request.workflow_ref,
                        &request.endpoint_ref,
                        &caller_user_id,
                        "invoke workflow endpoint",
                    )
                    .and_then(|()| {
                        owned.workflow_invoke_endpoint_with_admission(
                            &request.session_id,
                            &request.workflow_ref,
                            &request.endpoint_ref,
                            request.prompt,
                        )
                    }) {
                    Ok((outcome, dispatches)) => {
                        self.spawn_workflow_prompt_dispatches(dispatches);
                        let session = match owned.session_snapshot(&request.session_id) {
                            Ok(session) => session,
                            Err(error) => return (Err(error), None),
                        };
                        match outcome {
                            crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
                                workflow_run,
                                workflow,
                                endpoint,
                            } => Ok(LocalDaemonResponse::WorkflowRunInvoked {
                                workflow_run,
                                workflow,
                                endpoint,
                                session,
                            }),
                            crate::app::workflow_runtime::WorkflowLaunchOutcome::Queued {
                                queued_launch,
                                workflow,
                                endpoint,
                            } => Ok(LocalDaemonResponse::WorkflowRunQueued {
                                queued_launch,
                                workflow,
                                endpoint,
                                session,
                            }),
                        }
                    }
                    Err(error) => Err(error),
                };
                let session = result
                    .as_ref()
                    .ok()
                    .and_then(workflow_response_session)
                    .or_else(|| owned.session_snapshot(&session_id).ok());
                (result, session)
            }
            LocalDaemonRequest::CancelWorkflowRun(request) => {
                let session_id = request.session_id.clone();
                let result = (|| {
                    let workflow_run_id = owned
                        .session_store
                        .read()
                        .resolve_workflow_run_ref(&request.session_id, &request.workflow_run_ref)?
                        .id()
                        .to_string();
                    let session = owned.session_store.get_session(&request.session_id)?;
                    for agent in owned.agent_store.get_session_agents(&request.session_id) {
                        if owned
                            .prompt_state_owner
                            .active_prompt_for_agent(&session, agent.id())
                            .and_then(|prompt| prompt.workflow_run_id().map(str::to_string))
                            .as_deref()
                            == Some(workflow_run_id.as_str())
                        {
                            let _ = owned
                                .prompt_state_owner
                                .begin_cancelling_active_prompt(&session, agent.id())
                                .ok_or_else(|| DaemonError::NoActivePrompt {
                                    session_id: request.session_id.clone(),
                                })?;
                            let (active_prompt, queued_prompts) =
                                owned.prompt_state_owner.state_parts(&session, agent.id());
                            owned.session_store.mirror_agent_prompt_state(
                                &request.session_id,
                                agent.id(),
                                active_prompt,
                                queued_prompts,
                            )?;
                        }
                    }
                    let workflow_run = owned
                        .session_store
                        .write()
                        .cancel_workflow_run(&request.session_id, &request.workflow_run_ref)?;
                    let _ = owned.prompt_workspace_claims.remove_matching(|claim| {
                        claim.session_id == request.session_id
                            && claim.operation == "workflow_node_dispatch"
                    });
                    let workflow = owned
                        .session_store
                        .read()
                        .resolve_workflow_ref(&request.session_id, workflow_run.workflow_id())?;
                    for node in workflow.nodes() {
                        if let Some(run) = owned
                            .provider_store
                            .get_run_for_agent(&request.session_id, node.agent_id())
                        {
                            let _ = owned.clear_prompt_activity(run.id());
                        }
                    }
                    let session = owned.session_store.get_session(&request.session_id)?;
                    let _ = owned
                        .prompt_state_owner
                        .remove_queued_prompts_by_workflow_run(&session, &workflow_run_id);
                    for agent in owned.agent_store.get_session_agents(&request.session_id) {
                        let (active_prompt, queued_prompts) =
                            owned.prompt_state_owner.state_parts(&session, agent.id());
                        let _ = owned.session_store.mirror_agent_prompt_state(
                            &request.session_id,
                            agent.id(),
                            active_prompt,
                            queued_prompts,
                        );
                    }
                    owned.workflow_maybe_start_next_queued_launch(&request.session_id);
                    let session = owned.session_snapshot(&request.session_id)?;
                    Ok(LocalDaemonResponse::WorkflowRunCancelled {
                        workflow_run,
                        session,
                    })
                })();
                let session = result
                    .as_ref()
                    .ok()
                    .and_then(workflow_response_session)
                    .or_else(|| owned.session_snapshot(&session_id).ok());
                (result, session)
            }
            LocalDaemonRequest::ResumeWorkflowRun(request) => {
                let session_id = request.session_id.clone();
                let result = match owned
                    .workflow_resume_run(&request.session_id, &request.workflow_run_ref)
                {
                    Ok((workflow_run, dispatches)) => {
                        self.spawn_workflow_prompt_dispatches(dispatches);
                        owned.workflow_session(&request.session_id).map(|session| {
                            LocalDaemonResponse::WorkflowRunResumed {
                                workflow_run,
                                session,
                            }
                        })
                    }
                    Err(error) => Err(error),
                };
                let session = result
                    .as_ref()
                    .ok()
                    .and_then(workflow_response_session)
                    .or_else(|| owned.session_snapshot(&session_id).ok());
                (result, session)
            }
            LocalDaemonRequest::ValidateWorkflowOutput(request) => {
                let result = owned.workflow_validate_output(request);
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

fn workflow_response_session(
    response: &LocalDaemonResponse,
) -> Option<crate::session::RuntimeSession> {
    match response {
        LocalDaemonResponse::WorkflowCreated { session, .. }
        | LocalDaemonResponse::WorkflowDesignOpAccepted { session, .. }
        | LocalDaemonResponse::WorkflowAliased { session, .. }
        | LocalDaemonResponse::WorkflowPublicationCreated { session, .. }
        | LocalDaemonResponse::WorkflowPublicationDisabled { session, .. }
        | LocalDaemonResponse::WorkflowPublicationPairCodeCreated { session, .. }
        | LocalDaemonResponse::WorkflowPublicationSenderPaired { session, .. }
        | LocalDaemonResponse::WorkflowPublicationSenderRevoked { session, .. }
        | LocalDaemonResponse::WorkflowEndpointCreated { session, .. }
        | LocalDaemonResponse::WorkflowEndpointAliased { session, .. }
        | LocalDaemonResponse::WorkflowEndpointBound { session, .. }
        | LocalDaemonResponse::WorkflowNodeAdded { session, .. }
        | LocalDaemonResponse::WorkflowNodeRemoved { session, .. }
        | LocalDaemonResponse::WorkflowNodeInstructionsUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeCanCompleteRunUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeCanEmitIntermediateOutputUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeIntermediateOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowNodeMaxTurnsUpdated { session, .. }
        | LocalDaemonResponse::WorkflowEdgeAdded { session, .. }
        | LocalDaemonResponse::WorkflowEdgeRemoved { session, .. }
        | LocalDaemonResponse::WorkflowCanvasLayoutUpdated { session, .. }
        | LocalDaemonResponse::WorkflowRunInvoked { session, .. }
        | LocalDaemonResponse::WorkflowRunQueued { session, .. }
        | LocalDaemonResponse::WorkflowRunCancelled { session, .. }
        | LocalDaemonResponse::WorkflowRunResumed { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogCreated { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogUpdated { session, .. }
        | LocalDaemonResponse::WorkflowWatchdogRemoved { session, .. }
        | LocalDaemonResponse::WorkflowFlushContextUpdated { session, .. }
        | LocalDaemonResponse::WorkflowRunOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowIntermediateOutputSchemaUpdated { session, .. }
        | LocalDaemonResponse::WorkflowLaunchPolicyUpdated { session, .. }
        | LocalDaemonResponse::QueuedWorkflowLaunchRemoved { session, .. }
        | LocalDaemonResponse::QueuedWorkflowLaunchesCleared { session, .. }
        | LocalDaemonResponse::WorkflowTurnAcknowledged { session, .. } => Some(session.clone()),
        _ => None,
    }
}
