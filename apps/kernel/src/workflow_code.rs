use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use wait_timeout::ChildExt;

use crate::config::WorkflowCodeLimitsConfig;
use crate::extension::{ExtensionGrant, ExtensionKind};
use crate::mcp::validate_registry_name;
use crate::session::{
    WorkflowEdgeEndpointSide, WorkflowHandoffValidationPolicy, WorkflowWatchdogPolicy,
};

pub const WORKFLOW_CODE_SCHEMA_VERSION: u32 = 1;
pub const WORKFLOW_CODE_ARTIFACT_PACKAGE_VERSION: u32 = 2;
pub const WORKFLOW_CODE_ARTIFACT_SOURCE_KIND: &str = "workflow_code";
const WORKFLOW_CODE_ARTIFACT_HISTORY_LIMIT: usize = 100;

#[derive(Debug, Clone, Copy)]
pub struct WorkflowCodePatternExample {
    pub slug: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
    pub path: &'static str,
    pub source: &'static str,
}

pub const WORKFLOW_CODE_PATTERN_EXAMPLES: &[WorkflowCodePatternExample] = &[
    WorkflowCodePatternExample {
        slug: "prompt-chaining",
        title: "Prompt chaining",
        summary: "Two nodes: a drafter hands a structured draft to a refiner.",
        path: "examples/workflow-code/prompt-chaining.js",
        source: include_str!("../../../examples/workflow-code/prompt-chaining.js"),
    },
    WorkflowCodePatternExample {
        slug: "routing",
        title: "Classify and act / routing",
        summary: "A classifier routes work to one of two specialist nodes.",
        path: "examples/workflow-code/routing.js",
        source: include_str!("../../../examples/workflow-code/routing.js"),
    },
    WorkflowCodePatternExample {
        slug: "fan-out-synthesize",
        title: "Fan-out and synthesize",
        summary: "A planner fans out to two workers, then a synthesizer waits for both inputs.",
        path: "examples/workflow-code/fan-out-synthesize.js",
        source: include_str!("../../../examples/workflow-code/fan-out-synthesize.js"),
    },
    WorkflowCodePatternExample {
        slug: "parallelization",
        title: "Parallelization",
        summary: "A dispatcher sends the same task to two reviewers, then an aggregator waits for both votes.",
        path: "examples/workflow-code/parallelization.js",
        source: include_str!("../../../examples/workflow-code/parallelization.js"),
    },
    WorkflowCodePatternExample {
        slug: "adversarial-verification",
        title: "Adversarial verification",
        summary: "A proposer, critic, and judge collaborate with a critique loop.",
        path: "examples/workflow-code/adversarial-verification.js",
        source: include_str!("../../../examples/workflow-code/adversarial-verification.js"),
    },
    WorkflowCodePatternExample {
        slug: "generate-filter",
        title: "Generate and filter",
        summary: "A generator creates candidates, a filter selects them, and a finisher completes.",
        path: "examples/workflow-code/generate-filter.js",
        source: include_str!("../../../examples/workflow-code/generate-filter.js"),
    },
    WorkflowCodePatternExample {
        slug: "tournament",
        title: "Tournament",
        summary: "A seeder fans out to two contestants, then a judge selects a winner.",
        path: "examples/workflow-code/tournament.js",
        source: include_str!("../../../examples/workflow-code/tournament.js"),
    },
    WorkflowCodePatternExample {
        slug: "loop-until-done",
        title: "Loop until done",
        summary: "A worker and checker loop until the checker accepts final output.",
        path: "examples/workflow-code/loop-until-done.js",
        source: include_str!("../../../examples/workflow-code/loop-until-done.js"),
    },
    WorkflowCodePatternExample {
        slug: "orchestrator-workers",
        title: "Orchestrator-workers",
        summary: "An orchestrator delegates to a worker and a synthesizer produces final output.",
        path: "examples/workflow-code/orchestrator-workers.js",
        source: include_str!("../../../examples/workflow-code/orchestrator-workers.js"),
    },
    WorkflowCodePatternExample {
        slug: "evaluator-optimizer",
        title: "Evaluator-optimizer",
        summary: "An optimizer produces candidates and an evaluator loops back or accepts.",
        path: "examples/workflow-code/evaluator-optimizer.js",
        source: include_str!("../../../examples/workflow-code/evaluator-optimizer.js"),
    },
];

const NODE_WORKFLOW_CODE_COMPILER: &str = r#"
import fs from "node:fs"
import path from "node:path"
import vm from "node:vm"

const chunks = []
for await (const chunk of process.stdin) chunks.push(chunk)
const input = JSON.parse(Buffer.concat(chunks).toString() || "{}")
const logs = []

function createBuilder() {
  let nextSchema = 1
  let nextNode = 1
  let nextEdge = 1
  let nextEndpoint = 1
  let nextQueue = 1
  let nextWatchdog = 1
  const sourceSpans = {}
  const state = {
    schema_version: 1,
    workflow: {},
    schemas: [],
    nodes: [],
    edges: [],
    endpoints: [],
    queues: [],
    watchdogs: []
  }
  function handle(kind, explicit) {
    return explicit || `${kind}:${kind === "schema" ? nextSchema++ : kind === "node" ? nextNode++ : kind === "edge" ? nextEdge++ : kind === "endpoint" ? nextEndpoint++ : kind === "queue" ? nextQueue++ : nextWatchdog++}`
  }
  function sourceSpan() {
    const stack = String(new Error().stack || "")
    const frame = stack
      .split("\n")
      .map((line) => line.match(/workflow-code\.js:(\d+):(\d+)/))
      .find(Boolean)
    if (!frame) return undefined
    const line = Math.max(1, Number(frame[1]) - 1)
    const column = Math.max(1, Number(frame[2]))
    return { start_line: line, start_column: column, end_line: line, end_column: column }
  }
  function recordSourceSpan(handle) {
    const span = sourceSpan()
    if (span) sourceSpans[handle] = span
  }
  function loadSchemaFromFile(schemaPath) {
    if (typeof schemaPath !== "string" || schemaPath.trim() === "") {
      throw new Error("schemaFromFile path must be a non-empty string")
    }
    if (!input.schema_import_root) {
      throw new Error("schemaFromFile requires an approved schema import root")
    }
    if (path.isAbsolute(schemaPath)) {
      throw new Error("schemaFromFile path must be relative to the approved import root")
    }
    const normalized = path.normalize(schemaPath)
    if (normalized === "." || normalized.startsWith("..") || path.isAbsolute(normalized)) {
      throw new Error("schemaFromFile path must stay inside the approved import root")
    }
    const root = fs.realpathSync(String(input.schema_import_root))
    const candidate = path.resolve(root, normalized)
    const resolved = fs.realpathSync(candidate)
    if (resolved !== root && !resolved.startsWith(root + path.sep)) {
      throw new Error("schemaFromFile path resolves outside the approved import root")
    }
    const stat = fs.statSync(resolved)
    if (!stat.isFile()) {
      throw new Error("schemaFromFile path must point to a JSON schema file")
    }
    if (path.extname(resolved) !== ".json") {
      throw new Error("schemaFromFile path must end in .json")
    }
    const maxBytes = Math.max(1, Number(input.max_schema_bytes || 1048576))
    if (stat.size > maxBytes) {
      throw new Error(`schemaFromFile ${schemaPath} exceeds configured schema byte limit of ${maxBytes}`)
    }
    try {
      return JSON.parse(fs.readFileSync(resolved, "utf8"))
    } catch (error) {
      throw new Error(`schemaFromFile failed to parse ${schemaPath}: ${error && error.message ? error.message : error}`)
    }
  }
  function ref(value, expected) {
    if (typeof value === "string") return value
    if (value && value.__workflowCodeHandle === expected) return value.handle
    throw new Error(`expected ${expected} handle`)
  }
  function agent(value) {
    if (value && value.__workflowCodeAgent) return value.binding
    if (value && typeof value === "object" && typeof value.kind === "string") return value
    throw new Error("node agent must be created with workflow.newAgent or workflow.existingAgent")
  }
  const api = {
    define(options = {}) {
      state.workflow = {
        ...state.workflow,
        ...(options.alias !== undefined ? { alias: options.alias } : {}),
        ...(options.prompt !== undefined ? { prompt: options.prompt } : {}),
        ...(options.flushAgentContextBeforeRun !== undefined ? { flush_agent_context_before_run: options.flushAgentContextBeforeRun } : {}),
        ...(options.maxConcurrent !== undefined ? { max_concurrent: options.maxConcurrent } : {}),
        ...(options.runOutputSchema !== undefined ? { run_output_schema: ref(options.runOutputSchema, "schema") } : {}),
        ...(options.intermediateOutputSchema !== undefined ? { intermediate_output_schema: ref(options.intermediateOutputSchema, "schema") } : {})
      }
      return api
    },
    schema(options = {}) {
      const item = {
        handle: handle("schema", options.handle),
        ...(options.alias !== undefined ? { alias: options.alias } : {}),
        ...(options.description !== undefined ? { description: options.description } : {}),
        schema: options.schema
      }
      recordSourceSpan(item.handle)
      state.schemas.push(item)
      return { __workflowCodeHandle: "schema", handle: item.handle }
    },
    schemaFromFile(pathOrOptions, maybeOptions = {}) {
      const options = typeof pathOrOptions === "string"
        ? { ...maybeOptions, path: pathOrOptions }
        : { ...(pathOrOptions || {}) }
      const item = {
        handle: handle("schema", options.handle),
        ...(options.alias !== undefined ? { alias: options.alias } : {}),
        ...(options.description !== undefined ? { description: options.description } : {}),
        schema: loadSchemaFromFile(options.path)
      }
      recordSourceSpan(item.handle)
      state.schemas.push(item)
      return { __workflowCodeHandle: "schema", handle: item.handle }
    },
    newAgent(options = {}) {
      return {
        __workflowCodeAgent: true,
        binding: {
          kind: "create",
          ...(options.alias !== undefined ? { alias: options.alias } : {}),
          provider: options.provider,
          ...(options.model !== undefined ? { model: options.model } : {}),
          ...(options.effort !== undefined ? { effort: options.effort } : {}),
          ...(options.accountProfile !== undefined ? { account_profile: options.accountProfile } : {})
        }
      }
    },
    existingAgent(agentRef) {
      return {
        __workflowCodeAgent: true,
        binding: { kind: "existing", agent_ref: agentRef }
      }
    },
    node(options = {}) {
      const item = {
        handle: handle("node", options.handle),
        agent: agent(options.agent),
        ...(options.publicLabel !== undefined ? { public_label: options.publicLabel } : {}),
        ...(options.instructions !== undefined ? { instructions: options.instructions } : {}),
        ...(options.canCompleteWorkflowRun !== undefined ? { can_complete_workflow_run: options.canCompleteWorkflowRun } : {}),
        ...(options.canEmitIntermediateRunOutput !== undefined ? { can_emit_intermediate_run_output: options.canEmitIntermediateRunOutput } : {}),
        ...(options.waitForAllInputs !== undefined ? { wait_for_all_inputs: options.waitForAllInputs } : {}),
        ...(options.intermediateOutputSchema !== undefined ? { intermediate_output_schema: ref(options.intermediateOutputSchema, "schema") } : {}),
        ...(options.maxTurns !== undefined ? { max_turns: options.maxTurns } : {}),
        ...(options.extensions !== undefined ? { extensions: options.extensions } : {}),
        ...(options.canvas !== undefined ? { canvas: options.canvas } : {})
      }
      recordSourceSpan(item.handle)
      state.nodes.push(item)
      return { __workflowCodeHandle: "node", handle: item.handle }
    },
    edge(fromNode, toNode, options = {}) {
      const item = {
        handle: handle("edge", options.handle),
        from_node: ref(fromNode, "node"),
        to_node: ref(toNode, "node"),
        ...(options.sourceSide !== undefined ? { source_side: options.sourceSide } : {}),
        ...(options.targetSide !== undefined ? { target_side: options.targetSide } : {}),
        ...(options.handoffSchema !== undefined ? { handoff_schema: ref(options.handoffSchema, "schema") } : {}),
        ...(options.validationPolicy !== undefined ? { validation_policy: options.validationPolicy } : {}),
        ...(options.canvas !== undefined ? { canvas: options.canvas } : {})
      }
      recordSourceSpan(item.handle)
      state.edges.push(item)
      return { __workflowCodeHandle: "edge", handle: item.handle }
    },
    endpoint(entryNode, options = {}) {
      const item = {
        handle: handle("endpoint", options.handle),
        entry_node: ref(entryNode, "node"),
        ...(options.alias !== undefined ? { alias: options.alias } : {}),
        ...(options.canvas !== undefined ? { canvas: options.canvas } : {})
      }
      recordSourceSpan(item.handle)
      state.endpoints.push(item)
      return { __workflowCodeHandle: "endpoint", handle: item.handle }
    },
    queue(options = {}) {
      const item = {
        handle: handle("queue", options.handle),
        alias: options.alias || "default",
        ...(options.priority !== undefined ? { priority: options.priority } : {}),
        ...(options.enabled !== undefined ? { enabled: options.enabled } : {})
      }
      recordSourceSpan(item.handle)
      state.queues.push(item)
      return { __workflowCodeHandle: "queue", handle: item.handle }
    },
    watchdog(endpoint, options = {}) {
      const item = {
        handle: handle("watchdog", options.handle),
        endpoint: ref(endpoint, "endpoint"),
        ...(options.queue !== undefined ? { queue: ref(options.queue, "queue") } : {}),
        ...(options.enabled !== undefined ? { enabled: options.enabled } : {}),
        interval_seconds: options.intervalSeconds,
        invocation_prompt: options.invocationPrompt,
        policy: options.policy,
        ...(options.maxWakeups !== undefined ? { max_wakeups: options.maxWakeups } : {})
      }
      recordSourceSpan(item.handle)
      state.watchdogs.push(item)
      return { __workflowCodeHandle: "watchdog", handle: item.handle }
    },
    export() {
      return state
    },
    __sourceSpans() {
      return sourceSpans
    }
  }
  return api
}

try {
  let source = String(input.source || "")
  if (input.language === "typescript") {
    const mod = await import("node:module")
    if (typeof mod.stripTypeScriptTypes !== "function") {
      throw new Error("TypeScript workflow-code requires Node.js with node:module stripTypeScriptTypes support")
    }
    source = mod.stripTypeScriptTypes(source, { mode: "transform" })
  }
  const workflow = createBuilder()
  const context = vm.createContext({
    workflow,
    console: {
      log: (...values) => logs.push(values.map(String).join(" ")),
      error: (...values) => logs.push(values.map(String).join(" "))
    }
  })
  const wrapped = `(async () => {\n${source}\nif (typeof defineWorkflow === "function") await defineWorkflow(workflow)\nreturn workflow.export()\n})()`
  const script = new vm.Script(wrapped, { filename: "workflow-code.js" })
  const definition = await script.runInContext(context, { timeout: Math.max(1, Number(input.timeout_ms || 30000)) })
  console.log(JSON.stringify({ ok: true, definition, source_spans: workflow.__sourceSpans(), logs: logs.join("\n") }))
} catch (error) {
  console.log(JSON.stringify({
    ok: false,
    error: String(error && error.message ? error.message : error),
    logs: logs.join("\n")
  }))
}
"#;

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
    pub watchdog_ids: BTreeMap<String, String>,
    pub canvas_layout_applied: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<WorkflowCodeApplyWarning>,
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

#[derive(Debug, Serialize)]
struct WorkflowCodeCompilerInput<'a> {
    source: &'a str,
    language: &'static str,
    timeout_ms: u64,
    max_schema_bytes: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema_import_root: Option<&'a Path>,
}

#[derive(Debug, Deserialize)]
struct WorkflowCodeCompilerOutput {
    ok: bool,
    #[serde(default)]
    definition: Option<WorkflowCodeDefinition>,
    #[serde(default)]
    source_spans: BTreeMap<String, WorkflowCodeSourceSpan>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    logs: Option<String>,
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

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowCodeLanguage {
    JavaScript,
    #[serde(rename = "typescript", alias = "type_script")]
    TypeScript,
}

impl WorkflowCodeLanguage {
    fn compiler_name(self) -> &'static str {
        match self {
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
        }
    }
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
    #[serde(default)]
    provenance: WorkflowCodeArtifactProvenance,
    #[serde(default)]
    history: Vec<WorkflowCodeArtifactHistoryEntry>,
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
        validation: WorkflowCodeValidationReport,
        actor: WorkflowCodeArtifactActor,
        action: WorkflowCodeArtifactHistoryAction,
    ) -> Result<WorkflowCodeArtifact, crate::DaemonError> {
        validate_registry_name(name, "workflow-code artifact name")?;
        if self.find_path(name)?.is_some() {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.save",
                message: format!("workflow-code artifact `{name}` is already saved"),
            });
        }
        let source = source.into();
        let now = crate::session::unix_epoch_ms();
        let source_sha256 = sha256_hex(source.as_bytes());
        let provenance = WorkflowCodeArtifactProvenance {
            created_by: actor.clone(),
            updated_by: actor.clone(),
        };
        let history = workflow_code_artifact_history(vec![workflow_code_artifact_history_entry(
            action,
            now,
            actor,
            source_sha256.clone(),
            Some(validation.ok),
            None,
            Vec::new(),
        )]);
        let stored = StoredWorkflowCodeArtifact {
            name: name.to_string(),
            language,
            source,
            source_sha256,
            definition,
            validation,
            provenance,
            history,
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
        validation: WorkflowCodeValidationReport,
        actor: WorkflowCodeArtifactActor,
        action: WorkflowCodeArtifactHistoryAction,
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
        let now = crate::session::unix_epoch_ms();
        let source_sha256 = sha256_hex(source.as_bytes());
        let mut provenance = previous.provenance.clone();
        provenance.updated_by = actor.clone();
        let mut history = previous.history.clone();
        history.push(workflow_code_artifact_history_entry(
            action,
            now,
            actor,
            source_sha256.clone(),
            Some(validation.ok),
            None,
            Vec::new(),
        ));
        let stored = StoredWorkflowCodeArtifact {
            name: name.to_string(),
            language,
            source_sha256,
            source,
            definition,
            validation,
            provenance,
            history: workflow_code_artifact_history(history),
            created_at_ms: previous.created_at_ms,
            updated_at_ms: now,
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

    pub fn export_package(
        &self,
        name: &str,
    ) -> Result<WorkflowCodeArtifactPackage, crate::DaemonError> {
        let artifact = self
            .get(name)?
            .ok_or_else(|| crate::DaemonError::LocalTransport {
                operation: "workflow_code.export",
                message: format!("workflow-code artifact `{name}` is not saved"),
            })?;
        Ok(artifact.into_package())
    }

    pub fn import_package(
        &self,
        name_override: Option<&str>,
        package: WorkflowCodeArtifactPackage,
        definition: WorkflowCodeDefinition,
        validation: WorkflowCodeValidationReport,
        actor: WorkflowCodeArtifactActor,
        overwrite: bool,
    ) -> Result<WorkflowCodeArtifact, crate::DaemonError> {
        package.validate_integrity()?;
        let name = name_override.unwrap_or(package.name.as_str());
        validate_registry_name(name, "workflow-code artifact name")?;
        let existing = self.get(name)?.is_some();
        if overwrite && existing {
            self.update(
                name,
                package.language,
                package.source,
                definition,
                validation,
                actor,
                WorkflowCodeArtifactHistoryAction::Imported,
            )
        } else {
            self.save(
                name,
                package.language,
                package.source,
                definition,
                validation,
                actor,
                WorkflowCodeArtifactHistoryAction::Imported,
            )
        }
    }

    pub fn record_apply_history(
        &self,
        name: &str,
        actor: WorkflowCodeArtifactActor,
        action: WorkflowCodeArtifactHistoryAction,
        report: &WorkflowCodeApplyReport,
    ) -> Result<WorkflowCodeArtifact, crate::DaemonError> {
        validate_registry_name(name, "workflow-code artifact name")?;
        let path = self
            .find_path(name)?
            .ok_or_else(|| crate::DaemonError::LocalTransport {
                operation: "workflow_code.history",
                message: format!("workflow-code artifact `{name}` is not saved"),
            })?;
        let mut stored = read_stored_artifact(&path)?;
        let now = crate::session::unix_epoch_ms();
        stored.updated_at_ms = now;
        stored.provenance.updated_by = actor.clone();
        stored.history.push(workflow_code_artifact_history_entry(
            action,
            now,
            actor,
            stored.source_sha256.clone(),
            None,
            Some(report.workflow_id.clone()),
            report.warnings.clone(),
        ));
        stored.history = workflow_code_artifact_history(stored.history);
        write_stored_artifact(&path, &stored)?;
        Ok(stored.into_artifact(path))
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

pub fn compile_workflow_code_javascript(
    node_path: impl AsRef<Path>,
    source: &str,
    limits: &WorkflowCodeLimitsConfig,
) -> Result<WorkflowCodeCompileResult, crate::DaemonError> {
    compile_workflow_code_source_with_schema_import_root(
        node_path,
        source,
        WorkflowCodeLanguage::JavaScript,
        limits,
        None,
    )
}

pub fn compile_workflow_code_javascript_with_schema_import_root(
    node_path: impl AsRef<Path>,
    source: &str,
    limits: &WorkflowCodeLimitsConfig,
    schema_import_root: Option<&Path>,
) -> Result<WorkflowCodeCompileResult, crate::DaemonError> {
    compile_workflow_code_source_with_schema_import_root(
        node_path,
        source,
        WorkflowCodeLanguage::JavaScript,
        limits,
        schema_import_root,
    )
}

pub fn compile_workflow_code_source_with_schema_import_root(
    node_path: impl AsRef<Path>,
    source: &str,
    language: WorkflowCodeLanguage,
    limits: &WorkflowCodeLimitsConfig,
    schema_import_root: Option<&Path>,
) -> Result<WorkflowCodeCompileResult, crate::DaemonError> {
    let max_old_space_mb = u64::max(16, limits.script_memory_bytes.div_ceil(1024 * 1024));
    let mut child = Command::new(node_path.as_ref())
        .arg(format!("--max-old-space-size={max_old_space_mb}"))
        .arg("--input-type=module")
        .arg("-e")
        .arg(NODE_WORKFLOW_CODE_COMPILER)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| crate::DaemonError::LocalTransport {
            operation: "workflow_code.compile",
            message: format!("failed to start Node workflow-code compiler: {error}"),
        })?;

    let input = serde_json::to_vec(&WorkflowCodeCompilerInput {
        source,
        language: language.compiler_name(),
        timeout_ms: limits.script_timeout_ms,
        max_schema_bytes: limits.max_schema_bytes,
        schema_import_root,
    })
    .map_err(|error| crate::DaemonError::LocalTransport {
        operation: "workflow_code.compile",
        message: format!("failed to serialize workflow-code compiler input: {error}"),
    })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| crate::DaemonError::LocalTransport {
            operation: "workflow_code.compile",
            message: "failed to open Node workflow-code compiler stdin".to_string(),
        })?;
    stdin
        .write_all(&input)
        .map_err(io_error("workflow_code.compile"))?;
    drop(stdin);

    let timeout = Duration::from_millis(limits.script_timeout_ms);
    match child
        .wait_timeout(timeout)
        .map_err(io_error("workflow_code.compile"))?
    {
        Some(_) => {}
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.compile",
                message: format!(
                    "workflow-code script exceeded configured timeout of {} ms",
                    limits.script_timeout_ms
                ),
            });
        }
    }

    let output = child
        .wait_with_output()
        .map_err(io_error("workflow_code.compile"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(crate::DaemonError::LocalTransport {
            operation: "workflow_code.compile",
            message: format!(
                "Node workflow-code compiler failed with status {}: {}{}",
                output.status,
                stderr.trim(),
                if stdout.trim().is_empty() {
                    String::new()
                } else {
                    format!("\nstdout: {}", stdout.trim())
                }
            ),
        });
    }

    let compiler_output = serde_json::from_str::<WorkflowCodeCompilerOutput>(stdout.trim())
        .map_err(|error| crate::DaemonError::LocalTransport {
            operation: "workflow_code.compile",
            message: format!("failed to parse Node workflow-code compiler output: {error}"),
        })?;
    let logs = compiler_output.logs.unwrap_or_default();
    if !compiler_output.ok {
        return Err(crate::DaemonError::LocalTransport {
            operation: "workflow_code.compile",
            message: compiler_output
                .error
                .unwrap_or_else(|| "workflow-code script failed".to_string()),
        });
    }
    let definition =
        compiler_output
            .definition
            .ok_or_else(|| crate::DaemonError::LocalTransport {
                operation: "workflow_code.compile",
                message: "Node workflow-code compiler did not return a definition".to_string(),
            })?;
    let source_spans = compiler_output.source_spans;
    let mut validation = definition.validate_with_limits(limits);
    attach_workflow_code_diagnostic_spans(&mut validation, &source_spans);
    Ok(WorkflowCodeCompileResult {
        definition,
        validation,
        logs,
        source_spans,
    })
}

pub fn discover_workflow_code_node_path() -> Result<PathBuf, crate::DaemonError> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("NODE") {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/node"),
        PathBuf::from("/usr/local/bin/node"),
        PathBuf::from("/usr/bin/node"),
    ]);
    if let Some(path) = env::var_os("PATH") {
        for dir in env::split_paths(&path) {
            candidates.push(dir.join("node"));
        }
    }
    candidates.sort();
    candidates.dedup();
    candidates
        .into_iter()
        .find(|candidate| {
            Command::new(candidate)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
        .ok_or_else(|| crate::DaemonError::LocalTransport {
            operation: "workflow_code.compile",
            message:
                "could not find Node.js for workflow-code compilation; pass node_path or set NODE"
                    .to_string(),
        })
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
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
        self.validate_count(
            "queues",
            workflow_code_materialized_queue_count(definition),
            self.limits.max_queues,
        );
        self.validate_count(
            "watchdogs",
            definition.watchdogs.len(),
            self.limits.max_watchdogs,
        );
        let generated_prompt_bytes = workflow_code_generated_prompt_bytes(definition);
        if generated_prompt_bytes > self.limits.max_generated_prompt_bytes as usize {
            self.error(
                "limit_exceeded",
                format!(
                    "workflow generated prompt text uses {generated_prompt_bytes} bytes, exceeding configured limit {}",
                    self.limits.max_generated_prompt_bytes
                ),
                None,
            );
        }

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
        self.validate_alias(definition.workflow.alias.as_deref(), "workflow.alias", None);

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
        let mut queue_handles = collect_unique_handles(
            self,
            "queue",
            definition.queues.iter().map(|queue| queue.handle.as_str()),
        );
        queue_handles.insert("default".to_string());
        self.validate_queues(definition);
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
            self.validate_node_extensions(node);
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
            if edge.from_node == edge.to_node {
                self.error(
                    "invalid_edge",
                    "edge source and target nodes must be different",
                    Some(edge.handle.clone()),
                );
            }
            self.validate_schema_ref(
                &schema_handles,
                edge.handoff_schema.as_deref(),
                "edge.handoff_schema",
                Some(edge.handle.clone()),
            );
        }
        self.validate_edge_pairs(definition);

        for endpoint in &definition.endpoints {
            self.validate_ref(
                &node_handles,
                &endpoint.entry_node,
                "endpoint.entry_node",
                Some(endpoint.handle.clone()),
            );
        }
        self.validate_endpoint_aliases(definition);

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

    fn validate_queues(&mut self, definition: &WorkflowCodeDefinition) {
        let mut aliases = BTreeMap::<String, String>::new();
        for queue in &definition.queues {
            let Some(normalized) = self.validate_alias(
                Some(queue.alias.as_str()),
                "queue.alias",
                Some(queue.handle.clone()),
            ) else {
                continue;
            };
            if let Some(existing_handle) = aliases.insert(normalized.clone(), queue.handle.clone())
            {
                self.error(
                    "duplicate_queue_alias",
                    format!(
                        "queue alias `{normalized}` is already used by queue `{existing_handle}`"
                    ),
                    Some(queue.handle.clone()),
                );
            }
        }
    }

    fn validate_edge_pairs(&mut self, definition: &WorkflowCodeDefinition) {
        let mut pairs = BTreeMap::<(&str, &str), &str>::new();
        for edge in &definition.edges {
            let pair = (edge.from_node.as_str(), edge.to_node.as_str());
            if let Some(existing_handle) = pairs.insert(pair, edge.handle.as_str()) {
                self.error(
                    "duplicate_edge",
                    format!(
                        "edge `{}` duplicates source-target pair from edge `{existing_handle}`",
                        edge.handle
                    ),
                    Some(edge.handle.clone()),
                );
            }
        }
    }

    fn validate_endpoint_aliases(&mut self, definition: &WorkflowCodeDefinition) {
        let mut aliases = BTreeMap::<String, String>::new();
        for endpoint in &definition.endpoints {
            let Some(normalized) = self.validate_alias(
                endpoint.alias.as_deref(),
                "endpoint.alias",
                Some(endpoint.handle.clone()),
            ) else {
                continue;
            };
            if let Some(existing_handle) =
                aliases.insert(normalized.clone(), endpoint.handle.clone())
            {
                self.error(
                    "duplicate_endpoint_alias",
                    format!(
                        "endpoint alias `{normalized}` is already used by endpoint `{existing_handle}`"
                    ),
                    Some(endpoint.handle.clone()),
                );
            }
        }
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

    fn validate_node_extensions(&mut self, node: &WorkflowCodeNodeDefinition) {
        for grant in &node.extensions {
            if grant.name.trim().is_empty() {
                self.error(
                    "invalid_extension_name",
                    "extension name must not be empty",
                    Some(node.handle.clone()),
                );
            }
            match &grant.kind {
                ExtensionKind::Script => {
                    if grant
                        .environment
                        .as_deref()
                        .is_none_or(|environment| environment.trim().is_empty())
                    {
                        self.error(
                            "invalid_extension_environment",
                            "script extension requirements must include environment",
                            Some(node.handle.clone()),
                        );
                    }
                }
                ExtensionKind::Connector => {
                    if let Err(error) =
                        crate::connector::ConnectorSafety::parse(grant.max_safety.as_deref())
                    {
                        self.error(
                            "invalid_connector_safety",
                            format!("connector extension safety is invalid: {error}"),
                            Some(node.handle.clone()),
                        );
                    }
                }
                ExtensionKind::Mcp | ExtensionKind::Skill => {}
            }
        }
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

    fn validate_alias(
        &mut self,
        value: Option<&str>,
        field: &'static str,
        handle: Option<String>,
    ) -> Option<String> {
        let value = value?;
        let normalized = value.trim().to_lowercase();
        if normalized.is_empty() {
            self.error(
                "invalid_alias",
                format!("{field} must not be empty"),
                handle,
            );
            return None;
        }
        if !normalized.chars().all(|char| {
            char.is_ascii_lowercase() || char.is_ascii_digit() || char == '-' || char == '_'
        }) {
            self.error(
                "invalid_alias",
                format!("{field} must use lowercase letters, digits, `-`, or `_`"),
                handle,
            );
            return None;
        }
        Some(normalized)
    }

    fn error(&mut self, code: &'static str, message: impl Into<String>, handle: Option<String>) {
        self.diagnostics.push(WorkflowCodeValidationDiagnostic {
            severity: WorkflowCodeValidationSeverity::Error,
            code: code.to_string(),
            message: message.into(),
            handle,
            source_span: None,
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

pub(crate) fn attach_workflow_code_diagnostic_spans(
    validation: &mut WorkflowCodeValidationReport,
    source_spans: &BTreeMap<String, WorkflowCodeSourceSpan>,
) {
    for diagnostic in &mut validation.diagnostics {
        if diagnostic.source_span.is_some() {
            continue;
        }
        let Some(handle) = diagnostic.handle.as_deref() else {
            continue;
        };
        if let Some(source_span) = source_spans.get(handle) {
            diagnostic.source_span = Some(source_span.clone());
        }
    }
}

impl WorkflowCodeArtifact {
    pub fn into_package(self) -> WorkflowCodeArtifactPackage {
        let definition_sha256 = workflow_code_definition_sha256_hex(&self.definition);
        WorkflowCodeArtifactPackage {
            package_version: WORKFLOW_CODE_ARTIFACT_PACKAGE_VERSION,
            name: self.metadata.name,
            language: self.metadata.language,
            source: self.source,
            source_sha256: self.metadata.source_sha256,
            source_bytes: self.metadata.source_bytes,
            definition_sha256,
            definition: self.definition,
            validation: self.metadata.validation,
            exported_at_ms: crate::session::unix_epoch_ms(),
        }
    }
}

impl WorkflowCodeArtifactPackage {
    pub fn validate_integrity(&self) -> Result<(), crate::DaemonError> {
        if self.package_version != WORKFLOW_CODE_ARTIFACT_PACKAGE_VERSION {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.import",
                message: format!(
                    "unsupported workflow-code package version {}; expected {}",
                    self.package_version, WORKFLOW_CODE_ARTIFACT_PACKAGE_VERSION
                ),
            });
        }
        validate_registry_name(&self.name, "workflow-code artifact package name")?;
        let source_bytes = self.source.len() as u64;
        if source_bytes != self.source_bytes {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.import",
                message: format!(
                    "workflow-code package source byte count mismatch: declared {}, actual {source_bytes}",
                    self.source_bytes
                ),
            });
        }
        let source_sha256 = sha256_hex(self.source.as_bytes());
        if source_sha256 != self.source_sha256 {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.import",
                message: "workflow-code package source sha256 mismatch".to_string(),
            });
        }
        let definition_sha256 = workflow_code_definition_sha256_hex(&self.definition);
        if definition_sha256 != self.definition_sha256 {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.import",
                message: "workflow-code package definition sha256 mismatch".to_string(),
            });
        }
        Ok(())
    }
}

pub fn apply_workflow_code_provider_rebindings(
    definition: &mut WorkflowCodeDefinition,
    rebindings: &[WorkflowCodeProviderRebinding],
) -> Result<(), crate::DaemonError> {
    if rebindings.is_empty() {
        return Ok(());
    }
    let mut seen = BTreeSet::new();
    for rebinding in rebindings {
        let node_handle = rebinding.node.trim();
        if node_handle.is_empty() {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.rebind",
                message: "provider rebinding node handle must not be empty".to_string(),
            });
        }
        if !seen.insert(node_handle.to_string()) {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.rebind",
                message: format!("duplicate provider rebinding for node `{node_handle}`"),
            });
        }
        if rebinding.provider.trim().is_empty() {
            return Err(crate::DaemonError::LocalTransport {
                operation: "workflow_code.rebind",
                message: format!(
                    "provider rebinding for node `{node_handle}` must include provider"
                ),
            });
        }
        let node = definition
            .nodes
            .iter_mut()
            .find(|node| node.handle == node_handle)
            .ok_or_else(|| crate::DaemonError::LocalTransport {
                operation: "workflow_code.rebind",
                message: format!("provider rebinding references unknown node `{node_handle}`"),
            })?;
        match &mut node.agent {
            WorkflowCodeAgentBinding::Create(agent) => {
                agent.provider = rebinding.provider.trim().to_string();
                if let Some(model) = rebinding.model.as_deref() {
                    let model = model.trim();
                    agent.model = if model.is_empty() {
                        None
                    } else {
                        Some(model.to_string())
                    };
                }
                if let Some(effort) = rebinding.effort.as_deref() {
                    let effort = effort.trim();
                    agent.effort = if effort.is_empty() {
                        None
                    } else {
                        Some(effort.to_string())
                    };
                }
                if let Some(account_profile) = rebinding.account_profile.as_deref() {
                    let account_profile = account_profile.trim();
                    agent.account_profile =
                        if account_profile.is_empty() || account_profile == "default" {
                            None
                        } else {
                            Some(account_profile.to_string())
                        };
                }
            }
            WorkflowCodeAgentBinding::Existing(_) => {
                return Err(crate::DaemonError::LocalTransport {
                    operation: "workflow_code.rebind",
                    message: format!(
                        "provider rebinding for node `{node_handle}` targets an existing-agent binding"
                    ),
                });
            }
        }
    }
    Ok(())
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
                provenance: self.provenance,
                history: self.history,
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

fn workflow_code_artifact_history_entry(
    action: WorkflowCodeArtifactHistoryAction,
    at_ms: u64,
    actor: WorkflowCodeArtifactActor,
    source_sha256: String,
    validation_ok: Option<bool>,
    workflow_id: Option<String>,
    warnings: Vec<WorkflowCodeApplyWarning>,
) -> WorkflowCodeArtifactHistoryEntry {
    WorkflowCodeArtifactHistoryEntry {
        action,
        at_ms,
        actor,
        source_sha256,
        validation_ok,
        workflow_id,
        warnings,
    }
}

fn workflow_code_artifact_history(
    mut history: Vec<WorkflowCodeArtifactHistoryEntry>,
) -> Vec<WorkflowCodeArtifactHistoryEntry> {
    if history.len() > WORKFLOW_CODE_ARTIFACT_HISTORY_LIMIT {
        history.drain(0..history.len() - WORKFLOW_CODE_ARTIFACT_HISTORY_LIMIT);
    }
    history
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

pub fn workflow_code_definition_sha256_hex(definition: &WorkflowCodeDefinition) -> String {
    let bytes = serde_json::to_vec(definition).unwrap_or_default();
    sha256_hex(&bytes)
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

fn workflow_code_generated_prompt_bytes(definition: &WorkflowCodeDefinition) -> usize {
    fn add_string(total: &mut usize, value: Option<&str>) {
        if let Some(value) = value {
            *total = total.saturating_add(value.len());
        }
    }

    let mut total = 0usize;
    add_string(&mut total, definition.workflow.alias.as_deref());
    add_string(&mut total, definition.workflow.prompt.as_deref());
    for schema in &definition.schemas {
        add_string(&mut total, schema.alias.as_deref());
        add_string(&mut total, schema.description.as_deref());
    }
    for node in &definition.nodes {
        add_string(&mut total, node.public_label.as_deref());
        add_string(&mut total, node.instructions.as_deref());
        add_string(&mut total, node.intermediate_output_schema.as_deref());
        match &node.agent {
            WorkflowCodeAgentBinding::Create(agent) => {
                add_string(&mut total, agent.alias.as_deref());
                add_string(&mut total, Some(&agent.provider));
                add_string(&mut total, agent.model.as_deref());
                add_string(&mut total, agent.effort.as_deref());
                add_string(&mut total, agent.account_profile.as_deref());
            }
            WorkflowCodeAgentBinding::Existing(agent) => {
                add_string(&mut total, Some(&agent.agent_ref));
            }
        }
    }
    for edge in &definition.edges {
        add_string(&mut total, edge.handoff_schema.as_deref());
    }
    for endpoint in &definition.endpoints {
        add_string(&mut total, endpoint.alias.as_deref());
    }
    for queue in &definition.queues {
        add_string(&mut total, Some(&queue.alias));
    }
    for watchdog in &definition.watchdogs {
        add_string(&mut total, Some(&watchdog.invocation_prompt));
    }
    total
}

fn workflow_code_materialized_queue_count(definition: &WorkflowCodeDefinition) -> usize {
    1 + definition
        .queues
        .iter()
        .filter(|queue| queue.alias.trim().to_lowercase() != "default")
        .count()
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
                prompt: Some("Run the toy workflow.".to_string()),
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
    fn rejects_invalid_aliases_and_duplicate_queue_aliases() {
        let mut definition = minimal_definition();
        definition.workflow.alias = Some("bad alias".to_string());
        definition.endpoints[0].alias = Some("bad/endpoint".to_string());
        definition.endpoints.push(WorkflowCodeEndpointDefinition {
            handle: "duplicate_endpoint".to_string(),
            entry_node: "planner".to_string(),
            alias: Some("ENTRY".to_string()),
            canvas: None,
        });
        definition.endpoints.push(WorkflowCodeEndpointDefinition {
            handle: "duplicate_endpoint_copy".to_string(),
            entry_node: "planner".to_string(),
            alias: Some("entry".to_string()),
            canvas: None,
        });
        definition.queues = vec![
            WorkflowCodeQueueDefinition {
                handle: "urgent".to_string(),
                alias: "urgent".to_string(),
                priority: 10,
                enabled: true,
            },
            WorkflowCodeQueueDefinition {
                handle: "urgent_copy".to_string(),
                alias: "URGENT".to_string(),
                priority: 5,
                enabled: true,
            },
            WorkflowCodeQueueDefinition {
                handle: "default".to_string(),
                alias: "default".to_string(),
                priority: 0,
                enabled: true,
            },
            WorkflowCodeQueueDefinition {
                handle: "default_copy".to_string(),
                alias: "default".to_string(),
                priority: -1,
                enabled: true,
            },
            WorkflowCodeQueueDefinition {
                handle: "empty".to_string(),
                alias: " ".to_string(),
                priority: 0,
                enabled: true,
            },
        ];

        let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());
        let invalid_alias_count = report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "invalid_alias")
            .count();
        let duplicate_queue_alias_count = report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "duplicate_queue_alias")
            .count();
        let duplicate_endpoint_alias_count = report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "duplicate_endpoint_alias")
            .count();

        assert!(!report.ok);
        assert_eq!(invalid_alias_count, 3, "{:?}", report.diagnostics);
        assert_eq!(duplicate_queue_alias_count, 2, "{:?}", report.diagnostics);
        assert_eq!(
            duplicate_endpoint_alias_count, 1,
            "{:?}",
            report.diagnostics
        );
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
    fn rejects_self_edges_and_duplicate_edge_pairs() {
        let mut definition = minimal_definition();
        definition.nodes.push(WorkflowCodeNodeDefinition {
            handle: "reviewer".to_string(),
            agent: WorkflowCodeAgentBinding::Create(WorkflowCodeAgentCreate {
                alias: Some("Reviewer".to_string()),
                provider: "dev-stub".to_string(),
                model: Some("default".to_string()),
                effort: None,
                account_profile: None,
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
            handle: "planner_self".to_string(),
            from_node: "planner".to_string(),
            to_node: "planner".to_string(),
            source_side: None,
            target_side: None,
            handoff_schema: None,
            validation_policy: None,
            canvas: None,
        });
        definition.edges.push(WorkflowCodeEdgeDefinition {
            handle: "plan_to_review".to_string(),
            from_node: "planner".to_string(),
            to_node: "reviewer".to_string(),
            source_side: None,
            target_side: None,
            handoff_schema: None,
            validation_policy: None,
            canvas: None,
        });
        definition.edges.push(WorkflowCodeEdgeDefinition {
            handle: "plan_to_review_again".to_string(),
            from_node: "planner".to_string(),
            to_node: "reviewer".to_string(),
            source_side: None,
            target_side: None,
            handoff_schema: None,
            validation_policy: None,
            canvas: None,
        });

        let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());
        let codes = report
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>();

        assert!(!report.ok);
        assert!(codes.contains(&"invalid_edge"), "{:?}", report.diagnostics);
        assert!(
            codes.contains(&"duplicate_edge"),
            "{:?}",
            report.diagnostics
        );
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
    fn enforces_materialized_queue_limit_including_default_queue() {
        let mut definition = minimal_definition();
        definition.queues = vec![WorkflowCodeQueueDefinition {
            handle: "urgent".to_string(),
            alias: "urgent".to_string(),
            priority: 5,
            enabled: true,
        }];
        let limits = WorkflowCodeLimitsConfig {
            max_queues: 1,
            ..WorkflowCodeLimitsConfig::default()
        };

        let report = definition.validate_with_limits(&limits);

        assert!(!report.ok);
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "limit_exceeded"
                && diagnostic.message.contains("queues count 2 exceeds")
        }));
    }

    #[test]
    fn enforces_generated_prompt_byte_limit() {
        let mut definition = minimal_definition();
        definition.nodes[0].instructions = Some("x".repeat(128));
        let limits = WorkflowCodeLimitsConfig {
            max_generated_prompt_bytes: 64,
            ..WorkflowCodeLimitsConfig::default()
        };

        let report = definition.validate_with_limits(&limits);

        assert!(!report.ok);
        assert!(report.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "limit_exceeded"
                && diagnostic
                    .message
                    .contains("workflow generated prompt text uses")
        }));
    }

    #[test]
    fn validates_node_extension_grant_shape() {
        let mut definition = minimal_definition();
        definition.nodes[0]
            .extensions
            .push(ExtensionGrant::new(ExtensionKind::Skill, ""));
        definition.nodes[0]
            .extensions
            .push(ExtensionGrant::new(ExtensionKind::Script, "release-script"));
        definition.nodes[0].extensions.push(ExtensionGrant {
            kind: ExtensionKind::Connector,
            name: "deploy-api".to_string(),
            environment: None,
            credential: None,
            max_safety: Some("admin".to_string()),
        });

        let report = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());
        let codes = report
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>();

        assert!(!report.ok);
        assert!(codes.contains(&"invalid_extension_name"));
        assert!(codes.contains(&"invalid_extension_environment"));
        assert!(codes.contains(&"invalid_connector_safety"));
    }

    #[test]
    fn provider_rebindings_normalize_optional_fields() {
        let mut definition = minimal_definition();

        apply_workflow_code_provider_rebindings(
            &mut definition,
            &[WorkflowCodeProviderRebinding {
                node: " planner ".to_string(),
                provider: " dev-stub ".to_string(),
                model: Some(" ".to_string()),
                effort: Some(" low ".to_string()),
                account_profile: Some(" default ".to_string()),
            }],
        )
        .expect("rebinding should apply");

        match &definition.nodes[0].agent {
            WorkflowCodeAgentBinding::Create(agent) => {
                assert_eq!(agent.provider, "dev-stub");
                assert_eq!(agent.model, None);
                assert_eq!(agent.effort.as_deref(), Some("low"));
                assert_eq!(agent.account_profile, None);
            }
            WorkflowCodeAgentBinding::Existing(_) => panic!("planner should use generated agent"),
        }
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

    fn find_node() -> Option<PathBuf> {
        let mut candidates = Vec::new();
        if let Some(path) = std::env::var_os("NODE") {
            candidates.push(PathBuf::from(path));
        }
        candidates.extend([
            PathBuf::from("/opt/homebrew/bin/node"),
            PathBuf::from("/usr/local/bin/node"),
            PathBuf::from("/usr/bin/node"),
        ]);

        candidates.into_iter().find(|candidate| {
            Command::new(candidate)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        })
    }

    #[test]
    fn compiles_javascript_builder_source() {
        let Some(node) = find_node() else {
            eprintln!("skipping workflow-code JS compiler test because node is not available");
            return;
        };

        let source = r#"
const finalSchema = workflow.schema({
  alias: "Final",
  schema: {
    type: "object",
    properties: { answer: { type: "string" } },
    required: ["answer"],
    additionalProperties: false
  }
})
workflow.define({
  alias: "compiled",
  prompt: "Run the compiled workflow.",
  maxConcurrent: 2,
  runOutputSchema: finalSchema
})
const planner = workflow.node({
  agent: workflow.newAgent({ alias: "planner", provider: "dev-stub", model: "default" }),
  publicLabel: "Planner",
  instructions: "Plan.",
  canCompleteWorkflowRun: true
})
workflow.endpoint(planner, { alias: "entry" })
"#;

        let result =
            compile_workflow_code_javascript(node, source, &WorkflowCodeLimitsConfig::default())
                .expect("workflow-code JS source should compile");

        assert!(result.validation.ok, "{:?}", result.validation.diagnostics);
        assert_eq!(
            result.definition.workflow.alias.as_deref(),
            Some("compiled")
        );
        assert_eq!(
            result.definition.workflow.prompt.as_deref(),
            Some("Run the compiled workflow.")
        );
        assert_eq!(result.definition.nodes.len(), 1);
        assert_eq!(result.definition.endpoints.len(), 1);
        assert_eq!(result.definition.schemas.len(), 1);
    }

    #[test]
    fn compiles_javascript_queues_and_watchdogs() {
        let Some(node) = find_node() else {
            eprintln!("skipping workflow-code JS compiler test because node is not available");
            return;
        };

        let source = r#"
const finalSchema = workflow.schema({
  handle: "final",
  schema: {
    type: "object",
    required: ["answer"],
    properties: { answer: { type: "string" } },
    additionalProperties: false
  }
})
workflow.define({ alias: "queued_watchdog_flow", runOutputSchema: finalSchema })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "worker", provider: "dev-stub", model: "default" }),
  canCompleteWorkflowRun: true
})
const entry = workflow.endpoint(worker, { handle: "entry", alias: "entry" })
const urgent = workflow.queue({ handle: "urgent", alias: "urgent", priority: 5, enabled: false })
workflow.watchdog(entry, {
  handle: "wake_entry",
  queue: urgent,
  enabled: false,
  intervalSeconds: 60,
  invocationPrompt: "Check for queued work.",
  policy: "skip",
  maxWakeups: 2
})
"#;

        let result =
            compile_workflow_code_javascript(node, source, &WorkflowCodeLimitsConfig::default())
                .expect("workflow-code JS source should compile");

        assert!(result.validation.ok, "{:?}", result.validation.diagnostics);
        assert_eq!(result.definition.queues.len(), 1);
        assert_eq!(result.definition.queues[0].handle, "urgent");
        assert_eq!(result.definition.queues[0].priority, 5);
        assert!(!result.definition.queues[0].enabled);
        assert_eq!(result.definition.watchdogs.len(), 1);
        assert_eq!(result.definition.watchdogs[0].endpoint, "entry");
        assert_eq!(
            result.definition.watchdogs[0].queue.as_deref(),
            Some("urgent")
        );
        assert_eq!(
            result.definition.watchdogs[0].policy,
            WorkflowWatchdogPolicy::Skip
        );
        assert_eq!(result.definition.watchdogs[0].enabled, Some(false));
        assert_eq!(result.definition.watchdogs[0].max_wakeups, Some(2));
    }

    #[test]
    fn workflow_code_language_serializes_canonical_typescript_name() {
        assert_eq!(
            serde_json::to_value(WorkflowCodeLanguage::TypeScript)
                .expect("language should serialize"),
            serde_json::json!("typescript")
        );
        assert_eq!(
            serde_json::from_value::<WorkflowCodeLanguage>(serde_json::json!("type_script"))
                .expect("legacy spelling should decode"),
            WorkflowCodeLanguage::TypeScript
        );
    }

    #[test]
    fn compiles_typescript_builder_source() {
        let Some(node) = find_node() else {
            eprintln!("skipping workflow-code TS compiler test because node is not available");
            return;
        };
        if !Command::new(&node)
            .arg("--no-warnings")
            .arg("--input-type=module")
            .arg("-e")
            .arg("const mod = await import('node:module'); if (typeof mod.stripTypeScriptTypes !== 'function') process.exit(1)")
            .status()
            .is_ok_and(|status| status.success())
        {
            eprintln!("skipping workflow-code TS compiler test because Node.js cannot strip TypeScript");
            return;
        }

        let source = r#"
type ProviderName = "dev-stub";
interface FinalAnswer {
  answer: string;
}
const provider: ProviderName = "dev-stub";
const finalSchema = workflow.schema({
  handle: "final",
  schema: {
    type: "object",
    required: ["answer"],
    properties: { answer: { type: "string" } },
    additionalProperties: false
  }
})
workflow.define({ alias: "compiled_ts", maxConcurrent: 2, runOutputSchema: finalSchema })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ alias: "ts-worker", provider, model: "default" }),
  instructions: "Return a FinalAnswer.",
  canCompleteWorkflowRun: true
})
workflow.endpoint(worker, { handle: "entry", alias: "entry" })
"#;

        let result = compile_workflow_code_source_with_schema_import_root(
            node,
            source,
            WorkflowCodeLanguage::TypeScript,
            &WorkflowCodeLimitsConfig::default(),
            None,
        )
        .expect("workflow-code TS source should compile");

        assert!(result.validation.ok, "{:?}", result.validation.diagnostics);
        assert_eq!(
            result.definition.workflow.alias.as_deref(),
            Some("compiled_ts")
        );
        assert_eq!(
            result.definition.workflow.run_output_schema.as_deref(),
            Some("final")
        );
        assert_eq!(
            result.definition.nodes[0].agent,
            WorkflowCodeAgentBinding::Create(WorkflowCodeAgentCreate {
                alias: Some("ts-worker".to_string()),
                provider: "dev-stub".to_string(),
                model: Some("default".to_string()),
                effort: None,
                account_profile: None,
            })
        );
    }

    #[test]
    fn canonical_pattern_examples_compile() {
        let Some(node) = find_node() else {
            eprintln!("skipping workflow-code pattern examples because node is not available");
            return;
        };

        for example in WORKFLOW_CODE_PATTERN_EXAMPLES {
            let result = compile_workflow_code_javascript(
                &node,
                example.source,
                &WorkflowCodeLimitsConfig::default(),
            )
            .unwrap_or_else(|error| {
                panic!(
                    "workflow-code pattern example `{}` at `{}` should compile: {error}",
                    example.slug, example.path
                )
            });

            assert!(
                result.validation.ok,
                "workflow-code pattern example `{}` should validate: {:?}",
                example.slug, result.validation.diagnostics
            );
            assert!(
                result.definition.workflow.alias.is_some(),
                "workflow-code pattern example `{}` should name the workflow",
                example.slug
            );
            assert!(
                result.definition.workflow.run_output_schema.is_some(),
                "workflow-code pattern example `{}` should define final output schema",
                example.slug
            );
            assert!(
                !result.definition.schemas.is_empty(),
                "workflow-code pattern example `{}` should define schemas",
                example.slug
            );
            assert!(
                !result.definition.nodes.is_empty(),
                "workflow-code pattern example `{}` should define nodes",
                example.slug
            );
            assert!(
                !result.definition.endpoints.is_empty(),
                "workflow-code pattern example `{}` should define endpoints",
                example.slug
            );
        }
    }

    #[test]
    fn javascript_compiler_attaches_source_spans_to_validation_diagnostics() {
        let Some(node) = find_node() else {
            eprintln!("skipping workflow-code JS compiler test because node is not available");
            return;
        };

        let result = compile_workflow_code_javascript(
            node,
            r#"
const final = workflow.schema({
  handle: "final",
  schema: { type: "object", additionalProperties: false }
})
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ provider: "dev-stub" }),
  canCompleteWorkflowRun: true,
  maxTurns: 0
})
workflow.endpoint(worker, { handle: "entry" })
workflow.define({ alias: "bad", runOutputSchema: final })
"#,
            &WorkflowCodeLimitsConfig::default(),
        )
        .expect("workflow-code script should compile");

        let diagnostic = result
            .validation
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "invalid_max_turns")
            .expect("invalid max_turns diagnostic should exist");
        assert_eq!(diagnostic.handle.as_deref(), Some("worker"));
        let source_span = diagnostic
            .source_span
            .as_ref()
            .expect("diagnostic should carry a source span");
        assert!(source_span.start_line >= 1);
        assert!(source_span.start_column >= 1);
        assert_eq!(source_span.end_line, source_span.start_line);
        assert_eq!(source_span.end_column, source_span.start_column);
    }

    #[test]
    fn javascript_compiler_reports_script_errors() {
        let Some(node) = find_node() else {
            eprintln!("skipping workflow-code JS compiler test because node is not available");
            return;
        };

        let error = compile_workflow_code_javascript(
            node,
            r#"throw new Error("boom")"#,
            &WorkflowCodeLimitsConfig::default(),
        )
        .expect_err("script error should be returned");

        assert!(format!("{error}").contains("boom"));
    }

    #[test]
    fn javascript_compiler_embeds_schema_from_approved_file() {
        let Some(node) = find_node() else {
            eprintln!("skipping workflow-code JS compiler test because node is not available");
            return;
        };
        let root = std::env::temp_dir().join(format!(
            "arroba-workflow-code-schema-import-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        fs::create_dir_all(root.join("schemas")).expect("schema directory should create");
        fs::write(
            root.join("schemas/final.json"),
            r#"{"type":"object","required":["answer"],"properties":{"answer":{"type":"string"}},"additionalProperties":false}"#,
        )
        .expect("schema file should write");

        let result = compile_workflow_code_javascript_with_schema_import_root(
            node,
            r#"
workflow.define({ alias: "imported_schema" })
const final = workflow.schemaFromFile({
  handle: "final",
  path: "schemas/final.json",
  alias: "Final output"
})
workflow.define({ runOutputSchema: final })
const worker = workflow.node({
  handle: "worker",
  agent: workflow.newAgent({ provider: "dev-stub" }),
  canCompleteWorkflowRun: true
})
workflow.endpoint(worker, { handle: "entry" })
"#,
            &WorkflowCodeLimitsConfig::default(),
            Some(&root),
        )
        .expect("workflow-code schema import should compile");

        assert!(result.validation.ok, "{:?}", result.validation.diagnostics);
        assert_eq!(result.definition.schemas.len(), 1);
        assert_eq!(result.definition.schemas[0].handle, "final");
        assert_eq!(
            result.definition.schemas[0].schema["properties"]["answer"]["type"],
            "string"
        );
        assert_eq!(
            result.definition.workflow.run_output_schema.as_deref(),
            Some("final")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn javascript_compiler_rejects_schema_file_escape() {
        let Some(node) = find_node() else {
            eprintln!("skipping workflow-code JS compiler test because node is not available");
            return;
        };
        let root = std::env::temp_dir().join(format!(
            "arroba-workflow-code-schema-escape-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        fs::create_dir_all(&root).expect("schema root should create");

        let error = compile_workflow_code_javascript_with_schema_import_root(
            node,
            r#"
workflow.schemaFromFile({ handle: "final", path: "../outside.json" })
"#,
            &WorkflowCodeLimitsConfig::default(),
            Some(&root),
        )
        .expect_err("schema import should reject parent traversal");

        assert!(format!("{error}").contains("approved import root"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn javascript_compiler_rejects_schema_file_without_json_extension() {
        let Some(node) = find_node() else {
            eprintln!("skipping workflow-code JS compiler test because node is not available");
            return;
        };
        let root = std::env::temp_dir().join(format!(
            "arroba-workflow-code-schema-extension-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        fs::create_dir_all(root.join("schemas")).expect("schema root should create");
        fs::write(
            root.join("schemas/final.txt"),
            r#"{"type":"object","additionalProperties":false}"#,
        )
        .expect("schema fixture should write");

        let error = compile_workflow_code_javascript_with_schema_import_root(
            node,
            r#"
workflow.schemaFromFile({ handle: "final", path: "schemas/final.txt" })
"#,
            &WorkflowCodeLimitsConfig::default(),
            Some(&root),
        )
        .expect_err("schema import should reject non-json files");

        assert!(format!("{error}").contains("must end in .json"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn registry_saves_lists_reads_updates_and_deletes_artifacts() {
        let root = std::env::temp_dir().join(format!(
            "arroba-workflow-code-registry-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let registry = WorkflowCodeArtifactRegistry::new(vec![root.clone()]);
        let definition = minimal_definition();
        let validation = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());
        let creator = WorkflowCodeArtifactActor::new("user-1", None);
        let updater = WorkflowCodeArtifactActor::new("user-2", Some("meta-1".to_string()));

        let created = registry
            .save(
                "toy",
                WorkflowCodeLanguage::JavaScript,
                "workflow.define({ alias: 'toy' })",
                definition.clone(),
                validation,
                creator.clone(),
                WorkflowCodeArtifactHistoryAction::Created,
            )
            .expect("workflow-code artifact should save");

        assert_eq!(created.metadata.name, "toy");
        assert_eq!(created.metadata.language, WorkflowCodeLanguage::JavaScript);
        assert_eq!(created.metadata.source_bytes, 33);
        assert!(created.metadata.validation.ok);
        assert_eq!(created.metadata.provenance.created_by, creator);
        assert_eq!(created.metadata.provenance.updated_by, creator);
        assert_eq!(created.metadata.history.len(), 1);
        assert_eq!(
            created.metadata.history[0].action,
            WorkflowCodeArtifactHistoryAction::Created
        );
        assert_eq!(created.metadata.history[0].validation_ok, Some(true));
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
                minimal_definition().validate_with_limits(&WorkflowCodeLimitsConfig::default()),
                updater.clone(),
                WorkflowCodeArtifactHistoryAction::Updated,
            )
            .expect("workflow-code artifact should update");
        assert_eq!(updated.metadata.language, WorkflowCodeLanguage::TypeScript);
        assert_eq!(
            updated.metadata.created_at_ms,
            created.metadata.created_at_ms
        );
        assert!(updated.metadata.updated_at_ms >= created.metadata.updated_at_ms);
        assert_eq!(updated.metadata.provenance.created_by, creator);
        assert_eq!(updated.metadata.provenance.updated_by, updater);
        assert_eq!(updated.metadata.history.len(), 2);
        assert_eq!(
            updated.metadata.history[1].action,
            WorkflowCodeArtifactHistoryAction::Updated
        );

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
        let validation = definition.validate_with_limits(&WorkflowCodeLimitsConfig::default());
        let actor = WorkflowCodeArtifactActor::new("user-1", None);

        let artifact = registry
            .save(
                "invalid",
                WorkflowCodeLanguage::JavaScript,
                "workflow.define({ alias: 'invalid' })",
                definition,
                validation,
                actor,
                WorkflowCodeArtifactHistoryAction::Created,
            )
            .expect("invalid workflow-code artifact should still save diagnostics");

        assert!(!artifact.metadata.validation.ok);
        assert!(artifact
            .metadata
            .validation
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "missing_endpoint"));
        assert_eq!(artifact.metadata.history[0].validation_ok, Some(false));

        let _ = fs::remove_dir_all(root);
    }
}
