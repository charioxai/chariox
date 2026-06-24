use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::config::WorkflowCodeLimitsConfig;
use crate::extension::ExtensionGrant;
use crate::mcp::validate_registry_name;
use crate::session::{
    WorkflowEdgeEndpointSide, WorkflowHandoffValidationPolicy, WorkflowWatchdogPolicy,
};

pub const WORKFLOW_CODE_SCHEMA_VERSION: u32 = 1;
pub const WORKFLOW_CODE_ARTIFACT_SOURCE_KIND: &str = "workflow_code";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCodeArtifactMetadata {
    pub name: String,
    pub language: WorkflowCodeLanguage,
    pub path: PathBuf,
    pub source_sha256: String,
    pub source_bytes: u64,
    pub validation: WorkflowCodeValidationReport,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowCodeArtifact {
    pub metadata: WorkflowCodeArtifactMetadata,
    pub source: String,
    pub definition: WorkflowCodeDefinition,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowCodeLanguage {
    JavaScript,
    TypeScript,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCodeArtifactRegistry {
    roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredWorkflowCodeArtifact {
    name: String,
    language: WorkflowCodeLanguage,
    source: String,
    source_sha256: String,
    definition: WorkflowCodeDefinition,
    validation: WorkflowCodeValidationReport,
    created_at_ms: u64,
    updated_at_ms: u64,
}

impl WorkflowCodeArtifactRegistry {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    pub fn project_root(workspace: impl AsRef<Path>) -> PathBuf {
        workspace.as_ref().join(".arroba").join("workflow-code")
    }

    pub fn user_root() -> Option<PathBuf> {
        arroba_home().map(|home| home.join("workflow-code"))
    }

    pub fn save(
        &self,
        name: &str,
        language: WorkflowCodeLanguage,
        source: impl Into<String>,
        definition: WorkflowCodeDefinition,
        limits: &WorkflowCodeLimitsConfig,
    ) -> Result<WorkflowCodeArtifact, crate::DaemonError> {
        validate_registry_name(name, "workflow-code artifact name")?;
        let validation = definition.validate_with_limits(limits);
        let source = source.into();
        let now = crate::session::unix_epoch_ms();
        let source_sha256 = sha256_hex(source.as_bytes());
        let stored = StoredWorkflowCodeArtifact {
            name: name.to_string(),
            language,
            source,
            source_sha256,
            definition,
            validation,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let path = self.artifact_path(name)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error("workflow_code.save"))?;
        }
        write_stored_artifact(&path, &stored)?;
        Ok(stored.into_artifact(path))
    }

    pub fn update(
        &self,
        name: &str,
        language: WorkflowCodeLanguage,
        source: impl Into<String>,
        definition: WorkflowCodeDefinition,
        limits: &WorkflowCodeLimitsConfig,
    ) -> Result<WorkflowCodeArtifact, crate::DaemonError> {
        validate_registry_name(name, "workflow-code artifact name")?;
        let path = self
            .find_path(name)?
            .ok_or_else(|| crate::DaemonError::LocalTransport {
                operation: "workflow_code.update",
                message: format!("workflow-code artifact `{name}` is not saved"),
            })?;
        let previous = read_stored_artifact(&path)?;
        let source = source.into();
        let validation = definition.validate_with_limits(limits);
        let stored = StoredWorkflowCodeArtifact {
            name: name.to_string(),
            language,
            source_sha256: sha256_hex(source.as_bytes()),
            source,
            definition,
            validation,
            created_at_ms: previous.created_at_ms,
            updated_at_ms: crate::session::unix_epoch_ms(),
        };
        write_stored_artifact(&path, &stored)?;
        Ok(stored.into_artifact(path))
    }

    pub fn get(&self, name: &str) -> Result<Option<WorkflowCodeArtifact>, crate::DaemonError> {
        validate_registry_name(name, "workflow-code artifact name")?;
        let Some(path) = self.find_path(name)? else {
            return Ok(None);
        };
        read_stored_artifact(&path).map(|stored| Some(stored.into_artifact(path)))
    }

    pub fn list(&self) -> Result<Vec<WorkflowCodeArtifactMetadata>, crate::DaemonError> {
        let mut entries = BTreeMap::new();
        for root in &self.roots {
            if !root.exists() {
                continue;
            }
            for entry in fs::read_dir(root).map_err(io_error("workflow_code.list"))? {
                let path = entry.map_err(io_error("workflow_code.list"))?.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    continue;
                }
                let artifact = read_stored_artifact(&path)?.into_artifact(path);
                entries
                    .entry(artifact.metadata.name.clone())
                    .or_insert(artifact.metadata);
            }
        }
        Ok(entries.into_values().collect())
    }

    pub fn delete(&self, name: &str) -> Result<PathBuf, crate::DaemonError> {
        validate_registry_name(name, "workflow-code artifact name")?;
        let path = self
            .find_path(name)?
            .ok_or_else(|| crate::DaemonError::LocalTransport {
                operation: "workflow_code.delete",
                message: format!("workflow-code artifact `{name}` is not saved"),
            })?;
        fs::remove_file(&path).map_err(io_error("workflow_code.delete"))?;
        Ok(path)
    }

    fn artifact_path(&self, name: &str) -> Result<PathBuf, crate::DaemonError> {
        Ok(self.primary_root()?.join(format!("{name}.json")))
    }

    fn find_path(&self, name: &str) -> Result<Option<PathBuf>, crate::DaemonError> {
        for root in &self.roots {
            let path = root.join(format!("{name}.json"));
            if path.exists() {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }

    fn primary_root(&self) -> Result<&PathBuf, crate::DaemonError> {
        self.roots.first().ok_or(crate::DaemonError::InvalidConfig {
            field: "workflow-code registry roots",
            message: "must include at least one root",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCodeDefinition {
    #[serde(default = "default_workflow_code_schema_version")]
    pub schema_version: u32,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub watchdogs: Vec<WorkflowCodeWatchdogDefinition>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCodeWorkflow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flush_agent_context_before_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_output_schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intermediate_output_schema: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCodeSchemaDefinition {
    pub handle: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub schema: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCodeNodeDefinition {
    pub handle: String,
    pub agent: WorkflowCodeAgentBinding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_complete_workflow_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_emit_intermediate_run_output: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_for_all_inputs: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intermediate_output_schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<ExtensionGrant>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<WorkflowCodeCanvasPoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkflowCodeAgentBinding {
    Create(WorkflowCodeAgentCreate),
    Existing(WorkflowCodeExistingAgent),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCodeAgentCreate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCodeExistingAgent {
    pub agent_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCodeEdgeDefinition {
    pub handle: String,
    pub from_node: String,
    pub to_node: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_side: Option<WorkflowEdgeEndpointSide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_side: Option<WorkflowEdgeEndpointSide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_policy: Option<WorkflowHandoffValidationPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<WorkflowCodeCanvasEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCodeEndpointDefinition {
    pub handle: String,
    pub entry_node: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<WorkflowCodeCanvasPoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCodeQueueDefinition {
    pub handle: String,
    pub alias: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCodeWatchdogDefinition {
    pub handle: String,
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<String>,
    pub interval_seconds: u64,
    pub invocation_prompt: String,
    pub policy: WorkflowWatchdogPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_wakeups: Option<u64>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCodeCanvasPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCodeCanvasEdge {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<WorkflowCodeCanvasPoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCodeValidationReport {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<WorkflowCodeValidationDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCodeValidationDiagnostic {
    pub severity: WorkflowCodeValidationSeverity,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowCodeValidationSeverity {
    Error,
    Warning,
}

struct WorkflowCodeValidator<'a> {
    limits: &'a WorkflowCodeLimitsConfig,
    diagnostics: Vec<WorkflowCodeValidationDiagnostic>,
}

impl<'a> WorkflowCodeValidator<'a> {
    fn new(limits: &'a WorkflowCodeLimitsConfig) -> Self {
        Self {
            limits,
            diagnostics: Vec::new(),
        }
    }

    fn validate(&mut self, definition: &WorkflowCodeDefinition) {
        if definition.schema_version != WORKFLOW_CODE_SCHEMA_VERSION {
            self.error(
                "unsupported_schema_version",
                format!(
                    "workflow-code schema_version {} is not supported",
                    definition.schema_version
                ),
                None,
            );
        }

        self.validate_count("nodes", definition.nodes.len(), self.limits.max_nodes);
        self.validate_count("agents", definition.nodes.len(), self.limits.max_agents);
        self.validate_count("edges", definition.edges.len(), self.limits.max_edges);
        self.validate_count("queues", definition.queues.len(), self.limits.max_queues);
        self.validate_count(
            "watchdogs",
            definition.watchdogs.len(),
            self.limits.max_watchdogs,
        );

        if definition.nodes.is_empty() {
            self.error(
                "missing_node",
                "workflow-code must define at least one node",
                None,
            );
        }
        if definition.endpoints.is_empty() {
            self.error(
                "missing_endpoint",
                "workflow-code must define at least one endpoint",
                None,
            );
        }
        if let Some(max_concurrent) = definition.workflow.max_concurrent {
            if max_concurrent == 0 {
                self.error(
                    "invalid_max_concurrent",
                    "workflow max_concurrent must not be zero",
                    None,
                );
            } else if max_concurrent > self.limits.max_concurrent {
                self.error(
                    "limit_exceeded",
                    format!(
                        "workflow max_concurrent {max_concurrent} exceeds configured limit {}",
                        self.limits.max_concurrent
                    ),
                    None,
                );
            }
        }

        let schema_handles = self.validate_schemas(definition);
        let node_handles = collect_unique_handles(
            self,
            "node",
            definition.nodes.iter().map(|node| node.handle.as_str()),
        );
        let edge_handles = collect_unique_handles(
            self,
            "edge",
            definition.edges.iter().map(|edge| edge.handle.as_str()),
        );
        let endpoint_handles = collect_unique_handles(
            self,
            "endpoint",
            definition
                .endpoints
                .iter()
                .map(|endpoint| endpoint.handle.as_str()),
        );
        let queue_handles = collect_unique_handles(
            self,
            "queue",
            definition.queues.iter().map(|queue| queue.handle.as_str()),
        );
        collect_unique_handles(
            self,
            "watchdog",
            definition
                .watchdogs
                .iter()
                .map(|watchdog| watchdog.handle.as_str()),
        );

        self.validate_schema_ref(
            &schema_handles,
            definition.workflow.run_output_schema.as_deref(),
            "workflow.run_output_schema",
            None,
        );
        self.validate_schema_ref(
            &schema_handles,
            definition.workflow.intermediate_output_schema.as_deref(),
            "workflow.intermediate_output_schema",
            None,
        );

        let mut existing_agent_refs = BTreeMap::<&str, &str>::new();
        for node in &definition.nodes {
            self.validate_agent_binding(node, &mut existing_agent_refs);
            self.validate_schema_ref(
                &schema_handles,
                node.intermediate_output_schema.as_deref(),
                "node.intermediate_output_schema",
                Some(node.handle.clone()),
            );
            if node.max_turns.is_some_and(|max_turns| max_turns == 0) {
                self.error(
                    "invalid_max_turns",
                    "node max_turns must not be zero",
                    Some(node.handle.clone()),
                );
            }
        }

        for edge in &definition.edges {
            self.validate_ref(
                &node_handles,
                &edge.from_node,
                "edge.from_node",
                Some(edge.handle.clone()),
            );
            self.validate_ref(
                &node_handles,
                &edge.to_node,
                "edge.to_node",
                Some(edge.handle.clone()),
            );
            self.validate_schema_ref(
                &schema_handles,
                edge.handoff_schema.as_deref(),
                "edge.handoff_schema",
                Some(edge.handle.clone()),
            );
        }

        for endpoint in &definition.endpoints {
            self.validate_ref(
                &node_handles,
                &endpoint.entry_node,
                "endpoint.entry_node",
                Some(endpoint.handle.clone()),
            );
        }

        for watchdog in &definition.watchdogs {
            self.validate_ref(
                &endpoint_handles,
                &watchdog.endpoint,
                "watchdog.endpoint",
                Some(watchdog.handle.clone()),
            );
            if let Some(queue) = watchdog.queue.as_deref() {
                self.validate_ref(
                    &queue_handles,
                    queue,
                    "watchdog.queue",
                    Some(watchdog.handle.clone()),
                );
            }
            if watchdog.interval_seconds == 0 {
                self.error(
                    "invalid_watchdog_interval",
                    "watchdog interval_seconds must not be zero",
                    Some(watchdog.handle.clone()),
                );
            }
            if watchdog.invocation_prompt.trim().is_empty() {
                self.error(
                    "invalid_watchdog_prompt",
                    "watchdog invocation_prompt must not be empty",
                    Some(watchdog.handle.clone()),
                );
            }
            if watchdog
                .max_wakeups
                .is_some_and(|max_wakeups| max_wakeups == 0)
            {
                self.error(
                    "invalid_watchdog_max_wakeups",
                    "watchdog max_wakeups must not be zero",
                    Some(watchdog.handle.clone()),
                );
            }
        }

        let _ = edge_handles;
    }

    fn validate_schemas(&mut self, definition: &WorkflowCodeDefinition) -> BTreeSet<String> {
        let handles = collect_unique_handles(
            self,
            "schema",
            definition
                .schemas
                .iter()
                .map(|schema| schema.handle.as_str()),
        );
        let total_schema_bytes = definition
            .schemas
            .iter()
            .map(|schema| {
                serde_json::to_vec(&schema.schema)
                    .map(|bytes| bytes.len())
                    .unwrap_or(0)
            })
            .sum::<usize>();
        if total_schema_bytes > self.limits.max_schema_bytes as usize {
            self.error(
                "limit_exceeded",
                format!(
                    "workflow schemas use {total_schema_bytes} bytes, exceeding configured limit {}",
                    self.limits.max_schema_bytes
                ),
                None,
            );
        }
        for schema in &definition.schemas {
            if let Err(error) = jsonschema::JSONSchema::compile(&schema.schema) {
                self.error(
                    "invalid_schema",
                    format!("schema failed to compile: {error}"),
                    Some(schema.handle.clone()),
                );
            }
        }
        handles
    }

    fn validate_agent_binding<'b>(
        &mut self,
        node: &'b WorkflowCodeNodeDefinition,
        existing_agent_refs: &mut BTreeMap<&'b str, &'b str>,
    ) {
        match &node.agent {
            WorkflowCodeAgentBinding::Create(agent) => {
                if agent.provider.trim().is_empty() {
                    self.error(
                        "invalid_agent_provider",
                        "generated agent provider must not be empty",
                        Some(node.handle.clone()),
                    );
                }
            }
            WorkflowCodeAgentBinding::Existing(agent) => {
                if agent.agent_ref.trim().is_empty() {
                    self.error(
                        "invalid_agent_ref",
                        "existing agent_ref must not be empty",
                        Some(node.handle.clone()),
                    );
                    return;
                }
                if let Some(existing_node) =
                    existing_agent_refs.insert(agent.agent_ref.as_str(), node.handle.as_str())
                {
                    self.error(
                        "duplicate_existing_agent",
                        format!(
                            "existing agent_ref `{}` is already bound by node `{existing_node}`",
                            agent.agent_ref
                        ),
                        Some(node.handle.clone()),
                    );
                }
            }
        }
    }

    fn validate_count(&mut self, label: &'static str, actual: usize, limit: u32) {
        if actual > limit as usize {
            self.error(
                "limit_exceeded",
                format!("{label} count {actual} exceeds configured limit {limit}"),
                None,
            );
        }
    }

    fn validate_ref(
        &mut self,
        handles: &BTreeSet<String>,
        value: &str,
        field: &'static str,
        handle: Option<String>,
    ) {
        if value.trim().is_empty() {
            self.error(
                "empty_reference",
                format!("{field} must not be empty"),
                handle,
            );
        } else if !handles.contains(value) {
            self.error(
                "unknown_reference",
                format!("{field} references unknown handle `{value}`"),
                handle,
            );
        }
    }

    fn validate_schema_ref(
        &mut self,
        schema_handles: &BTreeSet<String>,
        value: Option<&str>,
        field: &'static str,
        handle: Option<String>,
    ) {
        if let Some(value) = value {
            self.validate_ref(schema_handles, value, field, handle);
        }
    }

    fn error(&mut self, code: &'static str, message: impl Into<String>, handle: Option<String>) {
        self.diagnostics.push(WorkflowCodeValidationDiagnostic {
            severity: WorkflowCodeValidationSeverity::Error,
            code: code.to_string(),
            message: message.into(),
            handle,
        });
    }

    fn finish(self) -> WorkflowCodeValidationReport {
        let ok = self
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != WorkflowCodeValidationSeverity::Error);
        WorkflowCodeValidationReport {
            ok,
            diagnostics: self.diagnostics,
        }
    }
}

impl StoredWorkflowCodeArtifact {
    fn into_artifact(self, path: PathBuf) -> WorkflowCodeArtifact {
        let source_bytes = self.source.len() as u64;
        WorkflowCodeArtifact {
            metadata: WorkflowCodeArtifactMetadata {
                name: self.name,
                language: self.language,
                path,
                source_sha256: self.source_sha256,
                source_bytes,
                validation: self.validation,
                created_at_ms: self.created_at_ms,
                updated_at_ms: self.updated_at_ms,
            },
            source: self.source,
            definition: self.definition,
        }
    }
}

fn read_stored_artifact(path: &Path) -> Result<StoredWorkflowCodeArtifact, crate::DaemonError> {
    let contents = fs::read_to_string(path).map_err(io_error("workflow_code.read"))?;
    serde_json::from_str(&contents).map_err(|error| crate::DaemonError::LocalTransport {
        operation: "workflow_code.read",
        message: format!(
            "failed to parse workflow-code artifact `{}`: {error}",
            path.display()
        ),
    })
}

fn write_stored_artifact(
    path: &Path,
    artifact: &StoredWorkflowCodeArtifact,
) -> Result<(), crate::DaemonError> {
    let payload = serde_json::to_string_pretty(artifact).map_err(|error| {
        crate::DaemonError::LocalTransport {
            operation: "workflow_code.write",
            message: format!("failed to serialize workflow-code artifact: {error}"),
        }
    })?;
    fs::write(path, format!("{payload}\n")).map_err(io_error("workflow_code.write"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn arroba_home() -> Option<PathBuf> {
    std::env::var_os("ARROBA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".arroba")))
}

fn io_error(operation: &'static str) -> impl FnOnce(std::io::Error) -> crate::DaemonError + Copy {
    move |error| crate::DaemonError::LocalTransport {
        operation,
        message: error.to_string(),
    }
}

fn collect_unique_handles<'a>(
    validator: &mut WorkflowCodeValidator<'_>,
    kind: &'static str,
    handles: impl Iterator<Item = &'a str>,
) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    for handle in handles {
        if handle.trim().is_empty() {
            validator.error(
                "empty_handle",
                format!("{kind} handle must not be empty"),
                None,
            );
        } else if !seen.insert(handle.to_string()) {
            validator.error(
                "duplicate_handle",
                format!("{kind} handle `{handle}` is duplicated"),
                Some(handle.to_string()),
            );
        }
    }
    seen
}

fn default_workflow_code_schema_version() -> u32 {
    WORKFLOW_CODE_SCHEMA_VERSION
}

fn default_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_definition() -> WorkflowCodeDefinition {
        WorkflowCodeDefinition {
            schema_version: WORKFLOW_CODE_SCHEMA_VERSION,
            workflow: WorkflowCodeWorkflow {
                alias: Some("toy".to_string()),
                flush_agent_context_before_run: Some(true),
                max_concurrent: Some(32),
                run_output_schema: Some("final".to_string()),
                intermediate_output_schema: None,
            },
            schemas: vec![WorkflowCodeSchemaDefinition {
                handle: "final".to_string(),
                alias: Some("Final output".to_string()),
                description: None,
                schema: serde_json::json!({
                    "type": "object",
                    "properties": {"answer": {"type": "string"}},
                    "required": ["answer"],
                    "additionalProperties": false
                }),
            }],
            nodes: vec![WorkflowCodeNodeDefinition {
                handle: "planner".to_string(),
                agent: WorkflowCodeAgentBinding::Create(WorkflowCodeAgentCreate {
                    alias: Some("planner".to_string()),
                    provider: "dev-stub".to_string(),
                    model: Some("default".to_string()),
                    effort: None,
                    account_profile: None,
                }),
                public_label: Some("Planner".to_string()),
                instructions: Some("Plan the task.".to_string()),
                can_complete_workflow_run: Some(true),
                can_emit_intermediate_run_output: None,
                wait_for_all_inputs: None,
                intermediate_output_schema: None,
                max_turns: Some(4),
                extensions: Vec::new(),
                canvas: Some(WorkflowCodeCanvasPoint { x: 0, y: 0 }),
            }],
            edges: Vec::new(),
            endpoints: vec![WorkflowCodeEndpointDefinition {
                handle: "entry".to_string(),
                entry_node: "planner".to_string(),
                alias: Some("entry".to_string()),
                canvas: None,
            }],
            queues: vec![WorkflowCodeQueueDefinition {
                handle: "default".to_string(),
                alias: "default".to_string(),
                priority: 0,
                enabled: true,
            }],
            watchdogs: Vec::new(),
        }
    }

    #[test]
    fn validates_minimal_workflow_code_definition() {
        let definition = minimal_definition();
        let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());

        assert!(report.ok, "{:?}", report.diagnostics);
        assert!(report.diagnostics.is_empty());
    }

    #[test]
    fn rejects_unknown_graph_references_and_duplicate_existing_agents() {
        let mut definition = minimal_definition();
        definition.nodes.push(WorkflowCodeNodeDefinition {
            handle: "reviewer".to_string(),
            agent: WorkflowCodeAgentBinding::Existing(WorkflowCodeExistingAgent {
                agent_ref: "agent-1".to_string(),
            }),
            public_label: None,
            instructions: None,
            can_complete_workflow_run: None,
            can_emit_intermediate_run_output: None,
            wait_for_all_inputs: None,
            intermediate_output_schema: Some("missing-schema".to_string()),
            max_turns: None,
            extensions: Vec::new(),
            canvas: None,
        });
        definition.nodes.push(WorkflowCodeNodeDefinition {
            handle: "duplicate".to_string(),
            agent: WorkflowCodeAgentBinding::Existing(WorkflowCodeExistingAgent {
                agent_ref: "agent-1".to_string(),
            }),
            public_label: None,
            instructions: None,
            can_complete_workflow_run: None,
            can_emit_intermediate_run_output: None,
            wait_for_all_inputs: None,
            intermediate_output_schema: None,
            max_turns: None,
            extensions: Vec::new(),
            canvas: None,
        });
        definition.edges.push(WorkflowCodeEdgeDefinition {
            handle: "bad-edge".to_string(),
            from_node: "planner".to_string(),
            to_node: "missing-node".to_string(),
            source_side: None,
            target_side: None,
            handoff_schema: Some("missing-schema".to_string()),
            validation_policy: Some(WorkflowHandoffValidationPolicy::Warn),
            canvas: None,
        });

        let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());
        let codes = report
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>();

        assert!(!report.ok);
        assert!(codes.contains(&"unknown_reference"));
        assert!(codes.contains(&"duplicate_existing_agent"));
    }

    #[test]
    fn enforces_configured_limits() {
        let mut definition = minimal_definition();
        definition.workflow.max_concurrent = Some(64);
        let limits = WorkflowCodeLimitsConfig {
            max_concurrent: 32,
            max_nodes: 0,
            ..WorkflowCodeLimitsConfig::default()
        };

        let report = definition.validate_with_limits(&limits);

        assert!(!report.ok);
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "limit_exceeded"));
    }

    #[test]
    fn rejects_unknown_fields_during_decode() {
        let value = serde_json::json!({
            "workflow": {},
            "nodes": [],
            "endpoints": [],
            "invented": true
        });

        let error = serde_json::from_value::<WorkflowCodeDefinition>(value)
            .expect_err("unknown workflow-code fields should be rejected");

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn registry_saves_lists_reads_updates_and_deletes_artifacts() {
        let root = std::env::temp_dir().join(format!(
            "arroba-workflow-code-registry-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let registry = WorkflowCodeArtifactRegistry::new(vec![root.clone()]);
        let limits = WorkflowCodeLimitsConfig::default();
        let definition = minimal_definition();

        let created = registry
            .save(
                "toy",
                WorkflowCodeLanguage::JavaScript,
                "workflow.define({ alias: 'toy' })",
                definition.clone(),
                &limits,
            )
            .expect("workflow-code artifact should save");

        assert_eq!(created.metadata.name, "toy");
        assert_eq!(created.metadata.language, WorkflowCodeLanguage::JavaScript);
        assert_eq!(created.metadata.source_bytes, 33);
        assert!(created.metadata.validation.ok);
        assert_eq!(registry.list().expect("list should load").len(), 1);

        let loaded = registry
            .get("toy")
            .expect("get should succeed")
            .expect("artifact should exist");
        assert_eq!(loaded.source, "workflow.define({ alias: 'toy' })");
        assert_eq!(loaded.definition, definition);

        let updated = registry
            .update(
                "toy",
                WorkflowCodeLanguage::TypeScript,
                "workflow.define({ alias: 'toy-2' })",
                minimal_definition(),
                &limits,
            )
            .expect("workflow-code artifact should update");
        assert_eq!(updated.metadata.language, WorkflowCodeLanguage::TypeScript);
        assert_eq!(
            updated.metadata.created_at_ms,
            created.metadata.created_at_ms
        );
        assert!(updated.metadata.updated_at_ms >= created.metadata.updated_at_ms);

        let deleted_path = registry.delete("toy").expect("artifact should delete");
        assert!(!deleted_path.exists());
        assert!(registry.get("toy").expect("get should succeed").is_none());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn registry_persists_validation_report_for_invalid_artifact() {
        let root = std::env::temp_dir().join(format!(
            "arroba-workflow-code-invalid-registry-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let registry = WorkflowCodeArtifactRegistry::new(vec![root.clone()]);
        let mut definition = minimal_definition();
        definition.endpoints.clear();

        let artifact = registry
            .save(
                "invalid",
                WorkflowCodeLanguage::JavaScript,
                "workflow.define({ alias: 'invalid' })",
                definition,
                &WorkflowCodeLimitsConfig::default(),
            )
            .expect("invalid workflow-code artifact should still save diagnostics");

        assert!(!artifact.metadata.validation.ok);
        assert!(artifact
            .metadata
            .validation
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "missing_endpoint"));

        let _ = fs::remove_dir_all(root);
    }
}
