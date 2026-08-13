use std::collections::BTreeSet;

use crate::session::{
    WorkflowPublicationSnapshot, WorkflowPublicationSourceSessionSnapshot,
    WORKFLOW_PUBLICATION_KIND_EVENT_BASED, WORKFLOW_PUBLICATION_KIND_INGRESS,
    WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY, WORKFLOW_PUBLICATION_WORKSPACE_ROOT,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::*;

impl SessionService {
    #[allow(clippy::too_many_arguments)]
    pub fn create_workflow_publication(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        queue_ref: Option<String>,
        alias: Option<String>,
        kind: Option<String>,
        route: Option<String>,
        methods: Vec<String>,
        transport: Option<Value>,
        parser: Option<Value>,
        input_schema: Option<Value>,
        trace_exposure: Option<Value>,
        mode: Option<String>,
        sync_timeout_ms: Option<u64>,
        poll_ms: Option<u64>,
        created_by_user_id: String,
    ) -> Result<WorkflowPublicationDefinition, DaemonError> {
        let source_agents = self.get_session(session_id)?.agents().to_vec();
        self.create_workflow_publication_idempotent(
            session_id,
            workflow_ref,
            endpoint_ref,
            None,
            None,
            queue_ref,
            alias,
            kind,
            route,
            methods,
            transport,
            parser,
            input_schema,
            trace_exposure,
            mode,
            sync_timeout_ms,
            poll_ms,
            source_agents,
            created_by_user_id,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_workflow_publication_idempotent(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        endpoint_ref: &str,
        expected_workflow_revision: Option<u64>,
        operation_key: Option<String>,
        queue_ref: Option<String>,
        alias: Option<String>,
        kind: Option<String>,
        route: Option<String>,
        methods: Vec<String>,
        transport: Option<Value>,
        parser: Option<Value>,
        input_schema: Option<Value>,
        trace_exposure: Option<Value>,
        mode: Option<String>,
        sync_timeout_ms: Option<u64>,
        poll_ms: Option<u64>,
        source_agents: Vec<crate::agent::AgentInstance>,
        created_by_user_id: String,
    ) -> Result<WorkflowPublicationDefinition, DaemonError> {
        let operation_key = normalize_workflow_publication_operation_key(operation_key)?;
        let publication_kind = resolve_workflow_publication_kind(kind.as_deref(), &transport)?;
        let normalized_queue_ref = normalize_workflow_publication_queue_ref(queue_ref);
        let alias = normalize_workflow_publication_alias(alias)?;
        let creation_request_digest = workflow_publication_creation_request_digest(
            session_id,
            workflow_ref,
            endpoint_ref,
            expected_workflow_revision,
            &normalized_queue_ref,
            alias.as_deref(),
            &publication_kind,
            route.as_deref(),
            &methods,
            transport.as_ref(),
            parser.as_ref(),
            input_schema.as_ref(),
            trace_exposure.as_ref(),
            mode.as_deref(),
            sync_timeout_ms,
            poll_ms,
        )?;
        if let Some(operation_key) = operation_key.as_deref() {
            if let Some(existing) = self
                .get_session(session_id)?
                .workflow_publications()
                .iter()
                .find(|publication| {
                    publication.creation_operation_key() == Some(operation_key)
                        && publication.created_by_user_id() == created_by_user_id
                })
            {
                if existing.creation_request_digest() == Some(creation_request_digest.as_str()) {
                    return Ok(existing.clone());
                }
                return invalid_workflow_publication_option(
                    "workflow publication operation key is already bound to different publication choices",
                );
            }
        }
        let workflow = self.resolve_workflow_ref(session_id, workflow_ref)?;
        if let Some(expected_revision) = expected_workflow_revision {
            if workflow.revision() != expected_revision {
                return Err(DaemonError::WorkflowRevisionConflict {
                    session_id: session_id.to_string(),
                    workflow_id: workflow.id().to_string(),
                    expected_revision,
                    current_revision: workflow.revision(),
                });
            }
        }
        let endpoint =
            self.resolve_workflow_endpoint_ref(session_id, workflow.id(), endpoint_ref)?;
        validate_workflow_publication_trace_exposure(&trace_exposure, &workflow)?;
        validate_workflow_publication_options(
            &publication_kind,
            &transport,
            route.as_deref(),
            &methods,
            &parser,
            mode.as_deref(),
        )?;
        self.resolve_workflow_prompt_queue_ref(session_id, workflow.id(), &normalized_queue_ref)?;
        if publication_kind == WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY {
            self.validate_schedule_only_workflow_publication(
                session_id,
                workflow.id(),
                endpoint.id(),
                &normalized_queue_ref,
            )?;
        }
        if let Some(alias) = alias.as_deref() {
            self.ensure_workflow_publication_alias_available(session_id, alias)?;
        }
        let source_snapshot = self.workflow_publication_source_snapshot(
            session_id,
            workflow.clone(),
            endpoint.clone(),
            source_agents,
        )?;
        let source_snapshot_digest =
            source_snapshot
                .digest()
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "create workflow publication",
                    message: format!("failed to encode workflow publication snapshot: {error}"),
                })?;
        let publication = WorkflowPublicationDefinition::new_immutable(
            self.next_workflow_publication_id(),
            session_id.to_string(),
            workflow.id().to_string(),
            endpoint.id().to_string(),
            Some(normalized_queue_ref),
            alias,
            publication_kind,
            route,
            methods,
            transport,
            parser,
            input_schema,
            trace_exposure,
            mode,
            sync_timeout_ms,
            poll_ms,
            workflow.revision(),
            source_snapshot_digest,
            operation_key,
            Some(creation_request_digest),
            created_by_user_id,
        );
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        Ok(session.create_workflow_publication(publication, Some(source_snapshot)))
    }

    fn workflow_publication_source_snapshot(
        &self,
        session_id: &str,
        workflow: WorkflowDefinition,
        endpoint: WorkflowEndpointDefinition,
        source_agents: Vec<crate::agent::AgentInstance>,
    ) -> Result<WorkflowPublicationSnapshot, DaemonError> {
        let session = self.get_session(session_id)?;
        let node_agent_ids = workflow
            .nodes()
            .iter()
            .map(|node| node.agent_id().to_string())
            .collect::<BTreeSet<_>>();
        let agents = source_agents
            .into_iter()
            .filter(|agent| node_agent_ids.contains(agent.id()))
            .map(|agent| {
                agent.canonicalized_for_publication_package(WORKFLOW_PUBLICATION_WORKSPACE_ROOT)
            })
            .collect::<Vec<_>>();
        let missing_agent_ids = node_agent_ids
            .iter()
            .filter(|agent_id| !agents.iter().any(|agent| agent.id() == agent_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !missing_agent_ids.is_empty() {
            return Err(DaemonError::LocalTransport {
                operation: "create workflow publication",
                message: format!(
                    "workflow publication snapshot is missing agents: {}",
                    missing_agent_ids.join(", ")
                ),
            });
        }
        Ok(WorkflowPublicationSnapshot {
            schema_version: 1,
            captured_at_ms: Some(unix_epoch_ms()),
            source_session: Some(WorkflowPublicationSourceSessionSnapshot {
                id: Some(session.id().to_string()),
                alias: session.alias().map(str::to_string),
                workspace_id: WORKFLOW_PUBLICATION_WORKSPACE_ROOT.to_string(),
                worktree_id: WORKFLOW_PUBLICATION_WORKSPACE_ROOT.to_string(),
            }),
            workflow: workflow.clone(),
            endpoint: Some(endpoint),
            queues: session
                .workflow_prompt_queues()
                .iter()
                .filter(|queue| queue.workflow_id() == workflow.id())
                .cloned()
                .collect(),
            schedules: session
                .workflow_schedules()
                .iter()
                .filter(|schedule| schedule.workflow_id() == workflow.id())
                .cloned()
                .collect(),
            agents,
        })
    }

    fn validate_schedule_only_workflow_publication(
        &self,
        session_id: &str,
        workflow_id: &str,
        endpoint_id: &str,
        queue_ref: &str,
    ) -> Result<(), DaemonError> {
        let session = self.get_session(session_id)?;
        let queue_id =
            self.resolve_workflow_prompt_queue_ref(session_id, workflow_id, queue_ref)?;
        let has_schedule = session.workflow_schedules().iter().any(|schedule| {
            let schedule_queue_matches = match schedule.queue_id() {
                Some(schedule_queue_id) => schedule_queue_id == queue_id.as_str(),
                None => queue_id == "default",
            };
            schedule.workflow_id() == workflow_id
                && schedule.endpoint_id() == endpoint_id
                && schedule.enabled()
                && schedule_queue_matches
        });
        if has_schedule {
            return Ok(());
        }
        invalid_workflow_publication_option(
            "schedule_only publications require an enabled schedule for the selected endpoint and queue",
        )
    }

    pub fn list_workflow_publications(
        &self,
        session_id: &str,
    ) -> Result<Vec<WorkflowPublicationDefinition>, DaemonError> {
        Ok(self
            .get_session(session_id)?
            .workflow_publications()
            .to_vec())
    }

    pub fn resolve_workflow_publication_ref(
        &self,
        session_id: &str,
        publication_ref: &str,
    ) -> Result<WorkflowPublicationDefinition, DaemonError> {
        let normalized_ref = publication_ref.trim().to_lowercase();
        let session = self.get_session(session_id)?;
        let publications = session.workflow_publications();
        if let Some(publication) = publications
            .iter()
            .find(|publication| publication.id() == normalized_ref)
        {
            return Ok(publication.clone());
        }
        if let Some(publication) = publications
            .iter()
            .find(|publication| publication.alias() == Some(normalized_ref.as_str()))
        {
            return Ok(publication.clone());
        }
        let id_matches = publications
            .iter()
            .filter(|publication| publication.id().starts_with(&normalized_ref))
            .cloned()
            .collect::<Vec<_>>();
        if id_matches.len() == 1 {
            return Ok(id_matches[0].clone());
        }
        let alias_matches = publications
            .iter()
            .filter(|publication| {
                publication
                    .alias()
                    .is_some_and(|alias| alias.starts_with(normalized_ref.as_str()))
            })
            .cloned()
            .collect::<Vec<_>>();
        if alias_matches.len() == 1 {
            return Ok(alias_matches[0].clone());
        }
        Err(DaemonError::LocalTransport {
            operation: "resolve workflow publication",
            message: format!("workflow publication `{publication_ref}` was not found"),
        })
    }

    pub(crate) fn resolve_workflow_publication_snapshot(
        &self,
        session_id: &str,
        publication_id: &str,
    ) -> Result<Option<WorkflowPublicationSnapshot>, DaemonError> {
        Ok(self
            .get_session(session_id)?
            .workflow_publication_snapshot(publication_id)
            .cloned())
    }

    pub fn disable_workflow_publication(
        &mut self,
        session_id: &str,
        publication_ref: &str,
    ) -> Result<WorkflowPublicationDefinition, DaemonError> {
        let publication_id = self
            .resolve_workflow_publication_ref(session_id, publication_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let publication = {
            let publication = session
                .workflow_publication_mut(&publication_id)
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "disable workflow publication",
                    message: format!("workflow publication `{publication_ref}` was not found"),
                })?;
            publication.disable();
            publication.clone()
        };
        let binding_ids = session
            .workflow_event_bindings()
            .iter()
            .filter(|binding| binding.publication_id == publication_id && binding.active())
            .map(|binding| binding.id.clone())
            .collect::<Vec<_>>();
        for binding_id in binding_ids {
            if let Some(binding) = session.workflow_event_binding_mut(&binding_id) {
                binding.set_status(crate::session::WorkflowEventBindingStatus::Tombstoned);
            }
        }
        Ok(publication)
    }

    pub fn register_workflow_publication_endpoint(
        &mut self,
        session_id: &str,
        publication_ref: &str,
        status: impl Into<String>,
        open_url: impl Into<String>,
        deployment: Value,
    ) -> Result<WorkflowPublicationDefinition, DaemonError> {
        let publication_id = self
            .resolve_workflow_publication_ref(session_id, publication_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let publication = session
            .workflow_publication_mut(&publication_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "register workflow publication endpoint",
                message: format!("workflow publication `{publication_ref}` was not found"),
            })?;
        publication.mark_served(status, open_url, deployment);
        Ok(publication.clone())
    }

    pub fn mark_workflow_publication_runtime_status(
        &mut self,
        session_id: &str,
        publication_ref: &str,
        status: impl Into<String>,
        open_url: Option<Option<String>>,
        deployment: Option<Value>,
    ) -> Result<WorkflowPublicationDefinition, DaemonError> {
        let status = status.into();
        let publication_id = self
            .resolve_workflow_publication_ref(session_id, publication_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let runtime_observability =
            session
                .workflow_publication(&publication_id)
                .map(|publication| {
                    workflow_publication_runtime_observability(
                        session,
                        publication,
                        runtime_reachability_for_status(&status),
                    )
                });
        let publication = session
            .workflow_publication_mut(&publication_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "mark workflow publication runtime status",
                message: format!("workflow publication `{publication_ref}` was not found"),
            })?;
        publication.mark_runtime_status(status, open_url, deployment);
        if let Some(runtime_observability) = runtime_observability {
            publication.set_runtime_observability(
                runtime_observability.runtime,
                runtime_observability.schedules,
                runtime_observability.latest_run,
                runtime_observability.recent_runs,
                runtime_observability.latest_output,
            );
        }
        Ok(publication.clone())
    }

    pub fn mark_workflow_publication_runtime_error(
        &mut self,
        session_id: &str,
        publication_ref: &str,
        message: impl Into<String>,
    ) -> Result<WorkflowPublicationDefinition, DaemonError> {
        let message = message.into();
        let publication_id = self
            .resolve_workflow_publication_ref(session_id, publication_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let runtime_observability =
            session
                .workflow_publication(&publication_id)
                .map(|publication| {
                    workflow_publication_runtime_observability(
                        session,
                        publication,
                        Some(serde_json::json!({
                            "reachable": false,
                            "error": message.clone(),
                        })),
                    )
                });
        let publication = session
            .workflow_publication_mut(&publication_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "mark workflow publication runtime error",
                message: format!("workflow publication `{publication_ref}` was not found"),
            })?;
        publication.mark_runtime_error(message);
        if let Some(runtime_observability) = runtime_observability {
            publication.set_runtime_observability(
                runtime_observability.runtime,
                runtime_observability.schedules,
                runtime_observability.latest_run,
                runtime_observability.recent_runs,
                runtime_observability.latest_output,
            );
        }
        Ok(publication.clone())
    }

    pub fn set_workflow_publication_runtime_run_observability(
        &mut self,
        session_id: &str,
        publication_ref: &str,
        latest_run: Option<Value>,
        recent_runs: Vec<Value>,
        latest_output: Option<Value>,
    ) -> Result<WorkflowPublicationDefinition, DaemonError> {
        let publication_id = self
            .resolve_workflow_publication_ref(session_id, publication_ref)?
            .id()
            .to_string();
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let publication = session
            .workflow_publication_mut(&publication_id)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "set workflow publication runtime run observability",
                message: format!("workflow publication `{publication_ref}` was not found"),
            })?;
        publication.set_runtime_run_observability(latest_run, recent_runs, latest_output);
        Ok(publication.clone())
    }
}

fn validate_workflow_publication_trace_exposure(
    trace_exposure: &Option<Value>,
    workflow: &WorkflowDefinition,
) -> Result<(), DaemonError> {
    let Some(value) = trace_exposure else {
        return Ok(());
    };
    let Some(object) = value.as_object() else {
        return Err(DaemonError::InvalidWorkflowPublicationTraceExposure {
            message: "`trace_exposure` must be an object".to_string(),
        });
    };
    let Some(nodes_value) = object.get("nodes") else {
        return Ok(());
    };
    let Some(nodes_object) = nodes_value.as_object() else {
        return Err(DaemonError::InvalidWorkflowPublicationTraceExposure {
            message: "`trace_exposure.nodes` must be an object keyed by workflow node id"
                .to_string(),
        });
    };
    let known_node_ids = workflow
        .nodes()
        .iter()
        .map(|node| node.id())
        .collect::<std::collections::BTreeSet<_>>();
    for (node_id, levels_value) in nodes_object {
        if !known_node_ids.contains(node_id.as_str()) {
            return Err(DaemonError::InvalidWorkflowPublicationTraceExposure {
                message: format!("unknown workflow node id `{node_id}`"),
            });
        }
        let Some(levels) = levels_value.as_array() else {
            return Err(DaemonError::InvalidWorkflowPublicationTraceExposure {
                message: format!("trace levels for node `{node_id}` must be an array"),
            });
        };
        for level in levels {
            let Some(level) = level.as_str() else {
                return Err(DaemonError::InvalidWorkflowPublicationTraceExposure {
                    message: format!("trace level for node `{node_id}` must be a string"),
                });
            };
            if !matches!(
                level,
                "output_summary" | "assistant_messages" | "thinking" | "tool_use"
            ) {
                return Err(DaemonError::InvalidWorkflowPublicationTraceExposure {
                    message: format!("unknown trace exposure level `{level}` for node `{node_id}`"),
                });
            }
        }
    }
    Ok(())
}

struct WorkflowPublicationRuntimeObservability {
    runtime: Option<Value>,
    schedules: Vec<Value>,
    latest_run: Option<Value>,
    recent_runs: Vec<Value>,
    latest_output: Option<Value>,
}

fn workflow_publication_runtime_observability(
    session: &RuntimeSession,
    publication: &WorkflowPublicationDefinition,
    runtime: Option<Value>,
) -> WorkflowPublicationRuntimeObservability {
    let queue_refs = workflow_publication_queue_reference_set(session, publication);
    let mut schedules = session
        .workflow_schedules()
        .iter()
        .filter(|schedule| {
            if schedule.workflow_id() != publication.workflow_id()
                || schedule.endpoint_id() != publication.endpoint_id()
            {
                return false;
            }
            schedule
                .queue_id()
                .is_none_or(|queue_id| queue_refs.contains(queue_id))
        })
        .filter_map(|schedule| serde_json::to_value(schedule).ok())
        .collect::<Vec<_>>();
    schedules.sort_by_key(|schedule| {
        schedule
            .get("next_run_at_ms")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX)
    });

    let mut runs = session
        .workflow_runs()
        .iter()
        .filter(|run| workflow_run_matches_publication(run, publication))
        .cloned()
        .collect::<Vec<_>>();
    runs.sort_by_key(|run| std::cmp::Reverse(workflow_run_sort_time(run)));

    let latest_run = runs
        .first()
        .and_then(|run| workflow_publication_visible_run_value(publication, run));
    let recent_runs = runs
        .iter()
        .take(5)
        .filter_map(|run| workflow_publication_visible_run_value(publication, run))
        .collect::<Vec<_>>();
    let latest_output = runs.iter().find_map(workflow_run_latest_output_value);

    WorkflowPublicationRuntimeObservability {
        runtime,
        schedules,
        latest_run,
        recent_runs,
        latest_output,
    }
}

fn workflow_publication_queue_reference_set(
    session: &RuntimeSession,
    publication: &WorkflowPublicationDefinition,
) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    let queue_ref = publication.queue_ref().unwrap_or("default").trim();
    let queue_ref = if queue_ref.is_empty() {
        "default"
    } else {
        queue_ref
    };
    refs.insert(queue_ref.to_string());
    if let Some(queue) = session.workflow_prompt_queues().iter().find(|candidate| {
        candidate.workflow_id() == publication.workflow_id()
            && (candidate.id() == queue_ref || candidate.alias() == queue_ref)
    }) {
        refs.insert(queue.id().to_string());
        refs.insert(queue.alias().to_string());
    }
    refs
}

fn workflow_run_matches_publication(
    run: &WorkflowRun,
    publication: &WorkflowPublicationDefinition,
) -> bool {
    if let Some(invocation) = run.publication_invocation() {
        return invocation.publication_id == publication.id();
    }
    if run.workflow_id() != publication.workflow_id()
        || run.endpoint_id() != publication.endpoint_id()
    {
        return false;
    }
    true
}

fn workflow_run_sort_time(run: &WorkflowRun) -> u64 {
    let Ok(value) = serde_json::to_value(run) else {
        return 0;
    };
    ["completed_at_ms", "started_at_ms", "created_at_ms"]
        .iter()
        .find_map(|field| value.get(*field).and_then(Value::as_u64))
        .unwrap_or(0)
}

fn workflow_publication_visible_run_value(
    publication: &WorkflowPublicationDefinition,
    run: &WorkflowRun,
) -> Option<Value> {
    let value = serde_json::to_value(run).ok()?;
    let policy = publication
        .trace_exposure()
        .and_then(|trace| trace.get("nodes"))
        .and_then(Value::as_object);
    let mut visible = serde_json::Map::new();
    copy_json_fields(
        &mut visible,
        &value,
        &[
            "id",
            "status",
            "workflow_id",
            "endpoint_id",
            "publication_invocation",
            "completed_by_node_run_id",
            "created_at_ms",
            "completed_at_ms",
            "final_output",
            "intermediate_outputs",
        ],
    );
    if let Some(node_runs) = value.get("node_runs").and_then(Value::as_array) {
        visible.insert(
            "node_runs".to_string(),
            Value::Array(
                node_runs
                    .iter()
                    .filter_map(|node_run| visible_node_run_value(node_run, policy))
                    .collect(),
            ),
        );
    }
    if let Some(messages) = value.get("messages").and_then(Value::as_array) {
        let visible_messages = messages
            .iter()
            .filter(|message| {
                let Some(source_node_run_id) =
                    message.get("source_node_run_id").and_then(Value::as_str)
                else {
                    return false;
                };
                node_id_for_node_run(&value, source_node_run_id).is_some_and(|node_id| {
                    trace_level_visible(policy, node_id, "assistant_messages")
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        visible.insert("messages".to_string(), Value::Array(visible_messages));
    }
    Some(Value::Object(visible))
}

fn visible_node_run_value(
    node_run: &Value,
    policy: Option<&serde_json::Map<String, Value>>,
) -> Option<Value> {
    let node_id = node_run.get("node_id").and_then(Value::as_str)?;
    let mut visible = serde_json::Map::new();
    copy_json_fields(
        &mut visible,
        node_run,
        &["id", "node_id", "agent_id", "status", "completed_at_ms"],
    );
    if trace_level_visible(policy, node_id, "output_summary") {
        copy_json_fields(&mut visible, node_run, &["summary"]);
        if let Some(summary) = node_run
            .get("completion")
            .and_then(|completion| completion.get("summary"))
            .cloned()
        {
            let mut completion = serde_json::Map::new();
            completion.insert("summary".to_string(), summary);
            visible.insert("completion".to_string(), Value::Object(completion));
        }
    }
    if trace_level_visible(policy, node_id, "assistant_messages") {
        if let Some(output) = node_run
            .get("completion")
            .and_then(|completion| completion.get("output"))
            .cloned()
        {
            let completion = visible
                .entry("completion".to_string())
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            if let Some(completion) = completion.as_object_mut() {
                completion.insert("output".to_string(), output);
            }
        }
    }
    if trace_level_visible(policy, node_id, "thinking") {
        copy_json_fields(&mut visible, node_run, &["thinking_traces"]);
    }
    if trace_level_visible(policy, node_id, "tool_use") {
        let runtime_tool_calls = node_run
            .get("turn_envelope")
            .and_then(|envelope| envelope.get("runtime_tool_calls"))
            .and_then(Value::as_array)
            .map(|tool_calls| {
                Value::Array(
                    tool_calls
                        .iter()
                        .map(visible_runtime_tool_call_value)
                        .collect(),
                )
            })
            .unwrap_or_else(|| Value::Array(Vec::new()));
        let mut turn_envelope = serde_json::Map::new();
        turn_envelope.insert("runtime_tool_calls".to_string(), runtime_tool_calls);
        visible.insert("turn_envelope".to_string(), Value::Object(turn_envelope));
    }
    Some(Value::Object(visible))
}

fn visible_runtime_tool_call_value(tool_call: &Value) -> Value {
    let mut visible = serde_json::Map::new();
    copy_json_fields(
        &mut visible,
        tool_call,
        &["tool_name", "ok", "timestamp_ms", "error"],
    );
    Value::Object(visible)
}

fn trace_level_visible(
    policy: Option<&serde_json::Map<String, Value>>,
    node_id: &str,
    level: &str,
) -> bool {
    policy
        .and_then(|nodes| nodes.get(node_id))
        .and_then(Value::as_array)
        .is_some_and(|levels| {
            levels
                .iter()
                .any(|candidate| candidate.as_str() == Some(level))
        })
}

fn node_id_for_node_run<'a>(run: &'a Value, node_run_id: &str) -> Option<&'a str> {
    run.get("node_runs")
        .and_then(Value::as_array)?
        .iter()
        .find(|node_run| node_run.get("id").and_then(Value::as_str) == Some(node_run_id))?
        .get("node_id")
        .and_then(Value::as_str)
}

fn copy_json_fields(target: &mut serde_json::Map<String, Value>, source: &Value, fields: &[&str]) {
    for field in fields {
        if let Some(value) = source.get(*field) {
            target.insert((*field).to_string(), value.clone());
        }
    }
}

fn workflow_run_latest_output_value(run: &WorkflowRun) -> Option<Value> {
    if let Some(output) = run.final_output() {
        return Some(serde_json::json!({
            "kind": "final",
            "message": serde_json::to_value(output).ok()?,
            "artifacts": [],
        }));
    }
    run.intermediate_outputs()
        .iter()
        .max_by_key(|output| {
            serde_json::to_value(output)
                .ok()
                .and_then(|value| value.get("timestamp_ms").and_then(Value::as_u64))
                .unwrap_or(0)
        })
        .and_then(|output| {
            let output_value = serde_json::to_value(output).ok()?;
            Some(serde_json::json!({
                "kind": "partial",
                "message": output_value.get("output").cloned().unwrap_or(Value::Null),
                "artifacts": [],
                "intermediate_output_id": output_value.get("id").cloned().unwrap_or(Value::Null),
            }))
        })
}

fn runtime_reachability_for_status(status: &str) -> Option<Value> {
    match status {
        "starting" | "ready" | "running" => Some(serde_json::json!({ "reachable": true })),
        "error" => Some(serde_json::json!({ "reachable": false })),
        _ => None,
    }
}

impl SessionService {
    fn ensure_workflow_publication_alias_available(
        &self,
        session_id: &str,
        alias: &str,
    ) -> Result<(), DaemonError> {
        if self
            .get_session(session_id)?
            .workflow_publications()
            .iter()
            .any(|publication| publication.alias() == Some(alias))
        {
            Err(DaemonError::LocalTransport {
                operation: "create workflow publication",
                message: format!("workflow publication alias `{alias}` is already in use"),
            })
        } else {
            Ok(())
        }
    }
}

fn normalize_workflow_publication_queue_ref(queue_ref: Option<String>) -> String {
    queue_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("default")
        .to_string()
}

fn normalize_workflow_publication_operation_key(
    operation_key: Option<String>,
) -> Result<Option<String>, DaemonError> {
    let Some(operation_key) = operation_key else {
        return Ok(None);
    };
    let operation_key = operation_key.trim();
    if operation_key.is_empty() {
        return Ok(None);
    }
    if operation_key.len() > 200 || operation_key.chars().any(char::is_control) {
        return invalid_workflow_publication_option(
            "workflow publication operation key must be at most 200 printable characters",
        );
    }
    Ok(Some(operation_key.to_string()))
}

#[allow(clippy::too_many_arguments)]
fn workflow_publication_creation_request_digest(
    session_id: &str,
    workflow_ref: &str,
    endpoint_ref: &str,
    expected_workflow_revision: Option<u64>,
    queue_ref: &str,
    alias: Option<&str>,
    kind: &str,
    route: Option<&str>,
    methods: &[String],
    transport: Option<&Value>,
    parser: Option<&Value>,
    input_schema: Option<&Value>,
    trace_exposure: Option<&Value>,
    mode: Option<&str>,
    sync_timeout_ms: Option<u64>,
    poll_ms: Option<u64>,
) -> Result<String, DaemonError> {
    workflow_publication_value_digest(&serde_json::json!({
        "session_id": session_id.trim(),
        "workflow_ref": workflow_ref.trim(),
        "endpoint_ref": endpoint_ref.trim(),
        "expected_workflow_revision": expected_workflow_revision,
        "queue_ref": queue_ref,
        "alias": alias,
        "kind": kind,
        "route": route,
        "methods": methods,
        "transport": transport,
        "parser": parser,
        "input_schema": input_schema,
        "trace_exposure": trace_exposure,
        "mode": mode,
        "sync_timeout_ms": sync_timeout_ms,
        "poll_ms": poll_ms,
    }))
}

fn workflow_publication_value_digest(value: &Value) -> Result<String, DaemonError> {
    let canonical = canonical_workflow_publication_value(value);
    let encoded = serde_json::to_vec(&canonical).map_err(|error| DaemonError::LocalTransport {
        operation: "create workflow publication",
        message: format!("failed to encode workflow publication digest input: {error}"),
    })?;
    Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
}

fn canonical_workflow_publication_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(canonical_workflow_publication_value)
                .collect(),
        ),
        Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort();
            let mut canonical = serde_json::Map::new();
            for key in keys {
                canonical.insert(
                    key.clone(),
                    canonical_workflow_publication_value(&values[key]),
                );
            }
            Value::Object(canonical)
        }
        value => value.clone(),
    }
}

fn validate_workflow_publication_options(
    publication_kind: &str,
    transport: &Option<serde_json::Value>,
    route: Option<&str>,
    methods: &[String],
    parser: &Option<serde_json::Value>,
    mode: Option<&str>,
) -> Result<(), DaemonError> {
    validate_workflow_publication_mode(mode)?;
    validate_workflow_publication_route(route)?;
    if publication_kind == WORKFLOW_PUBLICATION_KIND_EVENT_BASED {
        if route.is_some_and(|route| !route.trim().is_empty())
            || !methods.is_empty()
            || parser.is_some()
            || mode.is_some()
            || transport.is_some()
        {
            return invalid_workflow_publication_option(
                "event_based publications use event bindings and do not configure ingress transport, route, methods, parser, or response mode",
            );
        }
        return Ok(());
    }
    if publication_kind == WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY {
        if route.is_some_and(|route| !route.trim().is_empty()) {
            return invalid_workflow_publication_option(
                "schedule_only publications do not expose an ingress route",
            );
        }
        if !methods.is_empty() {
            return invalid_workflow_publication_option(
                "schedule_only publications do not support HTTP method overrides",
            );
        }
        if parser.is_some() {
            return invalid_workflow_publication_option(
                "schedule_only publications do not parse external request input",
            );
        }
        if mode.is_some() {
            return invalid_workflow_publication_option(
                "schedule_only publications do not support response mode overrides",
            );
        }
        let transport_kind = workflow_publication_transport_kind(transport)?;
        if transport.is_some() && transport_kind != WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY {
            return invalid_workflow_publication_option(
                "schedule_only publications must not configure an ingress transport",
            );
        }
        return Ok(());
    }

    let kind = workflow_publication_transport_kind(transport)?;
    if kind == WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY {
        return invalid_workflow_publication_option(
            "ingress publications must use an ingress transport",
        );
    }
    match kind.as_str() {
        "human_http" => {
            validate_workflow_publication_methods(&kind, methods, &["GET", "POST"])?;
            validate_human_http_publication_parser(parser)?;
        }
        "api_sse_json" | "websocket_json" | "mcp" => {
            return invalid_workflow_publication_option(&format!(
                "workflow publication transport `{kind}` was removed; use `human_http` with GET/POST (SSE remains an internal HTTP progress mechanism)",
            ));
        }
        _ => {
            return invalid_workflow_publication_option(&format!(
                "unsupported workflow publication transport `{kind}`"
            ));
        }
    }
    Ok(())
}

fn resolve_workflow_publication_kind(
    kind: Option<&str>,
    transport: &Option<serde_json::Value>,
) -> Result<String, DaemonError> {
    let inferred = || -> Result<String, DaemonError> {
        let transport_kind = workflow_publication_transport_kind(transport)?;
        if transport_kind == WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY {
            Ok(WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY.to_string())
        } else {
            Ok(WORKFLOW_PUBLICATION_KIND_INGRESS.to_string())
        }
    };
    let Some(kind) = kind.map(str::trim).filter(|value| !value.is_empty()) else {
        return inferred();
    };
    match kind {
        WORKFLOW_PUBLICATION_KIND_INGRESS
        | WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY
        | WORKFLOW_PUBLICATION_KIND_EVENT_BASED => Ok(kind.to_string()),
        _ => invalid_workflow_publication_option(&format!(
            "unsupported workflow publication kind `{kind}`"
        )),
    }
}

fn validate_workflow_publication_route(route: Option<&str>) -> Result<(), DaemonError> {
    let Some(route) = route.map(str::trim).filter(|route| !route.is_empty()) else {
        return Ok(());
    };
    if route.starts_with('/') {
        return Ok(());
    }
    invalid_workflow_publication_option("workflow publication route must start with `/`")
}

fn workflow_publication_transport_kind(
    transport: &Option<serde_json::Value>,
) -> Result<String, DaemonError> {
    let Some(transport) = transport else {
        return Ok("human_http".to_string());
    };
    if let Some(kind) = transport.get("kind").and_then(|value| value.as_str()) {
        return Ok(kind.to_string());
    }
    if let Some(kind) = transport.as_str() {
        return Ok(kind.to_string());
    }
    invalid_workflow_publication_option(
        "workflow publication transport must be a string or { kind }",
    )
}

fn validate_workflow_publication_mode(mode: Option<&str>) -> Result<(), DaemonError> {
    match mode {
        Some("sync" | "async") | None => Ok(()),
        Some(mode) => invalid_workflow_publication_option(&format!(
            "unsupported workflow publication mode `{mode}`"
        )),
    }
}

fn validate_workflow_publication_methods(
    transport: &str,
    methods: &[String],
    allowed: &[&str],
) -> Result<(), DaemonError> {
    for method in methods {
        if !allowed.iter().any(|allowed| *allowed == method) {
            return invalid_workflow_publication_option(&format!(
                "{transport} publications do not support HTTP method `{method}`"
            ));
        }
    }
    Ok(())
}

fn validate_human_http_publication_parser(
    parser: &Option<serde_json::Value>,
) -> Result<(), DaemonError> {
    let Some(parser) = parser else {
        return Ok(());
    };
    let Some(kind) = parser.get("kind").and_then(|value| value.as_str()) else {
        return invalid_workflow_publication_option(
            "human_http publication parser must be an object with a supported kind",
        );
    };
    if matches!(kind, "path_template" | "json" | "query_params" | "webhook") {
        return Ok(());
    }
    invalid_workflow_publication_option(&format!(
        "human_http publications do not support parser `{kind}`"
    ))
}

fn invalid_workflow_publication_option<T>(message: &str) -> Result<T, DaemonError> {
    Err(DaemonError::LocalTransport {
        operation: "create workflow publication",
        message: message.to_string(),
    })
}
