use super::*;

impl KernelRuntimeState {
    pub(crate) fn persist_workflow_code_run_event(
        &self,
        session_id: &str,
        caller_user_id: &str,
        controlled_by_metaagent_id: Option<&str>,
        result: &crate::workflow_code::WorkflowCodeRunResult,
    ) {
        let payload = workflow_code_run_event_payload(
            session_id,
            caller_user_id,
            controlled_by_metaagent_id,
            result,
        );
        if let Err(error) = self.owned.durable_state_store.append_event(
            "workflow_code.run",
            Some(result.apply.apply.workflow_id.clone()),
            payload,
        ) {
            crate::logging::warn_with_fields(
                "workflow_code.run",
                "failed to persist workflow-code run audit",
                serde_json::json!({
                    "session_id": session_id,
                    "workflow_id": &result.apply.apply.workflow_id,
                    "error": error.to_string(),
                }),
            );
        }
    }
}

impl KernelRuntimeOwnedState {
    pub(super) fn workflow_code_dispatch_refusal(
        &self,
        session_id: &str,
        code: Option<&crate::workflow_code::WorkflowCodeCompileAndApplyResult>,
        error: DaemonError,
    ) -> DaemonError {
        let Some(code) = code else {
            return error;
        };
        match self.rollback_workflow_code_run(session_id, code, None) {
            Ok(true) => workflow_code_refusal(error),
            Ok(false) => error,
            Err(rollback) => workflow_code_rollback_failed(error, rollback),
        }
    }

    pub(super) fn rollback_workflow_code_run(
        &self,
        session_id: &str,
        code: &crate::workflow_code::WorkflowCodeCompileAndApplyResult,
        queued_prompt_id: Option<&str>,
    ) -> Result<bool, DaemonError> {
        let workflow_id = &code.apply.workflow_id;
        let mutation = self.begin_managed_activity_mutation();
        let rolled_back = self
            .durable_state_store
            .with_workflow_runtime_transition_lock(|| {
                self.durable_state_store.require_writer_healthy()?;
                let mut sessions = self.session_store.write();
                let mut session = sessions.get_session(session_id)?;
                let Some(workflow) = session.workflow(workflow_id) else {
                    return Ok(None);
                };
                // Another invocation or an accepted gate entry can already own this workflow.
                if session.workflow_queued_prompts().iter().any(|prompt| {
                    prompt.workflow_id() == workflow_id && Some(prompt.id()) != queued_prompt_id
                }) || (workflow.gate().is_some()
                    && !self
                        .durable_state_store
                        .gate_entries(session.owner_user_id(), session_id, workflow_id, None, 1)?
                        .0
                        .is_empty())
                {
                    return Ok(None);
                }
                let mut runs = session
                    .workflow_runs()
                    .iter()
                    .filter(|run| run.workflow_id() == workflow_id);
                let claim = if let Some(run) = runs.next() {
                    if runs.next().is_some()
                        || queued_prompt_id.is_none()
                        || run.queue_item_id() != queued_prompt_id
                    {
                        return Ok(None);
                    }
                    let Some(claim) = self.workflow_entry_claims.try_claim(session_id, run.id())
                    else {
                        return Ok(None);
                    };
                    if !self
                        .workflow_entry_intent(session_id, run.id())?
                        .is_some_and(|intent| !intent.submitted)
                    {
                        return Ok(None);
                    }
                    Some(claim)
                } else {
                    None
                };
                // The caller owns this newly applied graph and has proved that no prompt was submitted.
                let generated_agents = code
                    .compile
                    .definition
                    .nodes
                    .iter()
                    .filter_map(|node| {
                        if matches!(
                            &node.agent,
                            crate::workflow_code::WorkflowCodeAgentBinding::Create(_)
                        ) {
                            code.apply.agent_ids.get(&node.handle)
                        } else {
                            None
                        }
                    })
                    .filter(|agent| {
                        !session
                            .workflows()
                            .iter()
                            .filter(|workflow| workflow.id() != workflow_id)
                            .any(|workflow| {
                                workflow
                                    .nodes()
                                    .iter()
                                    .any(|node| node.agent_id() == agent.as_str())
                            })
                            && session.active_prompt_for_agent(agent).is_none()
                            && !session
                                .queued_prompts()
                                .iter()
                                .any(|prompt| prompt.target_agent_id() == agent.as_str())
                    })
                    .collect::<Vec<_>>();
                session.discard_unadmitted_workflow(workflow_id);
                self.durable_state_store
                    .persist_workflow_runtime_transition(
                        &session,
                        "workflow_code_dispatch_refused",
                    )?;
                sessions.restore_session(session);
                drop(claim);
                Ok(Some(generated_agents))
            })?;
        let Some(generated_agents) = rolled_back else {
            return Ok(false);
        };
        mutation.record();
        for agent_id in generated_agents {
            let agent = self.agent_store.get_agent(agent_id)?;
            let destroyed = self.destroy_agent(agent_id, agent.owner_user_id())?;
            self.durable_state_store.append_event(
                "agent.deleted",
                Some(agent_id.clone()),
                serde_json::json!({ "agent": destroyed }),
            )?;
        }
        self.session_snapshot(session_id)?;
        Ok(true)
    }
}

pub(super) fn workflow_code_refusal(error: DaemonError) -> DaemonError {
    if crate::error::is_agent_deadline_refusal(&error) {
        return error;
    }
    DaemonError::LocalTransport {
        operation: "workflow_code.run",
        message: format!(
            "Dispatch refused. {error}. No workflow or run was created. Resolve the dispatch error and retry."
        ),
    }
}

pub(super) fn workflow_code_rollback_failed(
    error: DaemonError,
    rollback: DaemonError,
) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "workflow_code.run",
        message: format!(
            "Dispatch did not complete. {error}. Cleanup could not be confirmed: {rollback}. Check the room before retrying."
        ),
    }
}

pub(super) fn workflow_code_registry_for_session(
    app: &crate::app::DaemonApp,
    session_id: &str,
) -> Result<crate::workflow_code::WorkflowCodeArtifactRegistry, DaemonError> {
    let session = app.sessions().get_session(session_id)?;
    if app.config().room_agent_tools {
        return Ok(crate::workflow_code::WorkflowCodeArtifactRegistry::new(
            vec![app
                .config()
                .workflow_code_artifact_root()
                .join("rooms")
                .join(session.id())],
        ));
    }
    let mut roots = vec![app.config().workflow_code_artifact_root()];
    if let Some(root) = crate::workflow_code::WorkflowCodeArtifactRegistry::user_root() {
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    if !session.workspace_id().trim().is_empty() {
        roots.push(
            crate::workflow_code::WorkflowCodeArtifactRegistry::project_root(
                session.workspace_id(),
            ),
        );
    }
    Ok(crate::workflow_code::WorkflowCodeArtifactRegistry::new(
        roots,
    ))
}

pub(super) fn workflow_code_bindings_for_existing_workflow(
    app: &crate::app::DaemonApp,
    session_id: &str,
    workflow_ref: &str,
    definition: &crate::workflow_code::WorkflowCodeDefinition,
) -> Result<crate::workflow_code::WorkflowCodeApplyReport, DaemonError> {
    let session = app.sessions().get_session(session_id)?;
    let workflow = app
        .sessions()
        .resolve_workflow_ref(session_id, workflow_ref)?;
    if workflow.schemas().len() != definition.schemas.len()
        || workflow.nodes().len() != definition.nodes.len()
        || workflow.edges().len() != definition.edges.len()
        || workflow.endpoints().len() != definition.endpoints.len()
    {
        return Err(DaemonError::LocalTransport {
            operation: "workflow_code.bind",
            message: "generated source no longer matches the workflow structure".to_string(),
        });
    }
    let mut report = crate::workflow_code::WorkflowCodeApplyReport::for_workflow(workflow.id());
    for (source, current) in definition.schemas.iter().zip(workflow.schemas()) {
        report
            .schema_refs
            .insert(source.handle.clone(), current.id().to_string());
    }
    for (source, current) in definition.nodes.iter().zip(workflow.nodes()) {
        report
            .node_ids
            .insert(source.handle.clone(), current.id().to_string());
        report
            .agent_ids
            .insert(source.handle.clone(), current.agent_id().to_string());
    }
    for (source, current) in definition.edges.iter().zip(workflow.edges()) {
        report
            .edge_ids
            .insert(source.handle.clone(), current.id().to_string());
    }
    for (source, current) in definition.endpoints.iter().zip(workflow.endpoints()) {
        report
            .endpoint_ids
            .insert(source.handle.clone(), current.id().to_string());
    }
    let queues = session
        .workflow_prompt_queues()
        .iter()
        .filter(|queue| queue.workflow_id() == workflow.id())
        .collect::<Vec<_>>();
    let schedules = session
        .workflow_schedules()
        .iter()
        .filter(|schedule| schedule.workflow_id() == workflow.id())
        .collect::<Vec<_>>();
    let queues_match = if definition.queues.is_empty() {
        queues.len() == 1 && queues[0].alias() == "default"
    } else {
        queues.len() == definition.queues.len()
    };
    if !queues_match || schedules.len() != definition.schedules.len() {
        return Err(DaemonError::LocalTransport {
            operation: "workflow_code.bind",
            message: "generated source no longer matches workflow queues or schedules".to_string(),
        });
    }
    if definition.queues.is_empty() {
        report
            .queue_ids
            .insert("default".to_string(), queues[0].id().to_string());
    } else {
        for (source, current) in definition.queues.iter().zip(queues) {
            report
                .queue_ids
                .insert(source.handle.clone(), current.id().to_string());
        }
    }
    for (source, current) in definition.schedules.iter().zip(schedules) {
        report
            .schedule_ids
            .insert(source.handle.clone(), current.id().to_string());
    }
    Ok(report)
}

pub(super) fn workflow_registry_for_session(
    app: &crate::app::DaemonApp,
    session_id: &str,
) -> Result<crate::workflow_code::WorkflowRegistry, DaemonError> {
    let session = app.sessions().get_session(session_id)?;
    if app.config().room_agent_tools {
        return Ok(crate::workflow_code::WorkflowRegistry::new(
            None,
            Some(
                app.config()
                    .workflow_registry_root()
                    .join("rooms")
                    .join(session.id()),
            ),
        ));
    }
    let workspace_root = if !session.workspace_id().trim().is_empty() {
        Some(crate::workflow_code::WorkflowRegistry::workspace_root(
            session.workspace_id(),
        ))
    } else {
        None
    };
    Ok(crate::workflow_code::WorkflowRegistry::new(
        workspace_root,
        Some(app.config().workflow_registry_root()),
    ))
}

pub(super) fn workflow_registry_write_scope(
    app: &crate::app::DaemonApp,
    session_id: &str,
    requested: Option<crate::workflow_code::WorkflowRegistrySourceScope>,
) -> Result<crate::workflow_code::WorkflowRegistrySourceScope, DaemonError> {
    if app.config().room_agent_tools
        && requested == Some(crate::workflow_code::WorkflowRegistrySourceScope::Workspace)
    {
        return Err(crate::runtime::room_tool_admission::denied(
            "room agents cannot administer workspace or owner-wide registry entries",
        ));
    }
    if let Some(scope) = requested {
        if scope == crate::workflow_code::WorkflowRegistrySourceScope::Builtin {
            return Err(DaemonError::LocalTransport {
                operation: "workflow_registry.write_scope",
                message: "builtin workflow registry entries cannot be modified".to_string(),
            });
        }
        return Ok(scope);
    }
    app.sessions().get_session(session_id)?;
    Ok(crate::workflow_code::WorkflowRegistrySourceScope::User)
}

pub(super) struct WorkflowApplyContext<'a> {
    pub session_id: &'a str,
    pub provider_rebindings: &'a [crate::workflow_code::WorkflowCodeProviderRebinding],
    pub agent_rebindings: &'a [crate::workflow_code::WorkflowCodeAgentRebinding],
    pub caller_user_id: String,
    pub controlled_by_metaagent_id: Option<String>,
    pub operation: &'static str,
    pub run_endpoint: Option<Option<&'a str>>,
    pub run_queue: Option<&'a str>,
    pub authorize: &'a (dyn Fn() -> Result<(), DaemonError> + Send + Sync),
}

pub(super) fn workflow_registry_apply_result(
    app: &mut crate::app::DaemonApp,
    name: &str,
    entry: crate::workflow_code::WorkflowRegistryResolvedEntry,
    compile: crate::workflow_code::WorkflowCodeCompileResult,
    context: WorkflowApplyContext<'_>,
) -> Result<
    (
        crate::workflow_code::WorkflowRegistryEntryMetadata,
        crate::workflow_code::WorkflowCodeCompileAndApplyResult,
    ),
    DaemonError,
> {
    let WorkflowApplyContext {
        session_id,
        provider_rebindings,
        agent_rebindings,
        caller_user_id,
        controlled_by_metaagent_id,
        operation,
        run_endpoint,
        run_queue,
        authorize,
    } = context;

    authorize()?;
    let current = workflow_registry_for_session(app, session_id)?.resolve(name)?;
    if current.source != entry.source || current.metadata != entry.metadata {
        return Err(DaemonError::LocalTransport {
            operation,
            message: "workflow registry entry changed during compilation; retry".into(),
        });
    }
    let limits = app.config().workflow_code_limits();
    reject_invalid_workflow_code_run_compile(operation, &compile.validation)?;
    let metaagent_id = controlled_by_metaagent_id.as_deref();
    let (definition, validation) =
        crate::app::KernelSessionService::with_authorization(app, authorize)
            .validate_workflow_code_definition_with_rebindings(
                session_id,
                &compile.definition,
                &limits,
                provider_rebindings,
                agent_rebindings,
                metaagent_id,
            )?;
    reject_invalid_workflow_code_run_compile(operation, &validation)?;
    if let Some(endpoint) = run_endpoint {
        workflow_code_run_endpoint_preflight(&definition, endpoint, operation)?;
    }
    workflow_code_run_queue_preflight(&definition, run_queue, operation)?;
    let apply = crate::app::KernelSessionService::with_authorization(app, authorize)
        .apply_workflow_code_definition_with_alias_base(
            session_id,
            &definition,
            &limits,
            caller_user_id,
            controlled_by_metaagent_id,
            Some(name),
        )?;
    Ok((
        entry.metadata,
        crate::workflow_code::WorkflowCodeCompileAndApplyResult {
            compile: crate::workflow_code::WorkflowCodeCompileResult {
                definition,
                validation,
                logs: compile.logs,
                source_sha256: compile.source_sha256,
                source_spans: compile.source_spans,
            },
            apply,
        },
    ))
}

pub(super) fn workflow_code_artifact_apply_result(
    app: &mut crate::app::DaemonApp,
    artifact_name: &str,
    history_action: Option<crate::workflow_code::WorkflowCodeArtifactHistoryAction>,
    context: WorkflowApplyContext<'_>,
) -> Result<crate::workflow_code::WorkflowCodeCompileAndApplyResult, DaemonError> {
    let WorkflowApplyContext {
        session_id,
        provider_rebindings,
        agent_rebindings,
        caller_user_id,
        controlled_by_metaagent_id,
        operation,
        run_endpoint,
        run_queue,
        authorize,
    } = context;

    authorize()?;
    let artifact = {
        let registry = workflow_code_registry_for_session(app, session_id)?;
        registry
            .get(artifact_name)?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation,
                message: format!("workflow-code artifact `{artifact_name}` is not saved"),
            })?
    };
    let limits = app.config().workflow_code_limits();
    let metaagent_id = controlled_by_metaagent_id.as_deref();
    let (definition, validation) =
        crate::app::KernelSessionService::with_authorization(app, authorize)
            .validate_workflow_code_definition_with_rebindings(
                session_id,
                &artifact.definition,
                &limits,
                provider_rebindings,
                agent_rebindings,
                metaagent_id,
            )?;
    if !validation.ok {
        return Err(DaemonError::LocalTransport {
            operation,
            message: format!(
                "workflow-code artifact `{artifact_name}` is invalid for this target kernel: {}",
                validation
                    .diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.code.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }
    if let Some(endpoint) = run_endpoint {
        workflow_code_run_endpoint_preflight(&definition, endpoint, operation)?;
        workflow_code_run_entry_deadline(app, session_id, &definition, endpoint)?;
    }
    workflow_code_run_queue_preflight(&definition, run_queue, operation)?;
    let apply = crate::app::KernelSessionService::with_authorization(app, authorize)
        .apply_workflow_code_definition(
            session_id,
            &definition,
            &limits,
            caller_user_id.clone(),
            controlled_by_metaagent_id.clone(),
        )?;
    if let Some(action) = history_action {
        authorize()?;
        record_workflow_code_artifact_apply_history(
            app,
            session_id,
            artifact_name,
            &artifact.metadata.source_sha256,
            workflow_code_artifact_actor(&caller_user_id, metaagent_id),
            action,
            &apply,
        )?;
    }
    Ok(crate::workflow_code::WorkflowCodeCompileAndApplyResult {
        compile: crate::workflow_code::WorkflowCodeCompileResult {
            definition,
            validation,
            logs: String::new(),
            source_sha256: artifact.metadata.source_sha256,
            source_spans: std::collections::BTreeMap::new(),
        },
        apply,
    })
}

pub(super) fn record_workflow_code_artifact_apply_history(
    app: &crate::app::DaemonApp,
    session_id: &str,
    artifact_name: &str,
    source_sha256: &str,
    actor: crate::workflow_code::WorkflowCodeArtifactActor,
    action: crate::workflow_code::WorkflowCodeArtifactHistoryAction,
    apply: &crate::workflow_code::WorkflowCodeApplyReport,
) -> Result<(), DaemonError> {
    let registry = workflow_code_registry_for_session(app, session_id)?;
    let Some(current) = registry.get(artifact_name)? else {
        return Ok(());
    };
    if current.metadata.source_sha256 != source_sha256 {
        return Ok(());
    }
    if let Some(id) = actor.metaagent_id.as_deref() {
        if app.config().room_agent_tools
            && action == crate::workflow_code::WorkflowCodeArtifactHistoryAction::Run
        {
            let caller = app.agents().get_agent(id)?;
            // Running public source may create a caller-owned workflow without editing peer provenance.
            if !crate::runtime::room_tool_admission::owns_object(
                &caller,
                current
                    .metadata
                    .provenance
                    .created_by
                    .metaagent_id
                    .as_deref(),
                &app.agents().get_session_agents(session_id),
            ) {
                return Ok(());
            }
        }
    }
    registry.record_apply_history(artifact_name, actor, action, apply)?;
    Ok(())
}

pub(super) fn workflow_code_artifact_actor(
    caller_user_id: &str,
    caller_metaagent_id: Option<&str>,
) -> crate::workflow_code::WorkflowCodeArtifactActor {
    crate::workflow_code::WorkflowCodeArtifactActor::new(
        caller_user_id.to_string(),
        caller_metaagent_id.map(str::to_string),
    )
}

pub(super) fn reject_invalid_workflow_code_artifact_validation(
    operation: &'static str,
    validation: &crate::workflow_code::WorkflowCodeValidationReport,
) -> Result<(), DaemonError> {
    if validation.ok {
        return Ok(());
    }
    let diagnostics = validation
        .diagnostics
        .iter()
        .map(|diagnostic| {
            diagnostic
                .handle
                .as_deref()
                .map(|handle| format!("{}:{handle}", diagnostic.code))
                .unwrap_or_else(|| diagnostic.code.clone())
        })
        .collect::<Vec<_>>()
        .join(", ");
    Err(DaemonError::LocalTransport {
        operation,
        message: format!("workflow-code artifact validation failed: {diagnostics}"),
    })
}

pub(super) fn reject_invalid_workflow_code_run_compile(
    operation: &'static str,
    validation: &crate::workflow_code::WorkflowCodeValidationReport,
) -> Result<(), DaemonError> {
    if validation.ok {
        return Ok(());
    }
    let diagnostics = validation
        .diagnostics
        .iter()
        .map(|diagnostic| {
            diagnostic
                .handle
                .as_deref()
                .map(|handle| format!("{}:{handle}", diagnostic.code))
                .unwrap_or_else(|| diagnostic.code.clone())
        })
        .collect::<Vec<_>>()
        .join(", ");
    Err(DaemonError::LocalTransport {
        operation,
        message: format!("workflow-code definition is invalid: {diagnostics}"),
    })
}

pub(super) fn workflow_code_run_endpoint_preflight(
    definition: &crate::workflow_code::WorkflowCodeDefinition,
    endpoint: Option<&str>,
    operation: &'static str,
) -> Result<(), DaemonError> {
    if let Some(endpoint) = endpoint {
        if definition
            .endpoints
            .iter()
            .any(|definition_endpoint| definition_endpoint.handle == endpoint)
        {
            return Ok(());
        }
        return Err(DaemonError::LocalTransport {
            operation,
            message: format!("workflow-code endpoint handle `{endpoint}` is not defined"),
        });
    }
    if definition.endpoints.len() == 1 {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation,
        message: format!(
            "workflow-code defines {} endpoints; pass endpoint as a script handle",
            definition.endpoints.len()
        ),
    })
}

pub(super) fn workflow_code_run_entry_deadline(
    app: &crate::app::DaemonApp,
    session_id: &str,
    definition: &crate::workflow_code::WorkflowCodeDefinition,
    endpoint: Option<&str>,
) -> Result<(), DaemonError> {
    if definition.workflow.gate.is_some() {
        // A gate chooses its routes later, so there is no entry agent yet.
        return Ok(());
    }
    let entry = definition
        .endpoints
        .iter()
        .find(|candidate| endpoint.is_none_or(|handle| candidate.handle == handle))
        .and_then(|endpoint| {
            definition
                .nodes
                .iter()
                .find(|node| node.handle == endpoint.entry_node)
        });
    if let Some(crate::workflow_code::WorkflowCodeAgentBinding::Existing(agent)) =
        entry.map(|node| &node.agent)
    {
        app.durable_state_store()
            .ensure_agent_before_deadline(session_id, &agent.agent_ref)?;
    }
    Ok(())
}

pub(super) fn workflow_code_run_queue_preflight(
    definition: &crate::workflow_code::WorkflowCodeDefinition,
    queue_ref: Option<&str>,
    operation: &'static str,
) -> Result<(), DaemonError> {
    let Some(queue_ref) = queue_ref else {
        return Ok(());
    };
    if queue_ref == "default"
        || definition
            .queues
            .iter()
            .any(|definition_queue| definition_queue.handle == queue_ref)
    {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation,
        message: format!("workflow-code queue handle `{queue_ref}` is not defined"),
    })
}

fn workflow_code_run_event_payload(
    session_id: &str,
    caller_user_id: &str,
    controlled_by_metaagent_id: Option<&str>,
    result: &crate::workflow_code::WorkflowCodeRunResult,
) -> serde_json::Value {
    match &result.invocation {
        crate::workflow_code::WorkflowCodeRunInvocation::EntryAccepted { entry } => {
            serde_json::json!({
                "outcome": "entry_accepted", "apply": &result.apply.apply, "entry": entry,
            })
        }
        crate::workflow_code::WorkflowCodeRunInvocation::Started {
            workflow_run,
            workflow,
            endpoint,
        } => serde_json::json!({
            "session_id": session_id,
            "caller_user_id": caller_user_id,
            "controlled_by_metaagent_id": controlled_by_metaagent_id,
            "outcome": "invoked",
            "workflow_id": workflow.id(),
            "endpoint_id": endpoint.id(),
            "workflow_run_id": workflow_run.id(),
            "apply": &result.apply.apply,
        }),
        crate::workflow_code::WorkflowCodeRunInvocation::Enqueued {
            queued_prompt,
            workflow,
            endpoint,
        } => serde_json::json!({
            "session_id": session_id,
            "caller_user_id": caller_user_id,
            "controlled_by_metaagent_id": controlled_by_metaagent_id,
            "outcome": "enqueued",
            "workflow_id": workflow.id(),
            "endpoint_id": endpoint.id(),
            "queued_prompt_id": queued_prompt.id(),
            "queue_id": queued_prompt.queue_id(),
            "apply": &result.apply.apply,
        }),
    }
}

pub(super) fn workflow_code_endpoint_ref(
    apply_report: &crate::workflow_code::WorkflowCodeApplyReport,
    endpoint: Option<String>,
) -> Result<String, DaemonError> {
    match endpoint {
        Some(endpoint) => Ok(apply_report
            .endpoint_ids
            .get(&endpoint)
            .cloned()
            .unwrap_or(endpoint)),
        None if apply_report.endpoint_ids.len() == 1 => Ok(apply_report
            .endpoint_ids
            .values()
            .next()
            .expect("length checked")
            .clone()),
        None => Err(DaemonError::LocalTransport {
            operation: "workflow_code.run",
            message: format!(
                "workflow-code defines {} endpoints; pass endpoint as a script handle or kernel endpoint ref",
                apply_report.endpoint_ids.len()
            ),
        }),
    }
}

pub(super) fn workflow_code_queue_ref(
    apply_report: &crate::workflow_code::WorkflowCodeApplyReport,
    queue_ref: Option<String>,
) -> Option<String> {
    queue_ref.map(|queue_ref| {
        apply_report
            .queue_ids
            .get(&queue_ref)
            .cloned()
            .unwrap_or(queue_ref)
    })
}

pub(super) fn workflow_code_invocation_prompt(
    request_prompt: &str,
    default_prompt: Option<&str>,
) -> String {
    let request_prompt = request_prompt.trim();
    if !request_prompt.is_empty() {
        return request_prompt.to_string();
    }
    let default_prompt = default_prompt.unwrap_or_default().trim();
    if !default_prompt.is_empty() {
        return default_prompt.to_string();
    }
    "Run the workflow exactly as instructed.".to_string()
}
