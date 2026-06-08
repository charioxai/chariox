use super::*;
use crate::session::WorkflowPublicationInvocationEnvelope;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateWorkflowRequest {
    pub session_id: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasWorkflowRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListWorkflowsRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveWorkflowRequest {
    pub session_id: String,
    pub workflow_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateWorkflowPublicationRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub endpoint_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parser: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_exposure: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListWorkflowPublicationsRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetWorkflowPublicationRequest {
    pub session_id: String,
    pub publication_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportWorkflowPublicationPackageRequest {
    pub session_id: String,
    pub publication_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_app: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_app_assets_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPublicationPackageFile {
    pub path: String,
    pub content_base64: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub executable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisableWorkflowPublicationRequest {
    pub session_id: String,
    pub publication_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterWorkflowPublicationEndpointRequest {
    pub session_id: String,
    pub publication_ref: String,
    pub local_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterializeWorkflowPublicationRequest {
    pub publication_id: String,
    pub snapshot: WorkflowPublicationSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPublicationSnapshot {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captured_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session: Option<WorkflowPublicationSourceSessionSnapshot>,
    pub workflow: WorkflowDefinition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<WorkflowEndpointDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queues: Vec<WorkflowPromptQueueDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub watchdogs: Vec<WorkflowWatchdogDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<AgentInstance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPublicationSourceSessionSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub workspace_id: String,
    pub worktree_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateWorkflowEndpointRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub entry_node_id: String,
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasWorkflowEndpointRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub endpoint_ref: String,
    pub alias: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindWorkflowEndpointRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub endpoint_ref: String,
    pub entry_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddWorkflowNodeRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveWorkflowNodeRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateWorkflowNodeInstructionsRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowNodeCanCompleteRunRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
    pub can_complete_workflow_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowNodeCanEmitIntermediateOutputRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
    pub can_emit_intermediate_workflow_run_output: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowNodeIntermediateOutputSchemaRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intermediate_output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowNodeMaxTurnsRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddWorkflowEdgeRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub from_node_id: String,
    pub to_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_policy: Option<crate::session::WorkflowHandoffValidationPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_side: Option<crate::session::WorkflowEdgeEndpointSide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_side: Option<crate::session::WorkflowEdgeEndpointSide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidateWorkflowHandoffRequest {
    pub session_id: String,
    pub handoff_schema_ref: String,
    pub handoff_json: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_policy: Option<crate::session::WorkflowHandoffValidationPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AckWorkflowTurnRequest {
    pub session_id: String,
    pub workflow_run_ref: String,
    pub workflow_node_run_id: String,
    pub delivery_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveWorkflowEdgeRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub edge_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateWorkflowCanvasLayoutRequest {
    pub session_id: String,
    pub workflow_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_layout_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patches: Vec<WorkflowCanvasLayoutPatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvokeWorkflowEndpointRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub endpoint_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_ref: Option<String>,
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication_invocation: Option<WorkflowPublicationInvocationEnvelope>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListWorkflowRunsRequest {
    pub session_id: String,
    pub workflow_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetWorkflowRunRequest {
    pub session_id: String,
    pub workflow_run_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelWorkflowRunRequest {
    pub session_id: String,
    pub workflow_run_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeWorkflowRunRequest {
    pub session_id: String,
    pub workflow_run_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateWorkflowWatchdogRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub endpoint_ref: String,
    pub interval_seconds: u64,
    pub invocation_prompt: String,
    pub policy: WorkflowWatchdogPolicy,
    pub max_wakeups_configured: bool,
    pub max_wakeups: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListWorkflowWatchdogsRequest {
    pub session_id: String,
    pub workflow_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowWatchdogEnabledRequest {
    pub session_id: String,
    pub watchdog_ref: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveWorkflowWatchdogRequest {
    pub session_id: String,
    pub watchdog_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowFlushContextRequest {
    pub session_id: String,
    pub workflow_ref: String,
    pub flush_agent_context_before_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowRunOutputSchemaRequest {
    pub session_id: String,
    pub workflow_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetWorkflowIntermediateOutputSchemaRequest {
    pub session_id: String,
    pub workflow_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intermediate_output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_workflow_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListWorkflowPromptQueuesRequest {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateWorkflowPromptQueueRequest {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_ref: Option<String>,
    pub alias: String,
    pub priority: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateWorkflowPromptQueueRequest {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_ref: Option<String>,
    pub queue_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveWorkflowPromptQueueRequest {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_ref: Option<String>,
    pub queue_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListQueuedWorkflowPromptsRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateQueuedWorkflowPromptRequest {
    pub session_id: String,
    pub queue_item_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveQueuedWorkflowPromptRequest {
    pub session_id: String,
    pub queue_item_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClearWorkflowPromptQueueRequest {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_ref: Option<String>,
    pub queue_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyWorkflowDesignOpRequest {
    pub session_id: String,
    pub origin_client_id: String,
    pub op_id: String,
    pub op: WorkflowDesignOp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignWorkflow {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flush_agent_context_before_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intermediate_output_schema_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignWorkflowPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flush_agent_context_before_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_output_schema_ref: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intermediate_output_schema_ref: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignNode {
    pub id: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_complete_workflow_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_emit_intermediate_run_output: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intermediate_output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignNodePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_complete_workflow_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_emit_intermediate_run_output: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intermediate_output_schema_ref: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<Option<u32>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignEdge {
    pub id: String,
    pub from_node_id: String,
    pub to_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_side: Option<crate::session::WorkflowEdgeEndpointSide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_side: Option<crate::session::WorkflowEdgeEndpointSide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_policy: Option<crate::session::WorkflowHandoffValidationPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignEdgePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_schema_ref: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_policy: Option<Option<crate::session::WorkflowHandoffValidationPolicy>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignEndpoint {
    pub id: String,
    pub entry_node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignEndpointPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_node_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowDesignOp {
    WorkflowCreate {
        workflow: WorkflowDesignWorkflow,
    },
    WorkflowUpdate {
        workflow_id: String,
        patch: WorkflowDesignWorkflowPatch,
    },
    WorkflowRemove {
        workflow_id: String,
    },
    NodeAdd {
        workflow_id: String,
        node: WorkflowDesignNode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position: Option<WorkflowDesignPoint>,
    },
    NodeUpdate {
        workflow_id: String,
        node_id: String,
        patch: WorkflowDesignNodePatch,
    },
    NodeMove {
        workflow_id: String,
        node_id: String,
        position: WorkflowDesignPoint,
    },
    NodeRemove {
        workflow_id: String,
        node_id: String,
    },
    EdgeAdd {
        workflow_id: String,
        edge: WorkflowDesignEdge,
    },
    EdgeUpdate {
        workflow_id: String,
        edge_id: String,
        patch: WorkflowDesignEdgePatch,
    },
    EdgeRemove {
        workflow_id: String,
        edge_id: String,
    },
    EndpointAdd {
        workflow_id: String,
        endpoint: WorkflowDesignEndpoint,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position: Option<WorkflowDesignPoint>,
    },
    EndpointUpdate {
        workflow_id: String,
        endpoint_id: String,
        patch: WorkflowDesignEndpointPatch,
    },
    EndpointMove {
        workflow_id: String,
        endpoint_id: String,
        position: WorkflowDesignPoint,
    },
    EndpointRemove {
        workflow_id: String,
        endpoint_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDesignOpForwarded {
    pub session_id: String,
    pub kernel_sequence: u64,
    pub origin_client_id: String,
    pub op_id: String,
    pub op: WorkflowDesignOp,
}
