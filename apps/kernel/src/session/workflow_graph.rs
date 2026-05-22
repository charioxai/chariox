use serde::{Deserialize, Serialize};

use super::types::DEFAULT_LOCAL_USER_ID;

fn default_workflow_owner_user_id() -> String {
    DEFAULT_LOCAL_USER_ID.to_string()
}

fn default_workflow_node_public_label() -> String {
    "agent".to_string()
}

fn default_workflow_node_can_complete_workflow_run() -> bool {
    false
}

fn default_workflow_node_can_emit_intermediate_run_output() -> bool {
    false
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowEndpointDefinition {
    id: String,
    #[serde(default = "default_workflow_owner_user_id")]
    owner_user_id: String,
    alias: Option<String>,
    entry_node_id: String,
}

impl WorkflowEndpointDefinition {
    pub fn new(
        id: impl Into<String>,
        alias: Option<String>,
        entry_node_id: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            owner_user_id: default_workflow_owner_user_id(),
            alias,
            entry_node_id: entry_node_id.into(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
    }

    pub fn entry_node_id(&self) -> &str {
        &self.entry_node_id
    }

    pub fn set_owner_user_id(&mut self, owner_user_id: impl Into<String>) {
        self.owner_user_id = owner_user_id.into();
    }

    pub fn set_alias(&mut self, alias: Option<String>) {
        self.alias = alias;
    }

    pub fn set_entry_node_id(&mut self, entry_node_id: impl Into<String>) {
        self.entry_node_id = entry_node_id.into();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowNodeDefinition {
    id: String,
    agent_id: String,
    #[serde(default = "default_workflow_owner_user_id")]
    owner_user_id: String,
    #[serde(default = "default_workflow_node_public_label")]
    public_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    #[serde(default = "default_workflow_node_can_complete_workflow_run")]
    can_complete_workflow_run: bool,
    #[serde(default = "default_workflow_node_can_emit_intermediate_run_output")]
    can_emit_intermediate_run_output: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    intermediate_output_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_turns: Option<u32>,
}

impl WorkflowNodeDefinition {
    pub fn new(id: impl Into<String>, agent_id: impl Into<String>) -> Self {
        let agent_id = agent_id.into();
        Self {
            id: id.into(),
            public_label: agent_id.clone(),
            agent_id,
            owner_user_id: default_workflow_owner_user_id(),
            instructions: None,
            can_complete_workflow_run: default_workflow_node_can_complete_workflow_run(),
            can_emit_intermediate_run_output:
                default_workflow_node_can_emit_intermediate_run_output(),
            intermediate_output_schema_ref: None,
            max_turns: None,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
    }

    pub fn public_label(&self) -> &str {
        &self.public_label
    }

    pub fn set_owner_user_id(&mut self, owner_user_id: impl Into<String>) {
        self.owner_user_id = owner_user_id.into();
    }

    pub fn set_public_label(&mut self, public_label: impl Into<String>) {
        self.public_label = public_label.into();
    }

    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }

    pub fn set_instructions(&mut self, instructions: Option<String>) {
        self.instructions = instructions;
    }

    pub fn can_complete_workflow_run(&self) -> bool {
        self.can_complete_workflow_run
    }

    pub fn set_can_complete_workflow_run(&mut self, value: bool) {
        self.can_complete_workflow_run = value;
    }

    pub fn can_emit_intermediate_run_output(&self) -> bool {
        self.can_emit_intermediate_run_output
    }

    pub fn set_can_emit_intermediate_run_output(&mut self, value: bool) {
        self.can_emit_intermediate_run_output = value;
    }

    pub fn intermediate_output_schema_ref(&self) -> Option<&str> {
        self.intermediate_output_schema_ref.as_deref()
    }

    pub fn set_intermediate_output_schema_ref(&mut self, value: Option<String>) {
        self.intermediate_output_schema_ref = value;
    }

    pub fn max_turns(&self) -> Option<u32> {
        self.max_turns
    }

    pub fn set_max_turns(&mut self, value: Option<u32>) {
        self.max_turns = value;
    }

    pub fn redacted_for_user(mut self, user_id: &str) -> Self {
        if self.owner_user_id != user_id {
            self.instructions = None;
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowEdgeDefinition {
    id: String,
    from_node_id: String,
    to_node_id: String,
    #[serde(default = "default_workflow_owner_user_id")]
    created_by_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    handoff_schema_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validation_policy: Option<WorkflowHandoffValidationPolicy>,
}

impl WorkflowEdgeDefinition {
    pub fn new(
        id: impl Into<String>,
        from_node_id: impl Into<String>,
        to_node_id: impl Into<String>,
        handoff_schema_ref: Option<String>,
        validation_policy: Option<WorkflowHandoffValidationPolicy>,
    ) -> Self {
        Self {
            id: id.into(),
            from_node_id: from_node_id.into(),
            to_node_id: to_node_id.into(),
            created_by_user_id: default_workflow_owner_user_id(),
            handoff_schema_ref,
            validation_policy,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn from_node_id(&self) -> &str {
        &self.from_node_id
    }

    pub fn to_node_id(&self) -> &str {
        &self.to_node_id
    }

    pub fn created_by_user_id(&self) -> &str {
        &self.created_by_user_id
    }

    pub fn set_created_by_user_id(&mut self, created_by_user_id: impl Into<String>) {
        self.created_by_user_id = created_by_user_id.into();
    }

    pub fn handoff_schema_ref(&self) -> Option<&str> {
        self.handoff_schema_ref.as_deref()
    }

    pub fn validation_policy(&self) -> Option<WorkflowHandoffValidationPolicy> {
        self.validation_policy
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowHandoffValidationPolicy {
    Warn,
    Halt,
}
