use std::collections::BTreeMap;

use super::*;
use crate::config::WorkflowCodeLimitsConfig;
use crate::session::{WorkflowCanvasLayoutPatch, WorkflowCanvasPoint};
use crate::workflow_code::{
    WorkflowCodeAgentBinding, WorkflowCodeApplyReport, WorkflowCodeApplyWarning,
    WorkflowCodeDefinition, WORKFLOW_CODE_CANVAS_DEFAULT_ENDPOINT_OFFSET_X,
    WORKFLOW_CODE_CANVAS_RECOMMENDED_GRID_X, WORKFLOW_CODE_CANVAS_RECOMMENDED_GRID_Y,
};

impl SessionService {
    pub(crate) fn rebuild_workflow_code_definition(
        &mut self,
        session_id: &str,
        workflow_ref: &str,
        expected_workflow_revision: u64,
        definition: &WorkflowCodeDefinition,
        source: crate::session::WorkflowCodeSourceDescriptor,
    ) -> Result<WorkflowCodeApplyReport, DaemonError> {
        let current = self.resolve_workflow_ref(session_id, workflow_ref)?;
        if current.revision() != expected_workflow_revision {
            return Err(DaemonError::LocalTransport {
                operation: "workflow_code.rebuild",
                message: format!(
                    "workflow revision conflict: expected {expected_workflow_revision}, current {}",
                    current.revision()
                ),
            });
        }
        let binding = current
            .code_source()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "workflow_code.rebuild",
                message: "workflow does not have a stored code source".to_string(),
            })?;
        if binding.artifact_name() != source.artifact_name {
            return Err(DaemonError::LocalTransport {
                operation: "workflow_code.rebuild",
                message: "stored workflow-code artifact does not match the workflow binding"
                    .to_string(),
            });
        }
        let mut report = binding.bindings().clone();
        let source_handles = |values: Vec<&str>| {
            values
                .into_iter()
                .map(str::to_string)
                .collect::<std::collections::BTreeSet<_>>()
        };
        let mapping_handles = |values: &BTreeMap<String, String>| {
            values
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        };
        for (kind, source, mapped) in [
            (
                "schema",
                source_handles(
                    definition
                        .schemas
                        .iter()
                        .map(|value| value.handle.as_str())
                        .collect(),
                ),
                mapping_handles(&report.schema_refs),
            ),
            (
                "node",
                source_handles(
                    definition
                        .nodes
                        .iter()
                        .map(|value| value.handle.as_str())
                        .collect(),
                ),
                mapping_handles(&report.node_ids),
            ),
            (
                "edge",
                source_handles(
                    definition
                        .edges
                        .iter()
                        .map(|value| value.handle.as_str())
                        .collect(),
                ),
                mapping_handles(&report.edge_ids),
            ),
            (
                "endpoint",
                source_handles(
                    definition
                        .endpoints
                        .iter()
                        .map(|value| value.handle.as_str())
                        .collect(),
                ),
                mapping_handles(&report.endpoint_ids),
            ),
            (
                "schedule",
                source_handles(
                    definition
                        .schedules
                        .iter()
                        .map(|value| value.handle.as_str())
                        .collect(),
                ),
                mapping_handles(&report.schedule_ids),
            ),
        ] {
            if source != mapped {
                return Err(DaemonError::LocalTransport {
                    operation: "workflow_code.rebuild",
                    message: format!("stored {kind} bindings do not match the source structure"),
                });
            }
        }
        let required_queue_handles = if definition.queues.is_empty() {
            std::collections::BTreeSet::from(["default".to_string()])
        } else {
            definition
                .queues
                .iter()
                .map(|queue| queue.handle.clone())
                .collect()
        };
        if !required_queue_handles
            .iter()
            .all(|handle| report.queue_ids.contains_key(handle))
        {
            return Err(DaemonError::LocalTransport {
                operation: "workflow_code.rebuild",
                message: "stored queue bindings do not match the source structure".to_string(),
            });
        }
        let resolve_schema =
            |handle: Option<&str>| handle.and_then(|value| report.schema_refs.get(value).cloned());
        let schemas = definition
            .schemas
            .iter()
            .map(|source| {
                crate::session::WorkflowSchemaDefinition::new(
                    report.schema_refs[&source.handle].clone(),
                    source.alias.clone(),
                    source.description.clone(),
                    source.schema.clone(),
                )
            })
            .collect::<Vec<_>>();
        let nodes = definition
            .nodes
            .iter()
            .map(|source| {
                let node_id = report.node_ids[&source.handle].clone();
                let agent_id = report.agent_ids[&source.handle].clone();
                let previous = current.node(&node_id);
                let mut node =
                    crate::session::WorkflowNodeDefinition::new(node_id, agent_id.clone());
                node.set_owner_user_id(
                    previous.map_or(DEFAULT_LOCAL_USER_ID, |value| value.owner_user_id()),
                );
                node.set_created_by_user_id(
                    previous.map_or(DEFAULT_LOCAL_USER_ID, |value| value.created_by_user_id()),
                );
                node.set_public_label(source.public_label.clone().unwrap_or(agent_id));
                node.set_instructions(source.instructions.clone());
                node.set_can_complete_workflow_run(
                    source.can_complete_workflow_run.unwrap_or(false),
                );
                node.set_can_emit_intermediate_run_output(
                    source.can_emit_intermediate_run_output.unwrap_or(false),
                );
                node.set_wait_for_all_inputs(source.wait_for_all_inputs.unwrap_or(false));
                node.set_intermediate_output_schema_ref(resolve_schema(
                    source.intermediate_output_schema.as_deref(),
                ));
                node.set_max_turns(source.max_turns);
                node
            })
            .collect::<Vec<_>>();
        let edges = definition
            .edges
            .iter()
            .map(|source| {
                let mut edge = crate::session::WorkflowEdgeDefinition::new_with_sides(
                    report.edge_ids[&source.handle].clone(),
                    report.node_ids[&source.from_node].clone(),
                    report.node_ids[&source.to_node].clone(),
                    source.source_side,
                    source.target_side,
                    resolve_schema(source.handoff_schema.as_deref()),
                    source.validation_policy,
                );
                if let Some(previous) = current.edges().iter().find(|value| value.id() == edge.id())
                {
                    edge.set_created_by_user_id(previous.created_by_user_id());
                }
                edge
            })
            .collect::<Vec<_>>();
        let endpoints = definition
            .endpoints
            .iter()
            .map(|source| {
                let endpoint_id = report.endpoint_ids[&source.handle].clone();
                let mut endpoint = crate::session::WorkflowEndpointDefinition::new(
                    endpoint_id.clone(),
                    source.alias.clone(),
                    report.node_ids[&source.entry_node].clone(),
                );
                if let Some(previous) = current
                    .endpoints()
                    .iter()
                    .find(|value| value.id() == endpoint_id)
                {
                    endpoint.set_owner_user_id(previous.owner_user_id());
                }
                endpoint
            })
            .collect::<Vec<_>>();
        let workflow_id = current.id().to_string();
        let desired_queues = if definition.queues.is_empty() {
            vec![("default".to_string(), "default".to_string(), 0, true)]
        } else {
            definition
                .queues
                .iter()
                .map(|queue| {
                    (
                        queue.handle.clone(),
                        queue.alias.clone(),
                        queue.priority,
                        queue.enabled,
                    )
                })
                .collect::<Vec<_>>()
        };
        let desired_queue_ids = desired_queues
            .iter()
            .map(|(handle, _, _, _)| report.queue_ids[handle].clone())
            .collect::<std::collections::BTreeSet<_>>();
        let session = self.get_session(session_id)?;
        let removed_queue_ids = session
            .workflow_prompt_queues_for_workflow(&workflow_id)
            .into_iter()
            .map(|queue| queue.id().to_string())
            .filter(|queue_id| !desired_queue_ids.contains(queue_id))
            .collect::<Vec<_>>();
        if session.workflow_queued_prompts().iter().any(|prompt| {
            removed_queue_ids
                .iter()
                .any(|queue_id| queue_id == prompt.queue_id())
        }) {
            return Err(DaemonError::LocalTransport {
                operation: "workflow_code.rebuild",
                message: "cannot remove a workflow queue while it still contains prompts"
                    .to_string(),
            });
        }
        let default_max_concurrent = self.workflow_default_max_concurrent;
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        {
            let workflow = session.workflow_mut(&workflow_id).ok_or_else(|| {
                DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.clone(),
                }
            })?;
            workflow.replace_code_structure(crate::session::WorkflowCodeStructureReplacement {
                alias: definition.workflow.alias.clone(),
                prompt: definition.workflow.prompt.clone(),
                flush_agent_context_before_run: definition
                    .workflow
                    .flush_agent_context_before_run
                    .unwrap_or(true),
                max_concurrent: definition
                    .workflow
                    .max_concurrent
                    .unwrap_or(default_max_concurrent),
                run_output_schema_ref: resolve_schema(
                    definition.workflow.run_output_schema.as_deref(),
                ),
                schemas,
                nodes,
                edges,
                endpoints,
            });
            let patches = workflow_code_canvas_patches(definition, &report);
            if !patches.is_empty() {
                workflow.update_canvas_layout(patches);
                report.canvas_layout_applied = true;
            }
        }

        for queue_id in removed_queue_ids {
            session.remove_workflow_prompt_queue(&workflow_id, &queue_id);
        }
        for (handle, alias, priority, enabled) in desired_queues {
            let queue_id = report.queue_ids[&handle].clone();
            if let Some(queue) = session.workflow_prompt_queue_mut(&workflow_id, &queue_id) {
                queue.set_alias(alias);
                queue.set_priority(priority);
                queue.set_enabled(enabled);
            } else {
                let mut queue = crate::session::WorkflowPromptQueueDefinition::new(
                    queue_id,
                    workflow_id.clone(),
                    alias,
                    priority,
                );
                queue.set_enabled(enabled);
                session.add_workflow_prompt_queue(queue);
            }
        }

        let desired_schedule_ids = definition
            .schedules
            .iter()
            .map(|schedule| report.schedule_ids[&schedule.handle].clone())
            .collect::<std::collections::BTreeSet<_>>();
        let removed_schedule_ids = session
            .workflow_schedules()
            .iter()
            .filter(|schedule| schedule.workflow_id() == workflow_id)
            .map(|schedule| schedule.id().to_string())
            .filter(|schedule_id| !desired_schedule_ids.contains(schedule_id))
            .collect::<Vec<_>>();
        for schedule_id in removed_schedule_ids {
            session.remove_workflow_schedule(&schedule_id);
        }
        for source in &definition.schedules {
            let schedule_id = report.schedule_ids[&source.handle].clone();
            let endpoint_id = report.endpoint_ids[&source.endpoint].clone();
            let queue_id = source
                .queue
                .as_ref()
                .and_then(|handle| report.queue_ids.get(handle).cloned());
            if let Some(schedule) = session
                .workflow_schedules_mut()
                .iter_mut()
                .find(|schedule| schedule.id() == schedule_id)
            {
                schedule.reconfigure(crate::session::WorkflowScheduleReconfiguration {
                    endpoint_id,
                    queue_id,
                    trigger: source.trigger.clone(),
                    invocation_prompt: source.invocation_prompt.clone(),
                    overlap_policy: source.overlap_policy,
                    max_runs: source.max_runs,
                    enabled: source.enabled.unwrap_or(true),
                });
            } else {
                let mut schedule = crate::session::WorkflowScheduleDefinition::new_with_trigger(
                    schedule_id,
                    workflow_id.clone(),
                    endpoint_id,
                    source.trigger.clone(),
                    source.invocation_prompt.clone(),
                    source.overlap_policy,
                    source.max_runs,
                );
                schedule.set_queue_id(queue_id);
                schedule.set_enabled(source.enabled.unwrap_or(true));
                session.add_workflow_schedule(schedule);
            }
        }
        let workflow =
            session
                .workflow_mut(&workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.clone(),
                })?;
        workflow.bind_code_source(
            source.artifact_name,
            source.language,
            source.source_sha256,
            source.origin,
            report.clone(),
        );
        Ok(report)
    }

    pub fn apply_workflow_code_definition(
        &mut self,
        session_id: &str,
        definition: &WorkflowCodeDefinition,
        node_agent_ids: &BTreeMap<String, String>,
        limits: &WorkflowCodeLimitsConfig,
        created_by_user_id: String,
        controlled_by_metaagent_id: Option<String>,
    ) -> Result<WorkflowCodeApplyReport, DaemonError> {
        self.apply_workflow_code_definition_with_alias_base(
            session_id,
            definition,
            node_agent_ids,
            limits,
            created_by_user_id,
            controlled_by_metaagent_id,
            None,
        )
    }

    pub fn apply_workflow_code_definition_with_alias_base(
        &mut self,
        session_id: &str,
        definition: &WorkflowCodeDefinition,
        node_agent_ids: &BTreeMap<String, String>,
        limits: &WorkflowCodeLimitsConfig,
        created_by_user_id: String,
        controlled_by_metaagent_id: Option<String>,
        alias_base: Option<&str>,
    ) -> Result<WorkflowCodeApplyReport, DaemonError> {
        let mut effective_limits = limits.clone();
        effective_limits.max_queues = effective_limits
            .max_queues
            .min(self.max_workflow_queues_per_workflow as u32);
        let validation = definition.validate_with_limits(&effective_limits);
        if !validation.ok {
            return Err(DaemonError::LocalTransport {
                operation: "workflow_code.apply",
                message: format!(
                    "workflow-code definition is invalid: {}",
                    validation
                        .diagnostics
                        .iter()
                        .map(|diagnostic| diagnostic.code.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
        }
        self.validate_workflow_code_agent_bindings(session_id, definition, node_agent_ids)?;
        let workflow = if let Some(alias_base) = alias_base {
            self.create_workflow_controlled_by_metaagent_with_alias_base(
                session_id,
                None,
                alias_base,
                controlled_by_metaagent_id.clone(),
            )?
        } else {
            self.create_workflow_code_workflow(
                session_id,
                definition.workflow.alias.as_deref(),
                controlled_by_metaagent_id.clone(),
            )?
        };
        let workflow_id = workflow.id().to_string();
        let schema_refs = self.apply_workflow_code_schemas(session_id, &workflow_id, definition)?;
        let mut report = WorkflowCodeApplyReport {
            workflow_id: workflow_id.clone(),
            schema_refs: schema_refs.clone(),
            node_ids: BTreeMap::new(),
            agent_ids: BTreeMap::new(),
            edge_ids: BTreeMap::new(),
            endpoint_ids: BTreeMap::new(),
            queue_ids: BTreeMap::new(),
            schedule_ids: BTreeMap::new(),
            canvas_layout_applied: false,
            warnings: Vec::new(),
        };

        self.apply_workflow_code_workflow_settings(
            session_id,
            &workflow_id,
            definition,
            &schema_refs,
        )?;

        for node in &definition.nodes {
            let agent_id =
                node_agent_ids
                    .get(&node.handle)
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "workflow_code.apply",
                        message: format!(
                            "node `{}` does not have a resolved agent id",
                            node.handle
                        ),
                    })?;
            let public_label = node
                .public_label
                .clone()
                .unwrap_or_else(|| agent_id.to_string());
            let created = self.add_workflow_node_owned(
                session_id,
                &workflow_id,
                agent_id,
                created_by_user_id.clone(),
                created_by_user_id.clone(),
                public_label,
            )?;
            report
                .node_ids
                .insert(node.handle.clone(), created.id().to_string());
            report
                .agent_ids
                .insert(node.handle.clone(), agent_id.to_string());
            self.apply_workflow_code_node_settings(
                session_id,
                &workflow_id,
                created.id(),
                node,
                &schema_refs,
            )?;
        }

        for edge in &definition.edges {
            let from_node_id = report.node_ids.get(&edge.from_node).ok_or_else(|| {
                DaemonError::LocalTransport {
                    operation: "workflow_code.apply",
                    message: format!("edge `{}` references unapplied source node", edge.handle),
                }
            })?;
            let to_node_id =
                report
                    .node_ids
                    .get(&edge.to_node)
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "workflow_code.apply",
                        message: format!("edge `{}` references unapplied target node", edge.handle),
                    })?;
            let created = self.add_workflow_edge_owned_with_sides(
                session_id,
                &workflow_id,
                from_node_id,
                to_node_id,
                created_by_user_id.clone(),
                edge.source_side,
                edge.target_side,
                resolve_workflow_code_schema_ref(edge.handoff_schema.as_deref(), &schema_refs),
                edge.validation_policy,
            )?;
            report
                .edge_ids
                .insert(edge.handle.clone(), created.id().to_string());
        }

        for endpoint in &definition.endpoints {
            let entry_node_id = report.node_ids.get(&endpoint.entry_node).ok_or_else(|| {
                DaemonError::LocalTransport {
                    operation: "workflow_code.apply",
                    message: format!(
                        "endpoint `{}` references unapplied entry node",
                        endpoint.handle
                    ),
                }
            })?;
            let created = self.create_workflow_endpoint(
                session_id,
                &workflow_id,
                entry_node_id,
                endpoint.alias.clone(),
            )?;
            let created = self.set_workflow_endpoint_owner(
                session_id,
                &workflow_id,
                created.id(),
                created_by_user_id.clone(),
            )?;
            report
                .endpoint_ids
                .insert(endpoint.handle.clone(), created.id().to_string());
        }

        self.apply_workflow_code_queues(session_id, &workflow_id, definition, &mut report)?;

        for schedule in &definition.schedules {
            let endpoint_id = report.endpoint_ids.get(&schedule.endpoint).ok_or_else(|| {
                DaemonError::LocalTransport {
                    operation: "workflow_code.apply",
                    message: format!(
                        "schedule `{}` references unapplied endpoint",
                        schedule.handle
                    ),
                }
            })?;
            let created = self.create_workflow_schedule(
                session_id,
                &workflow_id,
                endpoint_id,
                schedule
                    .queue
                    .as_deref()
                    .and_then(|queue| report.queue_ids.get(queue).map(String::as_str)),
                schedule.trigger.clone(),
                schedule.invocation_prompt.clone(),
                schedule.overlap_policy,
                Some(schedule.max_runs),
            )?;
            let created = if let Some(enabled) = schedule.enabled {
                self.set_workflow_schedule_enabled(session_id, created.id(), enabled)?
            } else {
                created
            };
            report
                .schedule_ids
                .insert(schedule.handle.clone(), created.id().to_string());
        }

        let canvas_auto_layout_needed = workflow_code_canvas_auto_layout_needed(definition);
        let patches = workflow_code_canvas_patches(definition, &report);
        if !patches.is_empty() {
            self.update_workflow_canvas_layout(session_id, &workflow_id, patches)?;
            report.canvas_layout_applied = true;
            if canvas_auto_layout_needed {
                report.warnings.push(WorkflowCodeApplyWarning {
                    code: "canvas_auto_layout_applied".to_string(),
                    message: "one or more nodes or endpoints omitted canvas coordinates; the kernel assigned canvas positions".to_string(),
                    handle: None,
                });
            }
        }

        Ok(report)
    }

    fn apply_workflow_code_schemas(
        &mut self,
        session_id: &str,
        workflow_id: &str,
        definition: &WorkflowCodeDefinition,
    ) -> Result<BTreeMap<String, String>, DaemonError> {
        let mut schema_refs = BTreeMap::new();
        for schema in &definition.schemas {
            let schema_id = self.next_workflow_schema_id();
            let session =
                self.store
                    .get_mut(session_id)
                    .ok_or_else(|| DaemonError::SessionNotFound {
                        session_id: session_id.to_string(),
                    })?;
            let workflow =
                session
                    .workflow_mut(workflow_id)
                    .ok_or_else(|| DaemonError::WorkflowNotFound {
                        session_id: session_id.to_string(),
                        workflow_id: workflow_id.to_string(),
                    })?;
            workflow.add_schema(WorkflowSchemaDefinition::new(
                schema_id.clone(),
                schema.alias.clone(),
                schema.description.clone(),
                schema.schema.clone(),
            ));
            schema_refs.insert(schema.handle.clone(), schema_id);
        }
        Ok(schema_refs)
    }

    fn create_workflow_code_workflow(
        &mut self,
        session_id: &str,
        requested_alias: Option<&str>,
        controlled_by_metaagent_id: Option<String>,
    ) -> Result<crate::session::WorkflowDefinition, DaemonError> {
        let Some(alias) = requested_alias else {
            return self.create_workflow_controlled_by_metaagent(
                session_id,
                None,
                controlled_by_metaagent_id,
            );
        };
        let trimmed_alias = alias.trim();
        if trimmed_alias.is_empty() {
            return self.create_workflow_controlled_by_metaagent(
                session_id,
                Some(alias.to_string()),
                controlled_by_metaagent_id,
            );
        }

        for attempt in 0..crate::workflow_code::WORKFLOW_CODE_ALIAS_ALLOCATION_ATTEMPTS {
            let candidate_alias = if attempt == 0 {
                trimmed_alias.to_string()
            } else {
                format!("{trimmed_alias}-{}", attempt + 1)
            };
            match self.create_workflow_controlled_by_metaagent(
                session_id,
                Some(candidate_alias),
                controlled_by_metaagent_id.clone(),
            ) {
                Ok(workflow) => return Ok(workflow),
                Err(DaemonError::WorkflowAliasConflict { .. }) => continue,
                Err(error) => return Err(error),
            }
        }

        Err(DaemonError::LocalTransport {
            operation: "workflow_code.apply",
            message: format!("could not allocate a unique workflow alias for `{trimmed_alias}`"),
        })
    }

    fn validate_workflow_code_agent_bindings(
        &self,
        _session_id: &str,
        definition: &WorkflowCodeDefinition,
        node_agent_ids: &BTreeMap<String, String>,
    ) -> Result<(), DaemonError> {
        for node in &definition.nodes {
            let agent_id =
                node_agent_ids
                    .get(&node.handle)
                    .ok_or_else(|| DaemonError::LocalTransport {
                        operation: "workflow_code.apply",
                        message: format!("node `{}` is missing a resolved agent id", node.handle),
                    })?;
            if let WorkflowCodeAgentBinding::Existing(existing) = &node.agent {
                if existing.agent_ref != *agent_id {
                    return Err(DaemonError::LocalTransport {
                        operation: "workflow_code.apply",
                        message: format!(
                            "node `{}` resolved existing agent `{}` to mismatched agent `{agent_id}`",
                            node.handle, existing.agent_ref
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    fn apply_workflow_code_workflow_settings(
        &mut self,
        session_id: &str,
        workflow_id: &str,
        definition: &WorkflowCodeDefinition,
        schema_refs: &BTreeMap<String, String>,
    ) -> Result<(), DaemonError> {
        let session =
            self.store
                .get_mut(session_id)
                .ok_or_else(|| DaemonError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?;
        let workflow =
            session
                .workflow_mut(workflow_id)
                .ok_or_else(|| DaemonError::WorkflowNotFound {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.to_string(),
                })?;
        workflow.set_prompt(definition.workflow.prompt.clone());
        if let Some(value) = definition.workflow.flush_agent_context_before_run {
            workflow.set_flush_agent_context_before_run(value);
        }
        if let Some(value) = definition.workflow.max_concurrent {
            workflow.set_max_concurrent(value);
        }
        workflow.set_run_output_schema_ref(resolve_workflow_code_schema_ref(
            definition.workflow.run_output_schema.as_deref(),
            schema_refs,
        ));
        Ok(())
    }

    fn apply_workflow_code_node_settings(
        &mut self,
        session_id: &str,
        workflow_id: &str,
        node_id: &str,
        node: &crate::workflow_code::WorkflowCodeNodeDefinition,
        schema_refs: &BTreeMap<String, String>,
    ) -> Result<(), DaemonError> {
        if node.instructions.is_some() {
            self.update_workflow_node_instructions(
                session_id,
                workflow_id,
                node_id,
                node.instructions.clone(),
            )?;
        }
        if let Some(value) = node.can_complete_workflow_run {
            self.set_workflow_node_can_complete_run(session_id, workflow_id, node_id, value)?;
        }
        if let Some(value) = node.can_emit_intermediate_run_output {
            self.set_workflow_node_can_emit_intermediate_output(
                session_id,
                workflow_id,
                node_id,
                value,
            )?;
        }
        if let Some(value) = node.wait_for_all_inputs {
            self.set_workflow_node_wait_for_all_inputs(session_id, workflow_id, node_id, value)?;
        }
        if node.intermediate_output_schema.is_some() {
            self.set_workflow_node_intermediate_output_schema_ref(
                session_id,
                workflow_id,
                node_id,
                resolve_workflow_code_schema_ref(
                    node.intermediate_output_schema.as_deref(),
                    schema_refs,
                ),
            )?;
        }
        if node.max_turns.is_some() {
            self.set_workflow_node_max_turns(session_id, workflow_id, node_id, node.max_turns)?;
        }
        Ok(())
    }

    fn apply_workflow_code_queues(
        &mut self,
        session_id: &str,
        workflow_id: &str,
        definition: &WorkflowCodeDefinition,
        report: &mut WorkflowCodeApplyReport,
    ) -> Result<(), DaemonError> {
        if definition.queues.is_empty() {
            let session = self.get_session(session_id)?;
            let default_queue = session
                .workflow_prompt_queue(workflow_id, "default")
                .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.to_string(),
                    reference: "default".to_string(),
                    message: "default workflow prompt queue was not created",
                })?;
            report
                .queue_ids
                .insert("default".to_string(), default_queue.id().to_string());
            report.warnings.push(WorkflowCodeApplyWarning {
                code: "default_queue_created".to_string(),
                message: "workflow-code omitted queues; the kernel used the workflow default prompt queue".to_string(),
                handle: Some("default".to_string()),
            });
            return Ok(());
        }

        for queue in &definition.queues {
            let normalized_alias = normalize_workflow_queue_alias(queue.alias.clone())?;
            let queue_id = if normalized_alias == "default" {
                let session = self.get_session(session_id)?;
                session
                    .workflow_prompt_queue(workflow_id, "default")
                    .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                        session_id: session_id.to_string(),
                        workflow_id: workflow_id.to_string(),
                        reference: "default".to_string(),
                        message: "default workflow prompt queue was not created",
                    })?
                    .id()
                    .to_string()
            } else {
                self.create_workflow_prompt_queue(
                    session_id,
                    workflow_id,
                    normalized_alias.clone(),
                    queue.priority,
                )?
                .id()
                .to_string()
            };
            if normalized_alias == "default" && (queue.priority != 0 || !queue.enabled) {
                self.update_workflow_prompt_queue(
                    session_id,
                    workflow_id,
                    "default",
                    None,
                    Some(queue.priority),
                    Some(queue.enabled),
                )?;
            } else if !queue.enabled {
                self.update_workflow_prompt_queue(
                    session_id,
                    workflow_id,
                    &queue_id,
                    None,
                    None,
                    Some(false),
                )?;
            }
            report.queue_ids.insert(queue.handle.clone(), queue_id);
        }
        if definition
            .schedules
            .iter()
            .any(|schedule| schedule.queue.as_deref() == Some("default"))
            && !report.queue_ids.contains_key("default")
        {
            let session = self.get_session(session_id)?;
            let default_queue = session
                .workflow_prompt_queue(workflow_id, "default")
                .ok_or_else(|| DaemonError::InvalidWorkflowGraphReference {
                    session_id: session_id.to_string(),
                    workflow_id: workflow_id.to_string(),
                    reference: "default".to_string(),
                    message: "default workflow prompt queue was not created",
                })?;
            report
                .queue_ids
                .insert("default".to_string(), default_queue.id().to_string());
        }
        Ok(())
    }
}

fn resolve_workflow_code_schema_ref(
    schema_ref: Option<&str>,
    schema_refs: &BTreeMap<String, String>,
) -> Option<String> {
    schema_ref.map(|schema_ref| {
        schema_refs
            .get(schema_ref)
            .cloned()
            .unwrap_or_else(|| schema_ref.to_string())
    })
}

fn workflow_code_canvas_patches(
    definition: &WorkflowCodeDefinition,
    report: &WorkflowCodeApplyReport,
) -> Vec<WorkflowCanvasLayoutPatch> {
    let mut patches = Vec::new();
    let node_depths = workflow_code_node_depths(definition);
    let mut rows_by_depth: BTreeMap<i32, i32> = BTreeMap::new();
    let mut node_points = BTreeMap::new();

    for node in &definition.nodes {
        let Some(node_id) = report.node_ids.get(&node.handle) else {
            continue;
        };
        let depth = *node_depths.get(&node.handle).unwrap_or(&0);
        let row = rows_by_depth.entry(depth).or_insert(0);
        let auto_point = WorkflowCanvasPoint {
            x: depth.saturating_mul(WORKFLOW_CODE_CANVAS_RECOMMENDED_GRID_X as i32),
            y: row.saturating_mul(WORKFLOW_CODE_CANVAS_RECOMMENDED_GRID_Y as i32),
        };
        *row = row.saturating_add(1);
        let point = node.canvas.map_or_else(
            || auto_point,
            |point| WorkflowCanvasPoint {
                x: point.x,
                y: point.y,
            },
        );
        node_points.insert(node.handle.clone(), point.clone());
        patches.push(WorkflowCanvasLayoutPatch::NodePosition {
            node_id: node_id.clone(),
            x: point.x,
            y: point.y,
        });
    }

    for endpoint in &definition.endpoints {
        let Some(endpoint_id) = report.endpoint_ids.get(&endpoint.handle) else {
            continue;
        };
        let point = endpoint.canvas.map_or_else(
            || {
                node_points
                    .get(&endpoint.entry_node)
                    .map(|point| WorkflowCanvasPoint {
                        x: point
                            .x
                            .saturating_add(WORKFLOW_CODE_CANVAS_DEFAULT_ENDPOINT_OFFSET_X as i32),
                        y: point.y,
                    })
                    .unwrap_or(WorkflowCanvasPoint {
                        x: WORKFLOW_CODE_CANVAS_DEFAULT_ENDPOINT_OFFSET_X as i32,
                        y: 0,
                    })
            },
            |point| WorkflowCanvasPoint {
                x: point.x,
                y: point.y,
            },
        );
        patches.push(WorkflowCanvasLayoutPatch::EndpointPosition {
            endpoint_id: endpoint_id.clone(),
            x: point.x,
            y: point.y,
        });
    }

    for edge in &definition.edges {
        if let Some(canvas) = &edge.canvas {
            if let Some(edge_id) = report.edge_ids.get(&edge.handle) {
                patches.push(WorkflowCanvasLayoutPatch::EdgeWaypoints {
                    edge_id: edge_id.clone(),
                    waypoints: canvas
                        .points
                        .iter()
                        .map(|point| WorkflowCanvasPoint {
                            x: point.x,
                            y: point.y,
                        })
                        .collect(),
                });
            }
        }
    }
    patches
}

fn workflow_code_canvas_auto_layout_needed(definition: &WorkflowCodeDefinition) -> bool {
    definition.nodes.iter().any(|node| node.canvas.is_none())
        || definition
            .endpoints
            .iter()
            .any(|endpoint| endpoint.canvas.is_none())
}

fn workflow_code_node_depths(definition: &WorkflowCodeDefinition) -> BTreeMap<String, i32> {
    let mut depths: BTreeMap<String, i32> = BTreeMap::new();
    let mut queue = Vec::new();
    for endpoint in &definition.endpoints {
        if depths.insert(endpoint.entry_node.clone(), 0).is_none() {
            queue.push(endpoint.entry_node.clone());
        }
    }

    let mut cursor = 0;
    while let Some(node_handle) = queue.get(cursor).cloned() {
        cursor += 1;
        let Some(depth) = depths.get(&node_handle).copied() else {
            continue;
        };
        for edge in &definition.edges {
            if edge.from_node == node_handle {
                let next_depth = depth.saturating_add(1);
                let should_update = depths
                    .get(&edge.to_node)
                    .is_none_or(|current| next_depth < *current);
                if should_update {
                    depths.insert(edge.to_node.clone(), next_depth);
                    queue.push(edge.to_node.clone());
                }
            }
        }
    }

    for node in &definition.nodes {
        depths.entry(node.handle.clone()).or_insert(0);
    }
    depths
}
