use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCodeDefinition {
    #[serde(default = "default_workflow_code_schema_version")]
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters_schema: Option<Value>,
    pub workflow: WorkflowCodeWorkflow,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schemas: Vec<WorkflowCodeSchemaDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<WorkflowCodeNodeDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<WorkflowCodeEdgeDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<WorkflowCodeEndpointDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queues: Vec<WorkflowCodeQueueDefinition>,
    #[serde(default, alias = "watchdogs", skip_serializing_if = "Vec::is_empty")]
    pub schedules: Vec<WorkflowCodeScheduleDefinition>,
}

impl WorkflowCodeDefinition {
    pub fn validate_with_limits(
        &self,
        limits: &WorkflowCodeLimitsConfig,
    ) -> WorkflowCodeValidationReport {
        let mut validator = WorkflowCodeValidator::new(limits);
        validator.validate(self);
        validator.finish()
    }
}
