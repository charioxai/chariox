use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowCodeCompileResult {
    pub definition: WorkflowCodeDefinition,
    pub validation: WorkflowCodeValidationReport,
    pub logs: String,
    #[serde(skip)]
    pub source_spans: BTreeMap<String, WorkflowCodeSourceSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCodeApplyReport {
    pub workflow_id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub schema_refs: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_ids: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agent_ids: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub edge_ids: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub endpoint_ids: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub queue_ids: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[serde(alias = "watchdog_ids")]
    pub schedule_ids: BTreeMap<String, String>,
    pub canvas_layout_applied: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<WorkflowCodeApplyWarning>,
}

impl WorkflowCodeApplyReport {
    pub fn for_workflow(workflow_id: impl Into<String>) -> Self {
        Self {
            workflow_id: workflow_id.into(),
            schema_refs: BTreeMap::new(),
            node_ids: BTreeMap::new(),
            agent_ids: BTreeMap::new(),
            edge_ids: BTreeMap::new(),
            endpoint_ids: BTreeMap::new(),
            queue_ids: BTreeMap::new(),
            schedule_ids: BTreeMap::new(),
            canvas_layout_applied: false,
            warnings: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCodeApplyWarning {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowCodeCompileAndApplyResult {
    pub compile: WorkflowCodeCompileResult,
    pub apply: WorkflowCodeApplyReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowCodeRunResult {
    pub apply: WorkflowCodeCompileAndApplyResult,
    pub invocation: WorkflowCodeRunInvocation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowCodeRunInvocation {
    Started {
        workflow_run: crate::session::WorkflowRun,
        workflow: crate::session::WorkflowDefinition,
        endpoint: crate::session::WorkflowEndpointDefinition,
    },
    Enqueued {
        queued_prompt: crate::session::WorkflowQueuedPrompt,
        workflow: crate::session::WorkflowDefinition,
        endpoint: crate::session::WorkflowEndpointDefinition,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCodeProviderRebinding {
    pub node: String,
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
pub struct WorkflowCodeAgentRebinding {
    pub node: String,
    pub agent_ref: String,
}

#[derive(Debug, Serialize)]
pub(super) struct WorkflowCodeCompilerInput<'a> {
    pub(super) source: &'a str,
    pub(super) language: &'static str,
    pub(super) timeout_ms: u64,
    pub(super) max_schema_bytes: u32,
    pub(super) parameters: &'a BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) schema_import_root: Option<&'a Path>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WorkflowCodeCompilerOutput {
    pub(super) ok: bool,
    #[serde(default)]
    pub(super) definition: Option<WorkflowCodeDefinition>,
    #[serde(default)]
    pub(super) source_spans: BTreeMap<String, WorkflowCodeSourceSpan>,
    #[serde(default)]
    pub(super) error: Option<String>,
    #[serde(default)]
    pub(super) logs: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCodeArtifactMetadata {
    pub name: String,
    pub language: WorkflowCodeLanguage,
    pub path: PathBuf,
    pub source_sha256: String,
    pub source_bytes: u64,
    pub validation: WorkflowCodeValidationReport,
    #[serde(default)]
    pub provenance: WorkflowCodeArtifactProvenance,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<WorkflowCodeArtifactHistoryEntry>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCodeArtifactActor {
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metaagent_id: Option<String>,
}

impl WorkflowCodeArtifactActor {
    pub fn new(user_id: impl Into<String>, metaagent_id: Option<String>) -> Self {
        Self {
            user_id: user_id.into(),
            metaagent_id,
        }
    }
}

impl Default for WorkflowCodeArtifactActor {
    fn default() -> Self {
        Self {
            user_id: "unknown".to_string(),
            metaagent_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkflowCodeArtifactProvenance {
    pub created_by: WorkflowCodeArtifactActor,
    pub updated_by: WorkflowCodeArtifactActor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCodeArtifactHistoryEntry {
    pub action: WorkflowCodeArtifactHistoryAction,
    pub at_ms: u64,
    pub actor: WorkflowCodeArtifactActor,
    pub source_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_ok: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<WorkflowCodeApplyWarning>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowCodeArtifactHistoryAction {
    Created,
    Updated,
    Imported,
    Applied,
    Run,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowCodeArtifact {
    pub metadata: WorkflowCodeArtifactMetadata,
    pub source: String,
    pub definition: WorkflowCodeDefinition,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowCodeArtifactPackage {
    pub package_version: u32,
    pub name: String,
    pub language: WorkflowCodeLanguage,
    pub source: String,
    pub source_sha256: String,
    pub source_bytes: u64,
    pub definition_sha256: String,
    pub definition: WorkflowCodeDefinition,
    pub validation: WorkflowCodeValidationReport,
    pub exported_at_ms: u64,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowCodeSourceExportFormat {
    #[default]
    Inline,
    Directory,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowCodeSourceExportAgentMode {
    #[default]
    PortableGenerated,
    ExistingAgents,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCodeSourceExportFile {
    pub path: String,
    pub contents: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowCodeSourceExport {
    pub name: String,
    pub language: WorkflowCodeLanguage,
    pub format: WorkflowCodeSourceExportFormat,
    pub source_path: String,
    pub source: String,
    pub source_sha256: String,
    pub source_bytes: u64,
    pub definition_sha256: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<WorkflowCodeSourceExportFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRegistrySourceScope {
    Workspace,
    User,
    Builtin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRegistrySourceKind {
    SingleFile,
    SourceDirectory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowRegistrySourceInput {
    SingleFile {
        source: String,
        #[serde(default)]
        source_path: Option<String>,
    },
    SourceDirectory {
        files: Vec<WorkflowCodeSourceExportFile>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRegistryValidationSummary {
    pub ok: bool,
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRegistryEntrySummary {
    #[serde(default)]
    pub endpoints: Vec<String>,
    #[serde(default)]
    pub queues: Vec<String>,
    #[serde(default)]
    pub nodes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_endpoint: Option<String>,
}

impl WorkflowRegistryEntrySummary {
    pub fn from_definition(definition: &WorkflowCodeDefinition) -> Self {
        let endpoints = definition
            .endpoints
            .iter()
            .map(|endpoint| endpoint.handle.clone())
            .collect::<Vec<_>>();
        let queues = definition
            .queues
            .iter()
            .map(|queue| queue.handle.clone())
            .collect::<Vec<_>>();
        let nodes = definition
            .nodes
            .iter()
            .map(|node| node.handle.clone())
            .collect::<Vec<_>>();
        let default_endpoint = endpoints
            .iter()
            .find(|endpoint| endpoint.as_str() == "entry")
            .cloned()
            .or_else(|| endpoints.first().cloned());
        Self {
            endpoints,
            queues,
            nodes,
            default_endpoint,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRegistryEntryMetadata {
    pub name: String,
    pub source_scope: WorkflowRegistrySourceScope,
    pub source_kind: WorkflowRegistrySourceKind,
    pub source_path: String,
    pub source_sha256: String,
    pub source_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_sha256: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub validation: WorkflowRegistryValidationSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<WorkflowRegistryEntrySummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters_schema: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowRegistryResolvedEntry {
    pub metadata: WorkflowRegistryEntryMetadata,
    pub source: String,
    pub node_path: String,
    pub schema_import_root: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub(super) struct WorkflowRegistrySummaryCacheEntry {
    pub(super) validation: WorkflowRegistryValidationSummary,
    pub(super) definition_sha256: Option<String>,
    pub(super) summary: Option<WorkflowRegistryEntrySummary>,
    pub(super) parameters_schema: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredWorkflowRegistryManifest {
    pub(super) manifest_version: u32,
    pub(super) name: String,
    pub(super) source_kind: WorkflowRegistrySourceKind,
    pub(super) source_path: String,
    pub(super) source_sha256: String,
    pub(super) source_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) definition_sha256: Option<String>,
    #[serde(default)]
    pub(super) file_sha256: BTreeMap<String, String>,
    pub(super) created_at_ms: u64,
    pub(super) updated_at_ms: u64,
    pub(super) validation: WorkflowRegistryValidationSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) summary: Option<WorkflowRegistryEntrySummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) parameters_schema: Option<Value>,
}

pub struct WorkflowRegistry {
    pub(super) workspace_root: Option<PathBuf>,
    pub(super) user_root: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct WorkflowCodeSourceExportManifest {
    pub(super) manifest_version: u32,
    pub(super) name: String,
    pub(super) language: WorkflowCodeLanguage,
    pub(super) source_path: String,
    pub(super) definition_sha256: String,
    pub(super) source_sha256: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(super) schema_paths: BTreeMap<String, String>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowCodeLanguage {
    #[serde(alias = "javascript")]
    JavaScript,
    #[serde(rename = "typescript", alias = "type_script")]
    TypeScript,
}

impl WorkflowCodeLanguage {
    pub(super) fn compiler_name(self) -> &'static str {
        match self {
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowCodeArtifactRegistry {
    pub(super) roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredWorkflowCodeArtifact {
    pub(super) name: String,
    pub(super) language: WorkflowCodeLanguage,
    pub(super) source: String,
    pub(super) source_sha256: String,
    pub(super) definition: WorkflowCodeDefinition,
    pub(super) validation: WorkflowCodeValidationReport,
    #[serde(default)]
    pub(super) provenance: WorkflowCodeArtifactProvenance,
    #[serde(default)]
    pub(super) history: Vec<WorkflowCodeArtifactHistoryEntry>,
    pub(super) created_at_ms: u64,
    pub(super) updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCodeWorkflow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flush_agent_context_before_run: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_output_schema: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkflowCodeScheduleDefinition {
    pub handle: String,
    pub endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    pub trigger: WorkflowScheduleTrigger,
    pub invocation_prompt: String,
    pub overlap_policy: WorkflowScheduleOverlapPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_runs: Option<u64>,
}

pub type WorkflowCodeWatchdogDefinition = WorkflowCodeScheduleDefinition;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowCodeScheduleDefinitionWire {
    handle: String,
    endpoint: String,
    #[serde(default)]
    queue: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    trigger: Option<WorkflowScheduleTrigger>,
    #[serde(default)]
    interval_seconds: Option<u64>,
    invocation_prompt: String,
    #[serde(default)]
    overlap_policy: Option<WorkflowScheduleOverlapPolicy>,
    #[serde(default)]
    policy: Option<WorkflowScheduleOverlapPolicy>,
    #[serde(default)]
    max_runs: Option<u64>,
    #[serde(default)]
    max_wakeups: Option<u64>,
}

impl<'de> Deserialize<'de> for WorkflowCodeScheduleDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = WorkflowCodeScheduleDefinitionWire::deserialize(deserializer)?;
        let trigger = wire
            .trigger
            .or_else(|| wire.interval_seconds.map(WorkflowScheduleTrigger::interval))
            .ok_or_else(|| serde::de::Error::missing_field("trigger"))?;
        let overlap_policy = wire
            .overlap_policy
            .or(wire.policy)
            .ok_or_else(|| serde::de::Error::missing_field("overlap_policy"))?;
        Ok(Self {
            handle: wire.handle,
            endpoint: wire.endpoint,
            queue: wire.queue,
            enabled: wire.enabled,
            trigger,
            invocation_prompt: wire.invocation_prompt,
            overlap_policy,
            max_runs: wire.max_runs.or(wire.max_wakeups),
        })
    }
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
pub struct WorkflowCodeSourceSpan {
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCodeValidationDiagnostic {
    pub severity: WorkflowCodeValidationSeverity,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<WorkflowCodeSourceSpan>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowCodeValidationSeverity {
    Error,
    Warning,
}
