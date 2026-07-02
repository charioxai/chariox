use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::types::unix_epoch_ms;
use super::workflow_canvas::{
    WorkflowCanvasEdgeLayout, WorkflowCanvasLayout, WorkflowCanvasLayoutPatch, WorkflowCanvasPoint,
};
use super::workflow_graph::{
    WorkflowEdgeDefinition, WorkflowEndpointDefinition, WorkflowNodeDefinition,
};

fn default_workflow_flush_agent_context_before_run() -> bool {
    true
}

fn default_workflow_max_concurrent() -> u32 {
    super::types::DEFAULT_WORKFLOW_CODE_MAX_CONCURRENT
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowSchemaDefinition {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    schema: Value,
}

impl Eq for WorkflowSchemaDefinition {}

impl WorkflowSchemaDefinition {
    pub fn new(
        id: impl Into<String>,
        alias: Option<String>,
        description: Option<String>,
        schema: Value,
    ) -> Self {
        Self {
            id: id.into(),
            alias,
            description,
            schema,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn schema(&self) -> &Value {
        &self.schema
    }

    pub fn set_alias(&mut self, alias: Option<String>) {
        self.alias = alias;
    }

    pub fn set_description(&mut self, description: Option<String>) {
        self.description = description;
    }

    pub fn set_schema(&mut self, schema: Value) {
        self.schema = schema;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    id: String,
    alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    controlled_by_metaagent_id: Option<String>,
    #[serde(default = "unix_epoch_ms")]
    created_at_ms: u64,
    #[serde(default)]
    revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    canvas_layout: Option<WorkflowCanvasLayout>,
    #[serde(default = "default_workflow_flush_agent_context_before_run")]
    flush_agent_context_before_run: bool,
    #[serde(default = "default_workflow_max_concurrent")]
    max_concurrent: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    intermediate_output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    schemas: Vec<WorkflowSchemaDefinition>,
    nodes: Vec<WorkflowNodeDefinition>,
    edges: Vec<WorkflowEdgeDefinition>,
    endpoints: Vec<WorkflowEndpointDefinition>,
}

impl WorkflowDefinition {
    pub fn new(id: impl Into<String>, alias: Option<String>) -> Self {
        Self {
            id: id.into(),
            alias,
            controlled_by_metaagent_id: None,
            created_at_ms: unix_epoch_ms(),
            revision: 0,
            canvas_layout: None,
            flush_agent_context_before_run: default_workflow_flush_agent_context_before_run(),
            max_concurrent: default_workflow_max_concurrent(),
            run_output_schema_ref: None,
            intermediate_output_schema_ref: None,
            schemas: Vec::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
            endpoints: Vec::new(),
        }
    }

    pub fn new_controlled_by_metaagent(
        id: impl Into<String>,
        alias: Option<String>,
        metaagent_id: impl Into<String>,
    ) -> Self {
        let mut workflow = Self::new(id, alias);
        workflow.controlled_by_metaagent_id = Some(metaagent_id.into());
        workflow
    }

    pub fn with_max_concurrent(mut self, value: u32) -> Self {
        self.max_concurrent = value.max(1);
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn controlled_by_metaagent_id(&self) -> Option<&str> {
        self.controlled_by_metaagent_id.as_deref()
    }

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn canvas_layout(&self) -> Option<&WorkflowCanvasLayout> {
        self.canvas_layout.as_ref()
    }

    pub fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    pub fn flush_agent_context_before_run(&self) -> bool {
        self.flush_agent_context_before_run
    }

    pub fn max_concurrent(&self) -> u32 {
        self.max_concurrent
    }

    pub fn nodes(&self) -> &[WorkflowNodeDefinition] {
        &self.nodes
    }

    pub fn run_output_schema_ref(&self) -> Option<&str> {
        self.run_output_schema_ref.as_deref()
    }

    pub fn intermediate_output_schema_ref(&self) -> Option<&str> {
        self.intermediate_output_schema_ref.as_deref()
    }

    pub fn schemas(&self) -> &[WorkflowSchemaDefinition] {
        &self.schemas
    }

    pub fn schema(&self, schema_id: &str) -> Option<&WorkflowSchemaDefinition> {
        self.schemas.iter().find(|schema| schema.id() == schema_id)
    }

    pub fn schema_mut(&mut self, schema_id: &str) -> Option<&mut WorkflowSchemaDefinition> {
        self.schemas
            .iter_mut()
            .find(|schema| schema.id() == schema_id)
    }

    pub fn edges(&self) -> &[WorkflowEdgeDefinition] {
        &self.edges
    }

    pub fn endpoints(&self) -> &[WorkflowEndpointDefinition] {
        &self.endpoints
    }

    pub fn set_alias(&mut self, alias: Option<String>) {
        self.alias = alias;
        self.bump_revision();
    }

    pub fn set_controlled_by_metaagent_id(&mut self, metaagent_id: Option<String>) {
        self.controlled_by_metaagent_id = metaagent_id;
        self.bump_revision();
    }

    pub fn set_flush_agent_context_before_run(&mut self, value: bool) {
        self.flush_agent_context_before_run = value;
        self.bump_revision();
    }

    pub fn set_max_concurrent(&mut self, value: u32) {
        self.max_concurrent = value.max(1);
        self.bump_revision();
    }

    pub fn set_run_output_schema_ref(&mut self, value: Option<String>) {
        self.run_output_schema_ref = value;
        self.bump_revision();
    }

    pub fn set_intermediate_output_schema_ref(&mut self, value: Option<String>) {
        self.intermediate_output_schema_ref = value;
        self.bump_revision();
    }

    pub fn add_schema(&mut self, schema: WorkflowSchemaDefinition) -> WorkflowSchemaDefinition {
        self.schemas.push(schema.clone());
        self.bump_revision();
        schema
    }

    pub fn remove_schema(&mut self, schema_id: &str) -> Option<WorkflowSchemaDefinition> {
        let index = self
            .schemas
            .iter()
            .position(|schema| schema.id() == schema_id)?;
        let schema = self.schemas.remove(index);
        self.bump_revision();
        Some(schema)
    }

    pub fn schema_ref_usages(&self, schema_id: &str) -> Vec<String> {
        let mut usages = Vec::new();
        if self.run_output_schema_ref.as_deref() == Some(schema_id) {
            usages.push("workflow.run_output_schema_ref".to_string());
        }
        if self.intermediate_output_schema_ref.as_deref() == Some(schema_id) {
            usages.push("workflow.intermediate_output_schema_ref".to_string());
        }
        for node in &self.nodes {
            if node.intermediate_output_schema_ref() == Some(schema_id) {
                usages.push(format!("node.{}.intermediate_output_schema_ref", node.id()));
            }
        }
        for edge in &self.edges {
            if edge.handoff_schema_ref() == Some(schema_id) {
                usages.push(format!("edge.{}.handoff_schema_ref", edge.id()));
            }
        }
        usages
    }

    pub fn add_node(&mut self, node: WorkflowNodeDefinition) -> WorkflowNodeDefinition {
        self.nodes.push(node.clone());
        self.bump_revision();
        node
    }

    pub fn node(&self, node_id: &str) -> Option<&WorkflowNodeDefinition> {
        self.nodes.iter().find(|node| node.id() == node_id)
    }

    pub fn node_mut(&mut self, node_id: &str) -> Option<&mut WorkflowNodeDefinition> {
        self.nodes.iter_mut().find(|node| node.id() == node_id)
    }

    pub fn remove_node(&mut self, node_id: &str) -> Option<WorkflowNodeDefinition> {
        let index = self.nodes.iter().position(|node| node.id() == node_id)?;
        let removed = self.nodes.remove(index);
        let removed_edge_ids = self
            .edges
            .iter()
            .filter(|edge| edge.from_node_id() == node_id || edge.to_node_id() == node_id)
            .map(|edge| edge.id().to_string())
            .collect::<Vec<_>>();
        let removed_endpoint_ids = self
            .endpoints
            .iter()
            .filter(|endpoint| endpoint.entry_node_id() == node_id)
            .map(|endpoint| endpoint.id().to_string())
            .collect::<Vec<_>>();
        self.edges
            .retain(|edge| edge.from_node_id() != node_id && edge.to_node_id() != node_id);
        self.endpoints
            .retain(|endpoint| endpoint.entry_node_id() != node_id);
        if let Some(layout) = self.canvas_layout.as_mut() {
            layout.nodes.remove(node_id);
            for edge_id in removed_edge_ids {
                layout.edges.remove(&edge_id);
            }
            for endpoint_id in removed_endpoint_ids {
                layout.endpoints.remove(&endpoint_id);
            }
            layout.bump_revision();
        }
        self.bump_revision();
        Some(removed)
    }

    pub fn set_node_position(&mut self, node_id: impl Into<String>, point: WorkflowCanvasPoint) {
        let layout = self
            .canvas_layout
            .get_or_insert_with(WorkflowCanvasLayout::new);
        layout.nodes.insert(node_id.into(), point);
        layout.bump_revision();
        self.bump_revision();
    }

    pub fn add_edge(&mut self, edge: WorkflowEdgeDefinition) -> WorkflowEdgeDefinition {
        self.edges.push(edge.clone());
        self.bump_revision();
        edge
    }

    pub fn edge(&self, edge_id: &str) -> Option<&WorkflowEdgeDefinition> {
        self.edges.iter().find(|edge| edge.id() == edge_id)
    }

    pub fn edge_mut(&mut self, edge_id: &str) -> Option<&mut WorkflowEdgeDefinition> {
        self.edges.iter_mut().find(|edge| edge.id() == edge_id)
    }

    pub fn has_edge(&self, from_node_id: &str, to_node_id: &str) -> bool {
        self.edges
            .iter()
            .any(|edge| edge.from_node_id() == from_node_id && edge.to_node_id() == to_node_id)
    }

    pub fn remove_edge(&mut self, edge_id: &str) -> Option<WorkflowEdgeDefinition> {
        let index = self.edges.iter().position(|edge| edge.id() == edge_id)?;
        let edge = self.edges.remove(index);
        if let Some(layout) = self.canvas_layout.as_mut() {
            layout.edges.remove(edge_id);
            layout.bump_revision();
        }
        self.bump_revision();
        Some(edge)
    }

    pub fn add_endpoint(
        &mut self,
        endpoint: WorkflowEndpointDefinition,
    ) -> WorkflowEndpointDefinition {
        self.endpoints.push(endpoint.clone());
        self.bump_revision();
        endpoint
    }

    pub fn endpoint(&self, endpoint_id: &str) -> Option<&WorkflowEndpointDefinition> {
        self.endpoints
            .iter()
            .find(|endpoint| endpoint.id() == endpoint_id)
    }

    pub fn endpoint_mut(&mut self, endpoint_id: &str) -> Option<&mut WorkflowEndpointDefinition> {
        self.endpoints
            .iter_mut()
            .find(|endpoint| endpoint.id() == endpoint_id)
    }

    pub fn remove_endpoint(&mut self, endpoint_id: &str) -> Option<WorkflowEndpointDefinition> {
        let index = self
            .endpoints
            .iter()
            .position(|endpoint| endpoint.id() == endpoint_id)?;
        let endpoint = self.endpoints.remove(index);
        if let Some(layout) = self.canvas_layout.as_mut() {
            layout.endpoints.remove(endpoint_id);
            layout.bump_revision();
        }
        self.bump_revision();
        Some(endpoint)
    }

    pub fn set_endpoint_position(
        &mut self,
        endpoint_id: impl Into<String>,
        point: WorkflowCanvasPoint,
    ) {
        let layout = self
            .canvas_layout
            .get_or_insert_with(WorkflowCanvasLayout::new);
        layout.endpoints.insert(endpoint_id.into(), point);
        layout.bump_revision();
        self.bump_revision();
    }

    pub fn update_canvas_layout(
        &mut self,
        patches: Vec<WorkflowCanvasLayoutPatch>,
    ) -> WorkflowCanvasLayout {
        let node_ids = self
            .nodes
            .iter()
            .map(|node| node.id().to_string())
            .collect::<BTreeSet<_>>();
        let edge_ids = self
            .edges
            .iter()
            .map(|edge| edge.id().to_string())
            .collect::<BTreeSet<_>>();
        let endpoint_ids = self
            .endpoints
            .iter()
            .map(|endpoint| endpoint.id().to_string())
            .collect::<BTreeSet<_>>();
        let layout = self
            .canvas_layout
            .get_or_insert_with(WorkflowCanvasLayout::new);
        let mut changed = false;
        for patch in patches {
            match patch {
                WorkflowCanvasLayoutPatch::NodePosition { node_id, x, y } => {
                    if node_ids.contains(&node_id) {
                        changed |= layout
                            .nodes
                            .insert(node_id, WorkflowCanvasPoint { x, y })
                            .as_ref()
                            .is_none_or(|existing| existing.x != x || existing.y != y);
                    }
                }
                WorkflowCanvasLayoutPatch::EndpointPosition { endpoint_id, x, y } => {
                    if endpoint_ids.contains(&endpoint_id) {
                        changed |= layout
                            .endpoints
                            .insert(endpoint_id, WorkflowCanvasPoint { x, y })
                            .as_ref()
                            .is_none_or(|existing| existing.x != x || existing.y != y);
                    }
                }
                WorkflowCanvasLayoutPatch::EdgeWaypoints { edge_id, waypoints } => {
                    if edge_ids.contains(&edge_id) {
                        let next = WorkflowCanvasEdgeLayout { waypoints };
                        let previous = layout.edges.insert(edge_id, next.clone());
                        changed |= previous.as_ref() != Some(&next);
                    }
                }
            }
        }
        layout.nodes.retain(|node_id, _| node_ids.contains(node_id));
        layout.edges.retain(|edge_id, _| edge_ids.contains(edge_id));
        layout
            .endpoints
            .retain(|endpoint_id, _| endpoint_ids.contains(endpoint_id));
        if changed {
            layout.bump_revision();
        }
        layout.clone()
    }

    pub fn redacted_for_user(mut self, user_id: &str) -> Self {
        self.nodes = self
            .nodes
            .into_iter()
            .map(|node| node.redacted_for_user(user_id))
            .collect();
        self
    }
}
