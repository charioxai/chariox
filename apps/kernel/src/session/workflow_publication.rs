use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::types::unix_epoch_ms;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPublicationDefinition {
    id: String,
    session_id: String,
    workflow_id: String,
    endpoint_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    queue_ref: Option<String>,
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
    trace_exposure: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sync_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    poll_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    local_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    open_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deployment: Option<Value>,
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
        queue_ref: Option<String>,
        alias: Option<String>,
        route: Option<String>,
        methods: Vec<String>,
        transport: Option<Value>,
        parser: Option<Value>,
        input_schema: Option<Value>,
        trace_exposure: Option<Value>,
        mode: Option<String>,
        sync_timeout_ms: Option<u64>,
        poll_ms: Option<u64>,
        local_port: Option<u16>,
        created_by_user_id: impl Into<String>,
    ) -> Self {
        let now = unix_epoch_ms();
        Self {
            id: id.into(),
            session_id: session_id.into(),
            workflow_id: workflow_id.into(),
            endpoint_id: endpoint_id.into(),
            queue_ref,
            alias,
            enabled: true,
            route,
            methods,
            transport,
            parser,
            input_schema,
            trace_exposure,
            mode,
            sync_timeout_ms,
            poll_ms,
            local_port,
            status: None,
            open_url: None,
            deployment: None,
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

    pub fn queue_ref(&self) -> Option<&str> {
        self.queue_ref.as_deref()
    }

    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn route(&self) -> Option<&str> {
        self.route.as_deref()
    }

    pub fn created_by_user_id(&self) -> &str {
        &self.created_by_user_id
    }

    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    pub fn open_url(&self) -> Option<&str> {
        self.open_url.as_deref()
    }

    pub fn deployment(&self) -> Option<&Value> {
        self.deployment.as_ref()
    }

    pub fn trace_exposure(&self) -> Option<&Value> {
        self.trace_exposure.as_ref()
    }

    pub fn local_port(&self) -> Option<u16> {
        self.local_port
    }

    pub fn disable(&mut self) {
        self.enabled = false;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn mark_served(
        &mut self,
        status: impl Into<String>,
        open_url: impl Into<String>,
        deployment: Value,
    ) {
        self.status = Some(status.into());
        self.open_url = Some(open_url.into());
        self.deployment = Some(deployment);
        self.updated_at_ms = unix_epoch_ms();
    }
}
