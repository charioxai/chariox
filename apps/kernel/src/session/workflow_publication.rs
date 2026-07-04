use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::types::unix_epoch_ms;

const MAX_WORKFLOW_PUBLICATION_RUNTIME_LOGS: usize = 20;
pub const WORKFLOW_PUBLICATION_KIND_INGRESS: &str = "ingress";
pub const WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY: &str = "schedule_only";

fn default_workflow_publication_kind() -> String {
    WORKFLOW_PUBLICATION_KIND_INGRESS.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPublicationRuntimeLogEntry {
    pub at_ms: u64,
    pub level: String,
    pub message: String,
}

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
    #[serde(default = "default_workflow_publication_kind")]
    kind: String,
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
    status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    open_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    viewer_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deployment: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime_last_heartbeat_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime_last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    runtime_logs: Vec<WorkflowPublicationRuntimeLogEntry>,
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
        kind: impl Into<String>,
        route: Option<String>,
        methods: Vec<String>,
        transport: Option<Value>,
        parser: Option<Value>,
        input_schema: Option<Value>,
        trace_exposure: Option<Value>,
        mode: Option<String>,
        sync_timeout_ms: Option<u64>,
        poll_ms: Option<u64>,
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
            kind: kind.into(),
            route,
            methods,
            transport,
            parser,
            input_schema,
            trace_exposure,
            mode,
            sync_timeout_ms,
            poll_ms,
            status: None,
            open_url: None,
            viewer_url: None,
            deployment: None,
            runtime_last_heartbeat_at_ms: None,
            runtime_last_error: None,
            runtime_logs: Vec::new(),
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

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn route(&self) -> Option<&str> {
        self.route.as_deref()
    }

    pub fn transport(&self) -> Option<&Value> {
        self.transport.as_ref()
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

    pub fn viewer_url(&self) -> Option<&str> {
        self.viewer_url.as_deref()
    }

    pub fn deployment(&self) -> Option<&Value> {
        self.deployment.as_ref()
    }

    pub fn runtime_last_heartbeat_at_ms(&self) -> Option<u64> {
        self.runtime_last_heartbeat_at_ms
    }

    pub fn runtime_last_error(&self) -> Option<&str> {
        self.runtime_last_error.as_deref()
    }

    pub fn runtime_logs(&self) -> &[WorkflowPublicationRuntimeLogEntry] {
        &self.runtime_logs
    }

    pub fn trace_exposure(&self) -> Option<&Value> {
        self.trace_exposure.as_ref()
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
        let status = status.into();
        let open_url = open_url.into();
        let now = unix_epoch_ms();
        self.status = Some(status.clone());
        self.open_url = Some(open_url.clone());
        self.viewer_url = Some(open_url);
        self.deployment = Some(deployment);
        self.runtime_last_heartbeat_at_ms = Some(now);
        self.runtime_last_error = None;
        self.push_runtime_log_at(now, "info", format!("publication endpoint {status}"));
        self.updated_at_ms = now;
    }

    pub fn mark_runtime_status(
        &mut self,
        status: impl Into<String>,
        open_url: Option<Option<String>>,
        deployment: Option<Value>,
    ) {
        let status = status.into();
        let now = unix_epoch_ms();
        self.status = Some(status.clone());
        if let Some(open_url) = open_url {
            self.viewer_url = open_url.clone();
            self.open_url = open_url;
        }
        if let Some(deployment) = deployment {
            self.deployment = Some(deployment);
        }
        self.runtime_last_heartbeat_at_ms = Some(now);
        if status == "error" {
            self.runtime_last_error = Some("publication runtime reported error".to_string());
        } else {
            self.runtime_last_error = None;
        }
        self.push_runtime_log_at(now, "info", format!("publication runtime {status}"));
        self.updated_at_ms = now;
    }

    pub fn mark_runtime_error(&mut self, message: impl Into<String>) {
        let now = unix_epoch_ms();
        let message = message.into();
        self.status = Some("error".to_string());
        self.runtime_last_heartbeat_at_ms = Some(now);
        self.runtime_last_error = Some(message.clone());
        self.push_runtime_log_at(now, "error", message);
        self.updated_at_ms = now;
    }

    fn push_runtime_log_at(
        &mut self,
        at_ms: u64,
        level: impl Into<String>,
        message: impl Into<String>,
    ) {
        self.runtime_logs.push(WorkflowPublicationRuntimeLogEntry {
            at_ms,
            level: level.into(),
            message: message.into(),
        });
        if self.runtime_logs.len() > MAX_WORKFLOW_PUBLICATION_RUNTIME_LOGS {
            let overflow = self.runtime_logs.len() - MAX_WORKFLOW_PUBLICATION_RUNTIME_LOGS;
            self.runtime_logs.drain(0..overflow);
        }
    }
}
