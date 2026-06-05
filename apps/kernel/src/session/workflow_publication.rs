use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::types::unix_epoch_ms;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPublicationDefinition {
    id: String,
    session_id: String,
    workflow_id: String,
    endpoint_id: String,
    alias: Option<String>,
    enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    route: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    methods: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transport: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parser: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
    created_by_user_id: String,
    created_at_ms: u64,
    updated_at_ms: u64,
}

impl WorkflowPublicationDefinition {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        session_id: impl Into<String>,
        workflow_id: impl Into<String>,
        endpoint_id: impl Into<String>,
        alias: Option<String>,
        route: Option<String>,
        methods: Vec<String>,
        transport: Option<Value>,
        parser: Option<Value>,
        input_schema: Option<Value>,
        mode: Option<String>,
        created_by_user_id: impl Into<String>,
    ) -> Self {
        let now = unix_epoch_ms();
        Self {
            id: id.into(),
            session_id: session_id.into(),
            workflow_id: workflow_id.into(),
            endpoint_id: endpoint_id.into(),
            alias,
            enabled: true,
            route,
            methods,
            transport,
            parser,
            input_schema,
            mode,
            created_by_user_id: created_by_user_id.into(),
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn workflow_id(&self) -> &str {
        &self.workflow_id
    }

    pub fn endpoint_id(&self) -> &str {
        &self.endpoint_id
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn created_by_user_id(&self) -> &str {
        &self.created_by_user_id
    }

    pub fn disable(&mut self) {
        self.enabled = false;
        self.updated_at_ms = unix_epoch_ms();
    }
}
