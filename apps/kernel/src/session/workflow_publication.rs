use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::agent::AgentInstance;

use super::types::unix_epoch_ms;
use super::workflow_definition::WorkflowDefinition;
use super::workflow_graph::WorkflowEndpointDefinition;
use super::workflow_scheduling::{WorkflowPromptQueueDefinition, WorkflowScheduleDefinition};

const MAX_WORKFLOW_PUBLICATION_RUNTIME_LOGS: usize = 20;
pub const WORKFLOW_PUBLICATION_KIND_INGRESS: &str = "ingress";
pub const WORKFLOW_PUBLICATION_KIND_SCHEDULE_ONLY: &str = "schedule_only";
pub const WORKFLOW_PUBLICATION_KIND_EVENT_BASED: &str = "event_based";
pub const WORKFLOW_PUBLICATION_WORKSPACE_ROOT: &str = "/workspace";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPublicationRuntimeMaterialization {
    pub key: String,
    pub agent_id_map: BTreeMap<String, String>,
}

fn default_workflow_publication_kind() -> String {
    WORKFLOW_PUBLICATION_KIND_INGRESS.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowEventDeliveryReceipt {
    pub delivery_id: String,
    pub binding_id: String,
    pub occurrence_id: String,
    #[serde(default)]
    pub queued_prompt_id: String,
    pub accepted_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowPublicationRuntimeLogEntry {
    pub at_ms: u64,
    pub level: String,
    pub message: String,
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
    #[serde(default, alias = "watchdogs", skip_serializing_if = "Vec::is_empty")]
    pub schedules: Vec<WorkflowScheduleDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<AgentInstance>,
}

impl WorkflowPublicationSnapshot {
    pub fn digest(&self) -> Result<String, serde_json::Error> {
        let value = serde_json::to_value(self)?;
        let encoded = serde_json::to_vec(&canonical_json_value(&value))?;
        Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
    }
}

fn canonical_json_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_json_value).collect()),
        Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort();
            let mut canonical = serde_json::Map::new();
            for key in keys {
                canonical.insert(key.clone(), canonical_json_value(&values[key]));
            }
            Value::Object(canonical)
        }
        value => value.clone(),
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    schedule_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    schedules: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    watchdog_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    watchdogs: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latest_run: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    recent_runs: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latest_output: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    runtime_logs: Vec<WorkflowPublicationRuntimeLogEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_workflow_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_snapshot_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    creation_operation_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    creation_request_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime_materialization: Option<WorkflowPublicationRuntimeMaterialization>,
    /// The App plan (`chariox.publication-apps.v1`) of the owner's latest
    /// deployment export (protocol 366; per release since 368).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    apps: Option<Value>,
    /// Protocol 368: each exported release's App plan by package digest, the
    /// newest last. Bind, recovery and rollback use the release's own plan.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    release_app_plans: Vec<ReleaseAppPlan>,
    /// The single plan a publication prepared before protocol 368 pinned for
    /// all its releases, kept for them once releases record their own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pre_release_app_plan: Option<Value>,
    created_by_user_id: String,
    created_at_ms: u64,
    updated_at_ms: u64,
}

/// Rollback reaches this many past releases' App plans (the bound release's
/// is always kept).
const MAX_RELEASE_APP_PLANS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAppPlan {
    pub package_digest: String,
    pub plan: Value,
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
            runtime: None,
            schedule_count: None,
            schedules: Vec::new(),
            watchdog_count: None,
            watchdogs: Vec::new(),
            latest_run: None,
            recent_runs: Vec::new(),
            latest_output: None,
            runtime_logs: Vec::new(),
            source_workflow_revision: None,
            source_snapshot_digest: None,
            creation_operation_key: None,
            creation_request_digest: None,
            runtime_materialization: None,
            apps: None,
            release_app_plans: Vec::new(),
            pre_release_app_plan: None,
            created_by_user_id: created_by_user_id.into(),
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_immutable(
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
        source_workflow_revision: u64,
        source_snapshot_digest: String,
        creation_operation_key: Option<String>,
        creation_request_digest: Option<String>,
        created_by_user_id: impl Into<String>,
    ) -> Self {
        let mut publication = Self::new(
            id,
            session_id,
            workflow_id,
            endpoint_id,
            queue_ref,
            alias,
            kind,
            route,
            methods,
            transport,
            parser,
            input_schema,
            trace_exposure,
            mode,
            sync_timeout_ms,
            poll_ms,
            created_by_user_id,
        );
        publication.source_workflow_revision = Some(source_workflow_revision);
        publication.source_snapshot_digest = Some(source_snapshot_digest);
        publication.creation_operation_key = creation_operation_key;
        publication.creation_request_digest = creation_request_digest;
        publication
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

    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    pub fn source_workflow_revision(&self) -> Option<u64> {
        self.source_workflow_revision
    }

    pub fn source_snapshot_digest(&self) -> Option<&str> {
        self.source_snapshot_digest.as_deref()
    }

    pub(crate) fn set_runtime_snapshot_digest(&mut self, digest: String) -> Result<(), String> {
        if self.runtime_materialization.is_none() {
            return Err(
                "only a materialized runtime can change its resolved configuration".to_string(),
            );
        }
        self.source_snapshot_digest = Some(digest);
        Ok(())
    }

    pub fn validate_source_snapshot(
        &self,
        snapshot: &WorkflowPublicationSnapshot,
    ) -> Result<(), String> {
        let source_revision = self.source_workflow_revision.ok_or_else(|| {
            format!(
                "workflow trigger `{}` is missing its immutable source revision",
                self.id
            )
        })?;
        let source_digest = self.source_snapshot_digest.as_deref().ok_or_else(|| {
            format!(
                "workflow trigger `{}` is missing its immutable source digest",
                self.id
            )
        })?;
        if snapshot.workflow.id() != self.workflow_id {
            return Err(format!(
                "workflow publication `{}` snapshot workflow `{}` does not match `{}`",
                self.id,
                snapshot.workflow.id(),
                self.workflow_id
            ));
        }
        if snapshot.workflow.revision() != source_revision {
            return Err(format!(
                "workflow publication `{}` snapshot revision {} does not match {}",
                self.id,
                snapshot.workflow.revision(),
                source_revision
            ));
        }
        let actual_digest = snapshot
            .digest()
            .map_err(|error| format!("failed to hash publication snapshot: {error}"))?;
        if actual_digest != source_digest {
            return Err(format!(
                "workflow publication `{}` snapshot digest does not match its immutable source digest",
                self.id
            ));
        }
        let endpoint = snapshot.endpoint.as_ref().ok_or_else(|| {
            format!(
                "workflow publication `{}` snapshot is missing endpoint `{}`",
                self.id, self.endpoint_id
            )
        })?;
        if endpoint.id() != self.endpoint_id
            || snapshot.workflow.endpoint(&self.endpoint_id) != Some(endpoint)
        {
            return Err(format!(
                "workflow publication `{}` snapshot endpoint does not match `{}`",
                self.id, self.endpoint_id
            ));
        }
        if snapshot
            .queues
            .iter()
            .any(|queue| queue.workflow_id() != self.workflow_id)
        {
            return Err(format!(
                "workflow publication `{}` snapshot contains a queue for another workflow",
                self.id
            ));
        }
        if snapshot.schedules.iter().any(|schedule| {
            schedule.workflow_id() != self.workflow_id
                || snapshot.workflow.endpoint(schedule.endpoint_id()).is_none()
        }) {
            return Err(format!(
                "workflow publication `{}` snapshot contains an invalid schedule",
                self.id
            ));
        }
        let referenced_agents = snapshot
            .workflow
            .nodes()
            .iter()
            .map(|node| node.agent_id())
            .collect::<BTreeSet<_>>();
        let captured_agents = snapshot
            .agents
            .iter()
            .map(|agent| agent.id())
            .collect::<BTreeSet<_>>();
        if captured_agents != referenced_agents || captured_agents.len() != snapshot.agents.len() {
            return Err(format!(
                "workflow publication `{}` snapshot agents do not exactly match workflow nodes",
                self.id
            ));
        }
        if snapshot.agents.iter().any(|agent| {
            agent.workspace_id() != Some(WORKFLOW_PUBLICATION_WORKSPACE_ROOT)
                || agent.worktree_id() != Some(WORKFLOW_PUBLICATION_WORKSPACE_ROOT)
                || agent.remote_execution().is_some()
                || !agent.provider_resume_state().is_empty()
                || agent.external_provider_import().is_some()
                || agent.remote_extension_manifest_sync().is_some()
        }) {
            return Err(format!(
                "workflow publication `{}` snapshot contains non-portable agent runtime state",
                self.id
            ));
        }
        let source_session = snapshot.source_session.as_ref().ok_or_else(|| {
            format!(
                "workflow publication `{}` snapshot is missing source session metadata",
                self.id
            )
        })?;
        if source_session.workspace_id != WORKFLOW_PUBLICATION_WORKSPACE_ROOT
            || source_session.worktree_id != WORKFLOW_PUBLICATION_WORKSPACE_ROOT
        {
            return Err(format!(
                "workflow publication `{}` snapshot contains non-portable source paths",
                self.id
            ));
        }
        Ok(())
    }

    pub fn creation_operation_key(&self) -> Option<&str> {
        self.creation_operation_key.as_deref()
    }

    pub fn runtime_materialization(&self) -> Option<&WorkflowPublicationRuntimeMaterialization> {
        self.runtime_materialization.as_ref()
    }

    pub(crate) fn set_runtime_materialization(
        &mut self,
        materialization: WorkflowPublicationRuntimeMaterialization,
    ) {
        self.runtime_materialization = Some(materialization);
    }

    pub fn apps(&self) -> Option<&Value> {
        self.apps.as_ref()
    }

    /// The App plan an export packages.
    pub(crate) fn use_apps(&mut self, plan: Value) {
        self.apps = Some(plan);
    }

    /// Records the App plan a successful export packaged as its release's.
    pub(crate) fn record_release_app_plan(&mut self, package_digest: &str, plan: Value) {
        if self.release_app_plans.is_empty() {
            self.pre_release_app_plan = self.apps.take();
        }
        self.release_app_plans
            .retain(|release| release.package_digest != package_digest);
        self.release_app_plans.push(ReleaseAppPlan {
            package_digest: package_digest.to_owned(),
            plan: plan.clone(),
        });
        // The oldest plans go first, never the bound release's: its restart
        // and recovery re-export with it.
        let bound = self
            .deployment
            .as_ref()
            .and_then(|deployment| deployment["binding"]["package_digest"].as_str())
            .map(str::to_owned);
        while self.release_app_plans.len() > MAX_RELEASE_APP_PLANS {
            let Some(oldest) = self
                .release_app_plans
                .iter()
                .position(|release| Some(&release.package_digest) != bound.as_ref())
            else {
                break;
            };
            self.release_app_plans.remove(oldest);
        }
        self.apps = Some(plan);
    }

    /// The App plan of the release with this package digest. A release
    /// exported before protocol 368 uses the publication's single plan of
    /// then; the bind's digest check rejects any other.
    pub fn release_app_plan(&self, package_digest: &str) -> Option<&Value> {
        if self.release_app_plans.is_empty() {
            return self.apps.as_ref();
        }
        self.release_app_plans
            .iter()
            .find(|release| release.package_digest == package_digest)
            .map(|release| &release.plan)
            .or(self.pre_release_app_plan.as_ref())
    }

    pub fn creation_request_digest(&self) -> Option<&str> {
        self.creation_request_digest.as_deref()
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

    /// The publication as concurrency checks compare it: its definition,
    /// without the deployment and runtime observability (binding, status,
    /// URLs, heartbeats, tunnel state, runs, logs, and the time they last
    /// changed), which change in memory between durable writes.
    pub(crate) fn without_runtime_state(&self) -> Self {
        Self {
            status: None,
            open_url: None,
            viewer_url: None,
            deployment: None,
            runtime_last_heartbeat_at_ms: None,
            runtime_last_error: None,
            runtime: None,
            schedule_count: None,
            schedules: Vec::new(),
            watchdog_count: None,
            watchdogs: Vec::new(),
            latest_run: None,
            recent_runs: Vec::new(),
            latest_output: None,
            runtime_logs: Vec::new(),
            updated_at_ms: 0,
            ..self.clone()
        }
    }

    pub fn set_runtime_observability(
        &mut self,
        runtime: Option<Value>,
        schedules: Vec<Value>,
        latest_run: Option<Value>,
        recent_runs: Vec<Value>,
        latest_output: Option<Value>,
    ) {
        let schedule_count = schedules.len() as u64;
        self.runtime = runtime;
        self.schedule_count = Some(schedule_count);
        self.schedules = schedules.clone();
        self.watchdog_count = Some(schedule_count);
        self.watchdogs = schedules;
        self.latest_run = latest_run;
        self.recent_runs = recent_runs;
        self.latest_output = latest_output;
        self.updated_at_ms = unix_epoch_ms();
    }

    pub fn set_runtime_run_observability(
        &mut self,
        latest_run: Option<Value>,
        recent_runs: Vec<Value>,
        latest_output: Option<Value>,
    ) {
        self.latest_run = latest_run;
        self.recent_runs = recent_runs;
        self.latest_output = latest_output;
        self.updated_at_ms = unix_epoch_ms();
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

#[cfg(test)]
mod release_app_plan_tests {
    use super::*;

    fn publication() -> WorkflowPublicationDefinition {
        serde_json::from_value(serde_json::json!({
            "id": "publication-1", "session_id": "session-1", "workflow_id": "workflow-1",
            "endpoint_id": "endpoint-1", "kind": "event_based", "enabled": true,
            "methods": [], "schedules": [], "watchdogs": [], "recent_runs": [], "runtime_logs": [],
            "created_by_user_id": "user-1", "created_at_ms": 1, "updated_at_ms": 1,
        }))
        .expect("publication")
    }

    #[test]
    fn a_release_exported_before_368_keeps_the_single_plan_after_new_releases_record_theirs() {
        let mut publication = publication();
        let pinned = serde_json::json!({"apps": [{"version": "1.0.0"}]});
        // A publication prepared before protocol 368: one plan, no releases.
        publication.use_apps(pinned.clone());
        assert_eq!(publication.release_app_plan("sha256:old"), Some(&pinned));
        let newer = serde_json::json!({"apps": [{"version": "1.1.0"}]});
        publication.record_release_app_plan("sha256:new", newer.clone());
        assert_eq!(publication.apps(), Some(&newer));
        assert_eq!(publication.release_app_plan("sha256:new"), Some(&newer));
        assert_eq!(publication.release_app_plan("sha256:old"), Some(&pinned));
        // Pruning keeps the bound release's plan however many exports follow.
        publication.deployment = Some(serde_json::json!({"binding": {"package_digest": "sha256:new"}}));
        for export in 0..(MAX_RELEASE_APP_PLANS + 4) {
            publication.record_release_app_plan(&format!("sha256:later-{export}"), serde_json::json!({"n": export}));
        }
        assert_eq!(publication.release_app_plans.len(), MAX_RELEASE_APP_PLANS);
        assert_eq!(publication.release_app_plan("sha256:new"), Some(&newer));
        assert_eq!(publication.release_app_plan("sha256:later-0"), Some(&pinned), "evicted: the pre-368 plan answers, and the bind's digest check decides");
        assert!(publication.release_app_plans.iter().any(|release| release.package_digest == format!("sha256:later-{}", MAX_RELEASE_APP_PLANS + 3)));
        // A publication first prepared at 368 has no such fallback.
        let mut fresh = self::publication();
        fresh.record_release_app_plan("sha256:new", newer.clone());
        assert_eq!(fresh.release_app_plan("sha256:other"), None);
    }
}
