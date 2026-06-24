use std::collections::BTreeMap;

use super::*;
use crate::config::WorkflowCodeLimitsConfig;
use crate::session::{WorkflowCanvasLayoutPatch, WorkflowCanvasPoint};
use crate::workflow_code::{
    WorkflowCodeAgentBinding, WorkflowCodeApplyReport, WorkflowCodeDefinition,
};

impl SessionService {
    pub fn apply_workflow_code_definition(
        &mut self,
        session_id: &str,
        definition: &WorkflowCodeDefinition,
        node_agent_ids: &BTreeMap<String, String>,
        limits: &WorkflowCodeLimitsConfig,
        created_by_user_id: String,
        controlled_by_metaagent_id: Option<String>,
    ) -> Result<WorkflowCodeApplyReport, DaemonError> {
        let validation = definition.validate_with_limits(limits);
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
        for watchdog in &definition.watchdogs {
            if watchdog.queue.is_some() {
                return Err(DaemonError::LocalTransport {
                    operation: "workflow_code.apply",
                    message: format!(
                        "watchdog `{}` declares a queue, but runtime watchdogs do not carry queue bindings yet",
                        watchdog.handle
                    ),
                });
            }
        }

        let workflow = self.create_workflow_controlled_by_metaagent(
            session_id,
            definition.workflow.alias.clone(),
            controlled_by_metaagent_id,
        )?;
        let workflow_id = workflow.id().to_string();
        let mut report = WorkflowCodeApplyReport {
            workflow_id: workflow_id.clone(),
            schema_refs: definition
                .schemas
                .iter()
                .map(|schema| (schema.handle.clone(), schema.handle.clone()))
                .collect(),
            node_ids: BTreeMap::new(),
            agent_ids: BTreeMap::new(),
            edge_ids: BTreeMap::new(),
            endpoint_ids: BTreeMap::new(),
            queue_ids: BTreeMap::new(),
            watchdog_ids: BTreeMap::new(),
            canvas_layout_applied: false,
        };

        self.apply_workflow_code_workflow_settings(session_id, &workflow_id, definition)?;

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
            self.apply_workflow_code_node_settings(session_id, &workflow_id, created.id(), node)?;
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
                edge.handoff_schema.clone(),
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
            report
                .endpoint_ids
                .insert(endpoint.handle.clone(), created.id().to_string());
        }

        self.apply_workflow_code_queues(session_id, &workflow_id, definition, &mut report)?;

        for watchdog in &definition.watchdogs {
            let endpoint_id = report.endpoint_ids.get(&watchdog.endpoint).ok_or_else(|| {
                DaemonError::LocalTransport {
                    operation: "workflow_code.apply",
                    message: format!(
                        "watchdog `{}` references unapplied endpoint",
                        watchdog.handle
                    ),
                }
            })?;
            let created = self.create_workflow_watchdog(
                session_id,
                &workflow_id,
                endpoint_id,
                watchdog.interval_seconds,
                watchdog.invocation_prompt.clone(),
                watchdog.policy,
                Some(watchdog.max_wakeups),
            )?;
            report
                .watchdog_ids
                .insert(watchdog.handle.clone(), created.id().to_string());
        }

        let patches = workflow_code_canvas_patches(definition, &report);
        if !patches.is_empty() {
            self.update_workflow_canvas_layout(session_id, &workflow_id, patches)?;
            report.canvas_layout_applied = true;
        }

        Ok(report)
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
        if let Some(value) = definition.workflow.flush_agent_context_before_run {
            workflow.set_flush_agent_context_before_run(value);
        }
        workflow.set_run_output_schema_ref(definition.workflow.run_output_schema.clone());
        workflow.set_intermediate_output_schema_ref(
            definition.workflow.intermediate_output_schema.clone(),
        );
        Ok(())
    }

    fn apply_workflow_code_node_settings(
        &mut self,
        session_id: &str,
        workflow_id: &str,
        node_id: &str,
        node: &crate::workflow_code::WorkflowCodeNodeDefinition,
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
                node.intermediate_output_schema.clone(),
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
            return Ok(());
        }

        for queue in &definition.queues {
            let queue_id = if queue.alias == "default" {
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
                    queue.alias.clone(),
                    queue.priority,
                )?
                .id()
                .to_string()
            };
            if queue.alias == "default" && (queue.priority != 0 || !queue.enabled) {
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
        Ok(())
    }
}

fn workflow_code_canvas_patches(
    definition: &WorkflowCodeDefinition,
    report: &WorkflowCodeApplyReport,
) -> Vec<WorkflowCanvasLayoutPatch> {
    let mut patches = Vec::new();
    for node in &definition.nodes {
        if let Some(point) = node.canvas {
            if let Some(node_id) = report.node_ids.get(&node.handle) {
                patches.push(WorkflowCanvasLayoutPatch::NodePosition {
                    node_id: node_id.clone(),
                    x: point.x,
                    y: point.y,
                });
            }
        }
    }
    for endpoint in &definition.endpoints {
        if let Some(point) = endpoint.canvas {
            if let Some(endpoint_id) = report.endpoint_ids.get(&endpoint.handle) {
                patches.push(WorkflowCanvasLayoutPatch::EndpointPosition {
                    endpoint_id: endpoint_id.clone(),
                    x: point.x,
                    y: point.y,
                });
            }
        }
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
