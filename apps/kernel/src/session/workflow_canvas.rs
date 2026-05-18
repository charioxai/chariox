use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCanvasPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCanvasEdgeLayout {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waypoints: Vec<WorkflowCanvasPoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCanvasLayout {
    pub version: u32,
    pub revision: u64,
    pub coordinate_space: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub nodes: BTreeMap<String, WorkflowCanvasPoint>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub endpoints: BTreeMap<String, WorkflowCanvasPoint>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub edges: BTreeMap<String, WorkflowCanvasEdgeLayout>,
}

impl WorkflowCanvasLayout {
    pub fn new() -> Self {
        Self {
            version: 1,
            revision: 0,
            coordinate_space: "workflow-canvas-v1".to_string(),
            nodes: BTreeMap::new(),
            endpoints: BTreeMap::new(),
            edges: BTreeMap::new(),
        }
    }

    pub fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
}

impl Default for WorkflowCanvasLayout {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowCanvasLayoutPatch {
    NodePosition {
        node_id: String,
        x: i32,
        y: i32,
    },
    EndpointPosition {
        endpoint_id: String,
        x: i32,
        y: i32,
    },
    EdgeWaypoints {
        edge_id: String,
        #[serde(default)]
        waypoints: Vec<WorkflowCanvasPoint>,
    },
}
