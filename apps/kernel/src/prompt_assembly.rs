use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::DaemonError;
use crate::provider::RuntimeProviderRun;
use crate::session::PromptAttachment;

const PROMPT_REGISTRY_VERSION: &str = "2";
const PROMPT_DEFAULTS_STATE_FILE: &str = ".bundled-defaults.json";
const LEGACY_WORKFLOW_TURN_HASHES: &[&str] = &[
    "ac2ffb8b8e5542bfbeda71eb61a89938215dbf2d5b2027545256d81f19c3b87e",
    "c5867472e1017d69c9426244ea9da5f5a1b03e86054fc3c6416442e55f518e4b",
];
const LEGACY_METAAGENT_DELEGATION_HASHES: &[&str] =
    &["4182ea00a5ca086d4edcaa32900e3586fdc9feef6e954559b5ace7743698816e"];
const LEGACY_META_MODE_ENTERED_HASHES: &[&str] = &[
    "62d1df699e55d3e4213dcb7cdf3eadee471155238c48d3898985e7e264dcea5e",
    "6aac2d837c2d629d1de3e5c96611dd877a9f5a54700a16e24469f53e052ddaa6",
];
const LEGACY_SLICE_HASHES: &[&str] = &[
    "ba79f023be2bcb85d9ab22ceebe992ada13b1fc2e5c3bbfe5b8aef60237ff412",
    "5b2723aede2fc23f4de963cc9aea35c7a0af4c1677e7ee5b64e0d74276be0a22",
    "fd8f931980e7a9645e79d24517b01e74d915e3592d366c4d003e0eba476a7ca6",
    "a9115e43cc0332fafcecdc29105b6151af4ed20bc584dc9f48ba00a267776b71",
    "e92428d4e8c387de479477a3f33b873b0e81863cd499c224f44661d2efabb28f",
];

const RUNTIME_BASE: &str = include_str!("provider/runtime_instructions.md");
const RUNTIME_WORKSPACE_LIVE_SYNC: &str =
    include_str!("provider/workspace_live_sync_instructions.md");
const RUNTIME_WORKSPACE_LIVE_SYNC_TRACKED: &str =
    include_str!("provider/workspace_live_sync_tracked_instructions.md");
const RUNTIME_NATIVE_PERMISSIONS: &str = include_str!("provider/native_permission_instructions.md");
const RUNTIME_SLICE: &str = include_str!("provider/slice_runtime_instructions.md");
const RUNTIME_METAAGENT_DELEGATION: &str =
    include_str!("provider/metaagent_delegation_instructions.md");
const RUNTIME_META_MODE_ENTERED: &str = include_str!("provider/meta_mode_entered_context.md");
const RUNTIME_MCP_SKILL_CONTINUATION: &str =
    include_str!("provider/mcp_skill_continuation_instructions.md");
const RUNTIME_WORKFLOW_DIRECT_JSON_FALLBACK: &str =
    include_str!("provider/workflow_direct_json_fallback_instructions.md");
const RUNTIME_METAAGENT_EVENT: &str = include_str!("provider/metaagent_event_instructions.md");
const WORKFLOW_TURN: &str = include_str!("provider/workflow_turn_instructions.md");
const WORKFLOW_RUN_COMPLETION: &str =
    include_str!("provider/workflow_run_completion_instructions.md");
const WORKFLOW_RUN_INTERMEDIATE_OUTPUT: &str =
    include_str!("provider/workflow_run_intermediate_output_instructions.md");
const WORKFLOW_RUN_OUTPUT_CORRECTION: &str =
    include_str!("provider/workflow_run_output_correction_instructions.md");
const WORKFLOW_HANDOFF_CORRECTION: &str =
    include_str!("provider/workflow_handoff_correction_instructions.md");
const WORKFLOW_MISSING_OUTPUT_CORRECTION: &str =
    include_str!("provider/workflow_missing_output_correction_instructions.md");
const UTILITY_WORKSPACE_COMMIT_MESSAGE: &str =
    include_str!("provider/workspace_commit_message_instructions.md");
const UTILITY_SEMANTIC_RECALL_SEARCH: &str =
    include_str!("provider/semantic_recall_search_instructions.md");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PromptEnvelope {
    pub(crate) visible_user_prompt: String,
    pub(crate) hidden_system_context: String,
    pub(crate) attachments: Vec<PromptAttachment>,
    pub(crate) manifest: PromptManifest,
    pub(crate) steering: bool,
}

impl PromptEnvelope {
    pub(crate) fn new(
        visible_user_prompt: impl Into<String>,
        hidden_system_context: impl Into<String>,
        attachments: Vec<PromptAttachment>,
        manifest: PromptManifest,
    ) -> Self {
        Self {
            visible_user_prompt: visible_user_prompt.into(),
            hidden_system_context: hidden_system_context.into(),
            attachments,
            manifest,
            steering: false,
        }
    }

    pub(crate) fn with_steering(mut self, steering: bool) -> Self {
        self.steering = steering;
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PromptManifest {
    pub(crate) version: String,
    pub(crate) entries: Vec<PromptManifestEntry>,
}

impl PromptManifest {
    pub(crate) fn current() -> Self {
        Self {
            version: PROMPT_REGISTRY_VERSION.to_string(),
            entries: Vec::new(),
        }
    }

    pub(crate) fn push_body(&mut self, template_id: impl Into<String>, body: &str) {
        self.entries.push(PromptManifestEntry {
            template_id: template_id.into(),
            sha256: sha256_hex(body),
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PromptManifestEntry {
    pub(crate) template_id: String,
    pub(crate) sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptAssemblyMode {
    NormalProviderTurn,
    NativeTuiProviderTurn,
    MetaagentProviderTurn,
    WorkflowNodeTurn,
    UtilityTurn,
    McpSkillContinuationTurn,
}

pub(crate) fn provider_turn_mode_for_prompt(
    agent_id: &str,
    agent_is_metaagent: bool,
    source_client_id: Option<&str>,
    hidden_system_context: &str,
) -> PromptAssemblyMode {
    let metaagent_client_prefix = format!("metaagent:{agent_id}:");
    let is_metaagent_control_turn = source_client_id
        .is_some_and(|client_id| client_id.starts_with(&metaagent_client_prefix))
        || hidden_system_context.contains("<meta-mode-entered-context>");
    if agent_is_metaagent && is_metaagent_control_turn {
        PromptAssemblyMode::MetaagentProviderTurn
    } else {
        PromptAssemblyMode::NormalProviderTurn
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PromptTemplateRegistry {
    root: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct BundledPromptDefaultsState {
    version: String,
    template_sha256: BTreeMap<String, String>,
}

impl PromptTemplateRegistry {
    pub(crate) fn from_env() -> Self {
        let chariox_home = std::env::var_os("CHARIOX_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|value| !value.is_empty())
                    .map(|home| PathBuf::from(home).join(".chariox"))
            })
            .unwrap_or_else(|| std::env::temp_dir().join("chariox"));
        Self::new(chariox_home.join("prompts"))
    }

    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn list_settings(&self) -> Result<Vec<PromptSettingRecord>, DaemonError> {
        self.materialize_bundled_defaults()?;
        bundled_templates()
            .into_iter()
            .map(|template| self.read_setting(template.id))
            .collect()
    }

    pub(crate) fn read_setting(
        &self,
        template_id: &str,
    ) -> Result<PromptSettingRecord, DaemonError> {
        let template = bundled_templates()
            .into_iter()
            .find(|template| template.id == template_id)
            .ok_or_else(|| prompt_settings_error("unknown prompt setting", template_id))?;
        self.materialize_bundled_defaults()?;
        let path = self.path_for(template.id);
        let current = fs::read_to_string(&path)
            .map_err(|error| prompt_io_error("read", &path, error))?
            .trim()
            .to_string();
        let default = template.body.trim().to_string();
        let metadata = prompt_setting_metadata(template.id);
        Ok(PromptSettingRecord {
            id: template.id.to_string(),
            title: metadata.title.to_string(),
            scope: metadata.scope.to_string(),
            audience: metadata.audience.to_string(),
            provider_applicability: metadata
                .provider_applicability
                .iter()
                .map(|provider| (*provider).to_string())
                .collect(),
            source: if sha256_hex(&current) == sha256_hex(&default) {
                "bundled".to_string()
            } else {
                "user_override".to_string()
            },
            current_sha256: sha256_hex(&current),
            default_sha256: sha256_hex(&default),
            current_bytes: current.len(),
            default_bytes: default.len(),
            revision: prompt_revision(&current),
            variables: prompt_variables(&current),
            current,
            default,
            editable: metadata.editable,
            protected: metadata.protected,
        })
    }

    pub(crate) fn update_setting(
        &self,
        template_id: &str,
        body: &str,
    ) -> Result<PromptSettingRecord, DaemonError> {
        let current = self.read_setting(template_id)?;
        if current.protected {
            return Err(prompt_settings_error(
                "protected prompt setting cannot be edited",
                template_id,
            ));
        }
        validate_prompt_markdown(template_id, &current.default, body)?;
        let path = self.path_for(template_id);
        let _guard = prompt_registry_write_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        atomic_write(&path, body.trim())?;
        drop(_guard);
        self.read_setting(template_id)
    }

    pub(crate) fn reset_setting(
        &self,
        template_id: &str,
    ) -> Result<PromptSettingRecord, DaemonError> {
        let template = bundled_templates()
            .into_iter()
            .find(|template| template.id == template_id)
            .ok_or_else(|| prompt_settings_error("unknown prompt setting", template_id))?;
        self.materialize_bundled_defaults()?;
        let path = self.path_for(template_id);
        let _guard = prompt_registry_write_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        atomic_write(&path, template.body.trim())?;
        drop(_guard);
        self.read_setting(template_id)
    }

    pub(crate) fn reset_all_settings(&self) -> Result<Vec<PromptSettingRecord>, DaemonError> {
        self.materialize_bundled_defaults()?;
        let _guard = prompt_registry_write_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for template in bundled_templates() {
            let path = self.path_for(template.id);
            atomic_write(&path, template.body.trim())?;
        }
        drop(_guard);
        self.list_settings()
    }

    pub(crate) fn materialize_bundled_defaults(&self) -> Result<(), DaemonError> {
        let _guard = prompt_registry_write_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state_path = self.root.join(PROMPT_DEFAULTS_STATE_FILE);
        let previous_state = fs::read_to_string(&state_path)
            .ok()
            .and_then(|body| serde_json::from_str::<BundledPromptDefaultsState>(&body).ok());
        let mut template_sha256 = BTreeMap::new();
        for template in bundled_templates() {
            let path = self.path_for(template.id);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| prompt_io_error("create", &path, error))?;
            }
            let bundled_body = template.body.trim_end();
            let bundled_hash = sha256_hex(bundled_body);
            template_sha256.insert(template.id.to_string(), bundled_hash);
            let existing_body = fs::read_to_string(&path).ok();
            let existing_hash = existing_body
                .as_deref()
                .map(|body| sha256_hex(body.trim_end()));
            let previous_bundled_hash = previous_state
                .as_ref()
                .and_then(|state| state.template_sha256.get(template.id));
            let is_known_legacy_default = existing_hash
                .as_deref()
                .is_some_and(|hash| known_legacy_bundled_default(template.id, hash));
            let should_materialize = existing_body.is_none()
                || previous_bundled_hash
                    .zip(existing_hash.as_ref())
                    .is_some_and(|(previous, existing)| previous == existing)
                || is_known_legacy_default;
            if should_materialize && existing_body.as_deref() != Some(bundled_body) {
                atomic_write(&path, bundled_body)?;
            }
        }
        fs::create_dir_all(&self.root)
            .map_err(|error| prompt_io_error("create", &self.root, error))?;
        let state = BundledPromptDefaultsState {
            version: PROMPT_REGISTRY_VERSION.to_string(),
            template_sha256,
        };
        let state_body = serde_json::to_string_pretty(&state).map_err(|error| {
            DaemonError::ProviderProtocol {
                provider_run_id: "prompt-assembly".to_string(),
                operation: "prompt_defaults_state_serialize",
                message: error.to_string(),
            }
        })?;
        atomic_write(&state_path, &state_body)?;
        Ok(())
    }

    pub(crate) fn read_required(&self, template_id: &str) -> Result<PromptTemplate, DaemonError> {
        let path = self.path_for(template_id);
        if !path.exists() {
            return Err(DaemonError::ProviderProtocol {
                provider_run_id: "prompt-assembly".to_string(),
                operation: "prompt_template_read",
                message: format!(
                    "required prompt template `{template_id}` missing at {:?}",
                    path
                ),
            });
        }
        let body =
            fs::read_to_string(&path).map_err(|error| prompt_io_error("read", &path, error))?;
        Ok(PromptTemplate {
            id: template_id.to_string(),
            body: body.trim().to_string(),
        })
    }

    fn path_for(&self, template_id: &str) -> PathBuf {
        let mut path = self.root.clone();
        for component in template_id.split('/') {
            path.push(component);
        }
        path.set_extension("md");
        path
    }
}

fn known_legacy_bundled_default(template_id: &str, hash: &str) -> bool {
    match template_id {
        "workflow/turn" => LEGACY_WORKFLOW_TURN_HASHES.contains(&hash),
        "runtime/metaagent-delegation" => LEGACY_METAAGENT_DELEGATION_HASHES.contains(&hash),
        "runtime/meta-mode-entered" => LEGACY_META_MODE_ENTERED_HASHES.contains(&hash),
        "runtime/slice" => LEGACY_SLICE_HASHES.contains(&hash),
        _ => false,
    }
}

fn prompt_registry_write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn atomic_write(path: &Path, body: &str) -> Result<(), DaemonError> {
    let Some(parent) = path.parent() else {
        return Err(prompt_settings_error(
            "prompt path has no parent",
            &path.display().to_string(),
        ));
    };
    fs::create_dir_all(parent).map_err(|error| prompt_io_error("create", parent, error))?;
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    let temp_path = parent.join(format!(
        ".{}.tmp-{}-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("prompt"),
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed),
    ));
    let result = (|| {
        let mut file = fs::File::create(&temp_path)
            .map_err(|error| prompt_io_error("write", &temp_path, error))?;
        file.write_all(body.as_bytes())
            .map_err(|error| prompt_io_error("write", &temp_path, error))?;
        file.sync_all()
            .map_err(|error| prompt_io_error("sync", &temp_path, error))?;
        fs::rename(&temp_path, path).map_err(|error| prompt_io_error("rename", path, error))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PromptTemplate {
    pub(crate) id: String,
    pub(crate) body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptSettingRecord {
    pub id: String,
    pub title: String,
    pub scope: String,
    pub audience: String,
    pub provider_applicability: Vec<String>,
    pub source: String,
    pub current: String,
    pub default: String,
    pub current_sha256: String,
    pub default_sha256: String,
    pub current_bytes: usize,
    pub default_bytes: usize,
    pub revision: u64,
    pub variables: Vec<String>,
    pub editable: bool,
    pub protected: bool,
}

#[derive(Debug, Clone, Copy)]
struct PromptSettingMetadata {
    title: &'static str,
    scope: &'static str,
    audience: &'static str,
    provider_applicability: &'static [&'static str],
    editable: bool,
    protected: bool,
}

fn prompt_setting_metadata(template_id: &str) -> PromptSettingMetadata {
    let (title, scope, audience) = match template_id {
        "workflow/turn" => ("Workflow turn contract", "workflow", "workflow-agent"),
        "workflow/run-completion" => ("Workflow completion", "workflow", "workflow-agent"),
        "workflow/run-intermediate-output" => {
            ("Workflow progress output", "workflow", "workflow-agent")
        }
        "workflow/run-output-correction" => {
            ("Workflow output correction", "workflow", "workflow-agent")
        }
        "workflow/handoff-correction" => {
            ("Workflow handoff correction", "workflow", "workflow-agent")
        }
        "workflow/missing-output-correction" => (
            "Workflow missing-output correction",
            "workflow",
            "workflow-agent",
        ),
        id if id.starts_with("runtime/metaagent") || id == "runtime/meta-mode-entered" => {
            ("Meta-agent runtime guidance", "runtime", "meta-agent")
        }
        id if id.starts_with("utility/") => ("Utility prompt", "utility", "utility-agent"),
        id if id.starts_with("runtime/") => {
            ("Runtime provider guidance", "runtime", "provider-agent")
        }
        _ => ("Chariox prompt", "runtime", "provider-agent"),
    };
    let protected = matches!(
        template_id,
        "runtime/base"
            | "runtime/native-permissions"
            | "runtime/workspace-live-sync"
            | "runtime/workspace-live-sync-tracked"
            | "runtime/slice"
            | "runtime/metaagent-delegation"
    );
    PromptSettingMetadata {
        title,
        scope,
        audience,
        provider_applicability: &["codex", "claude", "opencode"],
        editable: !protected,
        protected,
    }
}

fn prompt_variables(body: &str) -> Vec<String> {
    let mut variables = BTreeMap::<String, ()>::new();
    let mut remainder = body;
    while let Some(start) = remainder.find("{{") {
        let after_start = &remainder[start + 2..];
        let Some(end) = after_start.find("}}") else {
            break;
        };
        let variable = after_start[..end].trim();
        if !variable.is_empty() {
            variables.insert(variable.to_string(), ());
        }
        remainder = &after_start[end + 2..];
    }
    variables.into_keys().collect()
}

fn prompt_revision(body: &str) -> u64 {
    let digest = Sha256::digest(body.as_bytes());
    u64::from_be_bytes(digest[..8].try_into().unwrap_or_default())
}

fn validate_prompt_markdown(
    template_id: &str,
    bundled_default: &str,
    body: &str,
) -> Result<(), DaemonError> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(prompt_settings_error(
            "prompt Markdown cannot be empty",
            "body",
        ));
    }
    if trimmed.len() > 64 * 1024 {
        return Err(prompt_settings_error(
            "prompt Markdown exceeds the 64 KiB limit",
            "body",
        ));
    }
    let mut remainder = trimmed;
    while let Some(start) = remainder.find("{{") {
        let Some(end) = remainder[start + 2..].find("}}") else {
            return Err(prompt_settings_error(
                "prompt variables must have balanced delimiters",
                template_id,
            ));
        };
        remainder = &remainder[start + 2 + end + 2..];
    }
    let variables = prompt_variables(trimmed);
    if let Some(required) = prompt_variables(bundled_default)
        .into_iter()
        .find(|required| !variables.iter().any(|variable| variable == required))
    {
        return Err(prompt_settings_error(
            &format!("prompt must preserve required variable `{{{{{required}}}}}`"),
            template_id,
        ));
    }
    Ok(())
}

pub(crate) fn render_configured_prompt(
    template_id: &str,
    bundled_default: &str,
    substitutions: &[(&str, &str)],
) -> String {
    let body = PromptTemplateRegistry::from_env()
        .read_setting(template_id)
        .map(|setting| setting.current)
        .unwrap_or_else(|_| bundled_default.to_string());
    render_bundled_prompt(&body, substitutions)
}

fn prompt_settings_error(message: &str, setting_id: &str) -> DaemonError {
    DaemonError::ProviderProtocol {
        provider_run_id: "prompt-settings".to_string(),
        operation: "prompt_settings",
        message: format!("{message}: {setting_id}"),
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PromptAssemblyService {
    registry: PromptTemplateRegistry,
}

impl PromptAssemblyService {
    pub(crate) fn from_env() -> Result<Self, DaemonError> {
        let registry = PromptTemplateRegistry::from_env();
        registry.materialize_bundled_defaults()?;
        Ok(Self { registry })
    }

    pub(crate) fn new(registry: PromptTemplateRegistry) -> Self {
        Self { registry }
    }

    pub(crate) fn registry(&self) -> &PromptTemplateRegistry {
        &self.registry
    }

    pub(crate) fn assemble_provider_turn(
        &self,
        run: &RuntimeProviderRun,
        visible_user_prompt: &str,
        additional_hidden_context: Option<&str>,
        attachments: Vec<PromptAttachment>,
        mode: PromptAssemblyMode,
    ) -> Result<PromptEnvelope, DaemonError> {
        let mut hidden_fragments = Vec::new();
        let mut manifest = PromptManifest::current();

        self.push_template("runtime/base", &mut hidden_fragments, &mut manifest)?;
        if current_kernel_is_slice() {
            self.push_template("runtime/slice", &mut hidden_fragments, &mut manifest)?;
        }
        let execution_template = if run.requires_workspace_live_sync() {
            "runtime/workspace-live-sync"
        } else if run.tracks_workspace_live_sync() {
            "runtime/workspace-live-sync-tracked"
        } else {
            "runtime/native-permissions"
        };
        self.push_template(execution_template, &mut hidden_fragments, &mut manifest)?;
        if mode == PromptAssemblyMode::MetaagentProviderTurn {
            self.push_template(
                "runtime/metaagent-delegation",
                &mut hidden_fragments,
                &mut manifest,
            )?;
        }
        if let Some(additional_hidden_context) = additional_hidden_context
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            hidden_fragments.push(additional_hidden_context.to_string());
        }
        if additional_hidden_context.is_some_and(|context| {
            context.contains("<workflow-runtime-instructions>")
                || context.contains("Chariox workflow turn")
        }) {
            self.push_template(
                "runtime/workflow-direct-json-fallback",
                &mut hidden_fragments,
                &mut manifest,
            )?;
        }

        if matches!(
            mode,
            PromptAssemblyMode::McpSkillContinuationTurn
                | PromptAssemblyMode::NativeTuiProviderTurn
                | PromptAssemblyMode::MetaagentProviderTurn
                | PromptAssemblyMode::NormalProviderTurn
                | PromptAssemblyMode::WorkflowNodeTurn
                | PromptAssemblyMode::UtilityTurn
        ) {
            // Mode is currently recorded by the caller-specific manifest entries added later.
        }

        Ok(PromptEnvelope::new(
            visible_user_prompt,
            hidden_fragments.join("\n\n"),
            attachments,
            manifest,
        ))
    }

    pub(crate) fn assemble_hidden_context_only(
        &self,
        template_ids: &[&str],
    ) -> Result<(String, PromptManifest), DaemonError> {
        let mut hidden_fragments = Vec::new();
        let mut manifest = PromptManifest::current();
        for template_id in template_ids {
            self.push_template(template_id, &mut hidden_fragments, &mut manifest)?;
        }
        Ok((hidden_fragments.join("\n\n"), manifest))
    }

    pub(crate) fn assemble_mcp_skill_continuation_context(
        &self,
        mcp_name: &str,
    ) -> Result<(String, PromptManifest), DaemonError> {
        let (hidden_context, manifest) =
            self.assemble_hidden_context_only(&["runtime/mcp-skill-continuation"])?;
        Ok((
            hidden_context.replace("{{MCP_NAME}}", mcp_name.trim()),
            manifest,
        ))
    }

    pub(crate) fn assemble_meta_mode_entered_context(
        &self,
    ) -> Result<(String, PromptManifest), DaemonError> {
        self.assemble_hidden_context_only(&["runtime/meta-mode-entered"])
    }

    fn push_template(
        &self,
        template_id: &str,
        fragments: &mut Vec<String>,
        manifest: &mut PromptManifest,
    ) -> Result<(), DaemonError> {
        let template = self.registry.read_required(template_id)?;
        manifest.push_body(template.id.clone(), &template.body);
        if !template.body.trim().is_empty() {
            let body = strip_legacy_template_heading(template_id, &template.body);
            fragments.push(prompt_component(&prompt_component_tag(template_id), &body));
        }
        Ok(())
    }
}

pub(crate) fn bundled_workflow_turn_template() -> &'static str {
    WORKFLOW_TURN
}

pub(crate) fn bundled_workflow_run_completion_template() -> &'static str {
    WORKFLOW_RUN_COMPLETION
}

pub(crate) fn bundled_workflow_run_intermediate_output_template() -> &'static str {
    WORKFLOW_RUN_INTERMEDIATE_OUTPUT
}

pub(crate) fn bundled_workflow_run_output_correction_template() -> &'static str {
    WORKFLOW_RUN_OUTPUT_CORRECTION
}

pub(crate) fn bundled_workflow_handoff_correction_template() -> &'static str {
    WORKFLOW_HANDOFF_CORRECTION
}

pub(crate) fn bundled_workflow_missing_output_correction_template() -> &'static str {
    WORKFLOW_MISSING_OUTPUT_CORRECTION
}

pub(crate) fn render_bundled_prompt(template: &str, replacements: &[(&str, &str)]) -> String {
    replacements
        .iter()
        .fold(template.to_string(), |body, (key, value)| {
            body.replace(&format!("{{{{{key}}}}}"), value)
        })
}

pub(crate) fn bundled_metaagent_event_template() -> &'static str {
    RUNTIME_METAAGENT_EVENT
}

fn current_kernel_is_slice() -> bool {
    std::env::var("CHARIOX_MACHINE_ID")
        .ok()
        .is_some_and(|machine_id| machine_id.starts_with("slice:"))
        || std::env::var("CHARIOX_SLICE_MACHINE_ID")
            .ok()
            .is_some_and(|machine_id| machine_id.starts_with("slice:"))
}

fn bundled_templates() -> Vec<BundledPromptTemplate> {
    vec![
        BundledPromptTemplate::new("runtime/base", RUNTIME_BASE),
        BundledPromptTemplate::new("runtime/workspace-live-sync", RUNTIME_WORKSPACE_LIVE_SYNC),
        BundledPromptTemplate::new(
            "runtime/workspace-live-sync-tracked",
            RUNTIME_WORKSPACE_LIVE_SYNC_TRACKED,
        ),
        BundledPromptTemplate::new("runtime/native-permissions", RUNTIME_NATIVE_PERMISSIONS),
        BundledPromptTemplate::new("runtime/slice", RUNTIME_SLICE),
        BundledPromptTemplate::new("runtime/metaagent-delegation", RUNTIME_METAAGENT_DELEGATION),
        BundledPromptTemplate::new("runtime/meta-mode-entered", RUNTIME_META_MODE_ENTERED),
        BundledPromptTemplate::new(
            "runtime/mcp-skill-continuation",
            RUNTIME_MCP_SKILL_CONTINUATION,
        ),
        BundledPromptTemplate::new(
            "runtime/workflow-direct-json-fallback",
            RUNTIME_WORKFLOW_DIRECT_JSON_FALLBACK,
        ),
        BundledPromptTemplate::new("runtime/metaagent-event", RUNTIME_METAAGENT_EVENT),
        BundledPromptTemplate::new("workflow/turn", WORKFLOW_TURN),
        BundledPromptTemplate::new("workflow/run-completion", WORKFLOW_RUN_COMPLETION),
        BundledPromptTemplate::new(
            "workflow/run-intermediate-output",
            WORKFLOW_RUN_INTERMEDIATE_OUTPUT,
        ),
        BundledPromptTemplate::new(
            "workflow/run-output-correction",
            WORKFLOW_RUN_OUTPUT_CORRECTION,
        ),
        BundledPromptTemplate::new("workflow/handoff-correction", WORKFLOW_HANDOFF_CORRECTION),
        BundledPromptTemplate::new(
            "workflow/missing-output-correction",
            WORKFLOW_MISSING_OUTPUT_CORRECTION,
        ),
        BundledPromptTemplate::new(
            "utility/workspace-commit-message",
            UTILITY_WORKSPACE_COMMIT_MESSAGE,
        ),
        BundledPromptTemplate::new(
            "utility/semantic-recall-search",
            UTILITY_SEMANTIC_RECALL_SEARCH,
        ),
    ]
}

struct BundledPromptTemplate {
    id: &'static str,
    body: &'static str,
}

impl BundledPromptTemplate {
    fn new(id: &'static str, body: &'static str) -> Self {
        Self { id, body }
    }
}

fn sha256_hex(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn prompt_component(tag: &str, body: &str) -> String {
    let escaped = escape_prompt_component_delimiters(tag, body.trim());
    format!("<{tag}>\n{escaped}\n</{tag}>")
}

pub(crate) fn assembled_prompt_component(tag: &str, body: &str) -> String {
    format!("<{tag}>\n{}\n</{tag}>", body.trim())
}

pub(crate) fn unescape_prompt_component_delimiters(body: &str) -> String {
    CHARIOX_PROMPT_COMPONENT_TAGS
        .iter()
        .fold(body.to_string(), |value, tag| {
            unescape_prompt_component_tag_delimiters(&value, tag)
        })
}

fn prompt_component_tag(template_id: &str) -> String {
    match template_id {
        "runtime/base" => "runtime-instructions",
        "runtime/workspace-live-sync" => "workspace-live-sync-instructions",
        "runtime/workspace-live-sync-tracked" => "workspace-live-sync-tracked-instructions",
        "runtime/native-permissions" => "native-permission-instructions",
        "runtime/slice" => "slice-runtime-instructions",
        "runtime/metaagent-delegation" => "metaagent-delegation-instructions",
        "runtime/meta-mode-entered" => "meta-mode-entered-context",
        "runtime/mcp-skill-continuation" => "mcp-skill-continuation-context",
        "runtime/workflow-direct-json-fallback" => "workflow-direct-json-fallback",
        "runtime/metaagent-event" => "metaagent-event",
        "workflow/turn" => "workflow-runtime-instructions",
        "workflow/run-completion" | "workflow/run-intermediate-output" => {
            "system-node-level-prompt"
        }
        "utility/workspace-commit-message" => "workspace-commit-message-instructions",
        "utility/semantic-recall-search" => "semantic-recall-search-instructions",
        _ => return template_id.replace('/', "-"),
    }
    .to_string()
}

fn strip_legacy_template_heading(template_id: &str, body: &str) -> String {
    let heading = match template_id {
        "runtime/workflow-direct-json-fallback" => Some("Workflow direct JSON fallback:"),
        _ => None,
    };
    heading
        .and_then(|heading| body.trim().strip_prefix(heading))
        .unwrap_or(body.trim())
        .trim_start()
        .to_string()
}

fn escape_prompt_component_delimiters(component_tag: &str, body: &str) -> String {
    let escaped = escape_prompt_component_tag_delimiters(body, component_tag);
    CHARIOX_PROMPT_COMPONENT_TAGS
        .iter()
        .filter(|tag| **tag != component_tag)
        .fold(escaped, |value, tag| {
            escape_prompt_component_tag_delimiters(&value, tag)
        })
}

fn escape_prompt_component_tag_delimiters(body: &str, tag: &str) -> String {
    body.replace(&format!("<{tag}>"), &format!("&lt;{tag}&gt;"))
        .replace(&format!("</{tag}>"), &format!("&lt;/{tag}&gt;"))
}

fn unescape_prompt_component_tag_delimiters(body: &str, tag: &str) -> String {
    body.replace(&format!("&lt;{tag}&gt;"), &format!("<{tag}>"))
        .replace(&format!("&lt;/{tag}&gt;"), &format!("</{tag}>"))
}

const CHARIOX_PROMPT_COMPONENT_TAGS: &[&str] = &[
    "runtime-instructions",
    "workspace-live-sync-instructions",
    "workspace-live-sync-tracked-instructions",
    "native-permission-instructions",
    "slice-runtime-instructions",
    "metaagent-delegation-instructions",
    "meta-mode-entered-context",
    "mcp-skill-continuation-context",
    "workflow-direct-json-fallback",
    "metaagent-event",
    "workspace-commit-message-instructions",
    "semantic-recall-search-instructions",
    "endpoint-prompt",
    "workflow-level-prompt",
    "node-level-prompt",
    "workflow-runtime-instructions",
    "system-node-level-prompt",
    "workflow-handoff-payloads",
    "outgoing-edge-contracts",
    "node-instruction-reference",
    "control-mailbox",
];

fn prompt_io_error(operation: &'static str, path: &Path, error: std::io::Error) -> DaemonError {
    DaemonError::ProviderProtocol {
        provider_run_id: "prompt-assembly".to_string(),
        operation,
        message: format!("prompt template path {:?}: {error}", path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env_lock;
    use crate::provider::{
        AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult, RuntimeProviderRun,
    };
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_prompt_root(name: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let index = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir()
            .join("chariox-prompt-assembly-tests")
            .join(format!("{}-{}-{index}", name, std::process::id()))
    }

    #[test]
    fn metaagent_user_prompts_use_normal_turn_context() {
        assert_eq!(
            provider_turn_mode_for_prompt("agent-meta", true, Some("browser-terminal"), "",),
            PromptAssemblyMode::NormalProviderTurn
        );
        assert_eq!(
            provider_turn_mode_for_prompt(
                "agent-meta",
                true,
                Some("metaagent:agent-meta:task"),
                "",
            ),
            PromptAssemblyMode::MetaagentProviderTurn
        );
        assert_eq!(
            provider_turn_mode_for_prompt(
                "agent-meta",
                true,
                Some("browser-terminal"),
                "<meta-mode-entered-context>task</meta-mode-entered-context>",
            ),
            PromptAssemblyMode::MetaagentProviderTurn
        );
    }

    fn test_run(workspace_live_sync: bool) -> RuntimeProviderRun {
        test_run_with_live_sync_mode(if workspace_live_sync {
            crate::config::WorkspaceLiveSyncMode::Managed
        } else {
            crate::config::WorkspaceLiveSyncMode::Unrestricted
        })
    }

    fn test_run_with_live_sync_mode(
        mode: crate::config::WorkspaceLiveSyncMode,
    ) -> RuntimeProviderRun {
        let request = LaunchProviderRequest::new("session", "agent", "codex", "default", "gpt-5.4");
        let request = request.with_workspace_live_sync_mode(mode);
        RuntimeProviderRun::new(
            "provider-run",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "codex".to_string(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("ws://127.0.0.1:43112".to_string()),
            },
        )
    }

    #[test]
    fn prompt_registry_materializes_bundled_defaults_on_first_run() {
        let root = temp_prompt_root("materializes");
        let registry = PromptTemplateRegistry::new(root.clone());

        registry
            .materialize_bundled_defaults()
            .expect("defaults should materialize");

        assert!(root.join("runtime").join("base.md").exists());
        assert!(root
            .join("runtime")
            .join("workspace-live-sync-tracked.md")
            .exists());
        assert!(root.join("workflow").join("turn.md").exists());
        let base = fs::read_to_string(root.join("runtime").join("base.md"))
            .expect("base prompt should read");
        assert!(base.contains("chariox.list_extensions"));
        assert!(base.contains("chariox.list_session_agents"));
        assert!(base.contains("chariox.get_session_agent"));
        assert!(base.contains("chariox.send_agent_message"));
        assert!(base.contains("Treat every interaction as an equal-level, self-contained message"));
        assert!(base
            .contains("Include a follow-up destination only when the sender explicitly requests"));
        let workflow_turn = fs::read_to_string(root.join("workflow").join("turn.md"))
            .expect("workflow turn prompt should read");
        assert!(workflow_turn.contains("workflow_handoffs"));
        assert!(workflow_turn.contains("validate_workflow_handoff"));
    }

    #[test]
    fn prompt_registry_reads_user_edited_templates() {
        let root = temp_prompt_root("edited");
        let registry = PromptTemplateRegistry::new(root.clone());
        registry
            .materialize_bundled_defaults()
            .expect("defaults should materialize");
        fs::write(root.join("runtime").join("base.md"), "USER EDITED TOKEN")
            .expect("user edit should write");
        registry
            .materialize_bundled_defaults()
            .expect("rematerializing should preserve user edits");

        let template = registry
            .read_required("runtime/base")
            .expect("template should read");

        assert_eq!(template.body, "USER EDITED TOKEN");
    }

    #[test]
    fn prompt_registry_updates_unchanged_bundled_defaults() {
        let root = temp_prompt_root("updates-defaults");
        let registry = PromptTemplateRegistry::new(root.clone());
        registry
            .materialize_bundled_defaults()
            .expect("defaults should materialize");
        let path = root.join("workflow").join("turn.md");
        fs::write(&path, "PREVIOUS BUNDLED DEFAULT").expect("old default should write");
        let mut template_sha256 = BTreeMap::new();
        template_sha256.insert(
            "workflow/turn".to_string(),
            sha256_hex("PREVIOUS BUNDLED DEFAULT"),
        );
        let state = BundledPromptDefaultsState {
            version: "previous".to_string(),
            template_sha256,
        };
        fs::write(
            root.join(PROMPT_DEFAULTS_STATE_FILE),
            serde_json::to_string(&state).expect("state should serialize"),
        )
        .expect("state should write");

        registry
            .materialize_bundled_defaults()
            .expect("unchanged old defaults should update");

        let body = fs::read_to_string(path).expect("updated default should read");
        assert!(body.contains("do not validate the outer routing wrapper"));
        assert!(body.contains("completed incoming edge and MUST NOT be used"));
    }

    #[test]
    fn prompt_registry_updates_known_metaagent_legacy_defaults() {
        let root = temp_prompt_root("updates-metaagent-legacy");
        let registry = PromptTemplateRegistry::new(root.clone());
        registry
            .materialize_bundled_defaults()
            .expect("defaults should materialize");
        let path = root.join("runtime").join("meta-mode-entered.md");
        fs::write(
            &path,
            concat!(
                "Kernel mode transition: this agent is now operating in Chariox meta mode for the active task.\n\n",
                "Delegate implementation to owned regular agents or workflows. Use Chariox meta tools for planning, supervision, task state, and allowed capability provisioning. ",
                "Finish by calling `chariox.meta.complete_task`, `chariox.meta.mark_blocked`, or by honoring user pause/abort controls.",
            ),
        )
        .expect("legacy default should write");

        registry
            .materialize_bundled_defaults()
            .expect("legacy Meta default should update despite current state metadata");

        let body = fs::read_to_string(path).expect("updated Meta default should read");
        assert!(body.contains("On continuation, first check `chariox.meta.session_overview`"));
    }

    #[test]
    fn prompt_registry_updates_known_slice_legacy_defaults() {
        let root = temp_prompt_root("updates-slice-legacy");
        let registry = PromptTemplateRegistry::new(root.clone());
        registry
            .materialize_bundled_defaults()
            .expect("defaults should materialize");
        let path = root.join("runtime").join("slice.md");
        fs::write(
            &path,
            concat!(
                "You are running inside a Chariox slice. Slice-only runtime MCP tools are available for the slice screen, browser, keyboard, mouse, and OCR. Use these tools only for the slice environment attached to this agent.\n\n",
                "Use `slice_screen_status` to inspect the display and viewer URL, `slice_screenshot` to capture the screen, `slice_ocr` to extract screen text, `slice_find_text` to locate visible text coordinates, `slice_mouse` for mouse actions, `slice_keyboard` for keyboard actions, and `slice_open_url` to open a URL in the slice browser.\n\n",
                "Use `paste_secret_to_slice` only after focusing the intended browser field. Pass the credential id and set `submit` only when the focused form should be submitted with Return. This pastes the secret through the slice screen without exposing the secret value in your answer or terminal output.\n\n",
                "Prefer `slice_find_text` before clicking text in the browser or GUI because it returns screen coordinates directly. Use `slice_ocr` when visual text matters but the page or app is not accessible through files, terminal output, or browser automation.",
            ),
        )
        .expect("legacy slice default should write");

        registry
            .materialize_bundled_defaults()
            .expect("legacy slice default should update despite current state metadata");

        let body = fs::read_to_string(path).expect("updated slice default should read");
        assert!(body.contains("replace the loopback hostname with `host.docker.internal`"));
        assert!(body.contains("prefer DOM tools before OCR or coordinates"));
    }

    #[test]
    fn prompt_registry_fails_when_required_template_is_missing() {
        let root = temp_prompt_root("missing");
        let registry = PromptTemplateRegistry::new(root);

        let error = registry
            .read_required("runtime/base")
            .expect_err("missing template should fail");

        assert!(error
            .to_string()
            .contains("required prompt template `runtime/base` missing"));
    }

    #[test]
    fn prompt_manifest_records_template_hashes_and_version() {
        let root = temp_prompt_root("manifest");
        let registry = PromptTemplateRegistry::new(root);
        registry
            .materialize_bundled_defaults()
            .expect("defaults should materialize");
        let service = PromptAssemblyService::new(registry);

        let envelope = service
            .assemble_provider_turn(
                &test_run(false),
                "visible prompt",
                None,
                Vec::new(),
                PromptAssemblyMode::NormalProviderTurn,
            )
            .expect("envelope should assemble");

        assert_eq!(envelope.manifest.version, PROMPT_REGISTRY_VERSION);
        assert!(envelope
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "runtime/base" && entry.sha256.len() == 64));
        assert!(envelope
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "runtime/native-permissions"));
        assert!(envelope
            .hidden_system_context
            .contains("<runtime-instructions>"));
        assert!(envelope
            .hidden_system_context
            .contains("<native-permission-instructions>"));
    }

    #[test]
    fn metaagent_provider_turn_includes_delegation_template() {
        let root = temp_prompt_root("metaagent");
        let registry = PromptTemplateRegistry::new(root);
        registry
            .materialize_bundled_defaults()
            .expect("defaults should materialize");
        let service = PromptAssemblyService::new(registry);

        let envelope = service
            .assemble_provider_turn(
                &test_run(false),
                "visible prompt",
                None,
                Vec::new(),
                PromptAssemblyMode::MetaagentProviderTurn,
            )
            .expect("envelope should assemble");

        assert!(envelope
            .hidden_system_context
            .contains("This agent is operating in Chariox Meta mode"));
        assert!(envelope
            .hidden_system_context
            .contains("start by calling `chariox.meta.read_task`"));
        assert!(envelope
            .hidden_system_context
            .contains("confirm it appears in"));
        assert!(envelope
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "runtime/metaagent-delegation"));
    }

    #[test]
    fn normal_provider_turn_excludes_metaagent_delegation_template() {
        let root = temp_prompt_root("not-metaagent");
        let registry = PromptTemplateRegistry::new(root);
        registry
            .materialize_bundled_defaults()
            .expect("defaults should materialize");
        let service = PromptAssemblyService::new(registry);

        let envelope = service
            .assemble_provider_turn(
                &test_run(false),
                "visible prompt",
                None,
                Vec::new(),
                PromptAssemblyMode::NormalProviderTurn,
            )
            .expect("envelope should assemble");

        assert!(!envelope
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "runtime/metaagent-delegation"));
    }

    #[test]
    fn workflow_turns_include_direct_json_fallback() {
        let root = temp_prompt_root("native-workflow-fallback");
        let registry = PromptTemplateRegistry::new(root);
        registry
            .materialize_bundled_defaults()
            .expect("defaults should materialize");
        let service = PromptAssemblyService::new(registry);

        let envelope = service
            .assemble_provider_turn(
                &test_run(false),
                "visible prompt",
                Some(
                    "<workflow-runtime-instructions>\nYou are an agent participating in a Chariox workflow turn.\n</workflow-runtime-instructions>",
                ),
                Vec::new(),
                PromptAssemblyMode::NormalProviderTurn,
            )
            .expect("envelope should assemble");

        assert!(envelope
            .hidden_system_context
            .contains("do not write pseudo tool calls"));
        assert!(envelope
            .hidden_system_context
            .contains("<workflow-direct-json-fallback>"));
        assert!(!envelope
            .hidden_system_context
            .contains("Workflow direct JSON fallback:"));
        assert!(envelope
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "runtime/workflow-direct-json-fallback"));
    }

    #[test]
    fn prompt_envelope_keeps_hidden_context_out_of_visible_prompt() {
        let root = temp_prompt_root("hidden");
        let registry = PromptTemplateRegistry::new(root.clone());
        registry
            .materialize_bundled_defaults()
            .expect("defaults should materialize");
        fs::write(root.join("runtime").join("base.md"), "HIDDEN_RUNTIME_TOKEN")
            .expect("user edit should write");
        let service = PromptAssemblyService::new(registry);

        let envelope = service
            .assemble_provider_turn(
                &test_run(false),
                "user visible prompt",
                None,
                Vec::new(),
                PromptAssemblyMode::NormalProviderTurn,
            )
            .expect("envelope should assemble");

        assert_eq!(envelope.visible_user_prompt, "user visible prompt");
        assert!(envelope
            .hidden_system_context
            .contains("HIDDEN_RUNTIME_TOKEN"));
        assert!(!envelope
            .visible_user_prompt
            .contains("HIDDEN_RUNTIME_TOKEN"));
    }

    #[test]
    fn workspace_live_sync_uses_workspace_template() {
        let root = temp_prompt_root("workspace-live-sync");
        let registry = PromptTemplateRegistry::new(root);
        registry
            .materialize_bundled_defaults()
            .expect("defaults should materialize");
        let service = PromptAssemblyService::new(registry);

        let envelope = service
            .assemble_provider_turn(
                &test_run(true),
                "visible",
                None,
                Vec::new(),
                PromptAssemblyMode::NormalProviderTurn,
            )
            .expect("envelope should assemble");

        assert!(envelope
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "runtime/workspace-live-sync"));
        assert!(envelope
            .hidden_system_context
            .contains("Do not interpret live sync as a global filesystem restriction"));
        assert!(envelope
            .hidden_system_context
            .contains("use provider-native edit/write/patch or shell/bash tools normally"));
        assert!(!envelope
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "runtime/native-permissions"));
    }

    #[test]
    fn tracked_workspace_live_sync_uses_tracked_template() {
        let root = temp_prompt_root("workspace-live-sync-tracked");
        let registry = PromptTemplateRegistry::new(root);
        registry
            .materialize_bundled_defaults()
            .expect("defaults should materialize");
        let service = PromptAssemblyService::new(registry);

        let envelope = service
            .assemble_provider_turn(
                &test_run_with_live_sync_mode(crate::config::WorkspaceLiveSyncMode::Tracked),
                "visible",
                None,
                Vec::new(),
                PromptAssemblyMode::NormalProviderTurn,
            )
            .expect("envelope should assemble");

        assert!(envelope
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "runtime/workspace-live-sync-tracked"));
        assert!(envelope
            .hidden_system_context
            .contains("provider-native edits inside the selected synced roots are allowed"));
        assert!(envelope
            .hidden_system_context
            .contains("Other repositories outside the synced roots are not part of live sync"));
        assert!(!envelope
            .hidden_system_context
            .contains("Direct filesystem writes inside those roots are unavailable"));
        assert!(!envelope
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "runtime/workspace-live-sync"));
        assert!(!envelope
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "runtime/native-permissions"));
    }

    #[test]
    fn slice_kernels_include_slice_template() {
        let _guard = env_lock::lock();
        std::env::set_var("CHARIOX_MACHINE_ID", "slice:test");
        std::env::remove_var("CHARIOX_SLICE_MACHINE_ID");
        let root = temp_prompt_root("slice");
        let registry = PromptTemplateRegistry::new(root);
        registry
            .materialize_bundled_defaults()
            .expect("defaults should materialize");
        let service = PromptAssemblyService::new(registry);

        let envelope = service
            .assemble_provider_turn(
                &test_run(false),
                "visible",
                None,
                Vec::new(),
                PromptAssemblyMode::NormalProviderTurn,
            )
            .expect("envelope should assemble");
        std::env::remove_var("CHARIOX_MACHINE_ID");

        assert!(envelope
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "runtime/slice"));
        assert!(envelope
            .hidden_system_context
            .contains("replace the loopback hostname with `host.docker.internal`"));
    }

    #[test]
    fn mcp_skill_continuation_context_uses_registry_template() {
        let root = temp_prompt_root("mcp-continuation");
        let registry = PromptTemplateRegistry::new(root.clone());
        registry
            .materialize_bundled_defaults()
            .expect("defaults should materialize");
        fs::write(
            root.join("runtime").join("mcp-skill-continuation.md"),
            "CONTINUATION_TEMPLATE {{MCP_NAME}}",
        )
        .expect("user edit should write");
        let service = PromptAssemblyService::new(registry);

        let (hidden, manifest) = service
            .assemble_mcp_skill_continuation_context("playwright")
            .expect("continuation context should assemble");

        assert_eq!(
            hidden,
            "<mcp-skill-continuation-context>\nCONTINUATION_TEMPLATE playwright\n</mcp-skill-continuation-context>"
        );
        assert!(manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "runtime/mcp-skill-continuation"));
    }

    #[test]
    fn meta_mode_entered_context_uses_registry_template() {
        let root = temp_prompt_root("meta-mode-transition");
        let registry = PromptTemplateRegistry::new(root.clone());
        registry
            .materialize_bundled_defaults()
            .expect("defaults should materialize");
        fs::write(
            root.join("runtime").join("meta-mode-entered.md"),
            "ENTERED_TEMPLATE",
        )
        .expect("user edit should write");
        let service = PromptAssemblyService::new(registry);

        let (entered, entered_manifest) = service
            .assemble_meta_mode_entered_context()
            .expect("entered context should assemble");

        assert_eq!(
            entered,
            "<meta-mode-entered-context>\nENTERED_TEMPLATE\n</meta-mode-entered-context>"
        );
        assert!(entered_manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "runtime/meta-mode-entered"));
    }

    #[test]
    fn prompt_settings_catalog_reads_edits_and_resets_without_exposing_paths() {
        let root = temp_prompt_root("settings-catalog");
        let registry = PromptTemplateRegistry::new(root.clone());
        let settings = registry.list_settings().expect("catalog should load");
        let workflow = settings
            .iter()
            .find(|setting| setting.id == "workflow/turn")
            .expect("workflow prompt should be catalogued");
        assert!(workflow.editable);
        assert_eq!(workflow.source, "bundled");
        assert!(workflow.current_sha256.len() == 64);
        assert_eq!(workflow.current_bytes, workflow.current.len());
        assert!(!workflow.default.is_empty());

        let default = registry
            .read_setting("workflow/turn")
            .expect("workflow prompt should read");
        let updated = registry
            .update_setting(
                "workflow/turn",
                &format!("{}\nHello {{{{NAME}}}}", default.default),
            )
            .expect("editable prompt should update");
        assert!(updated.current.contains("Hello {{NAME}}"));
        assert!(updated.variables.iter().any(|variable| variable == "NAME"));
        assert_eq!(updated.source, "user_override");
        assert_eq!(updated.current_bytes, updated.current.len());
        assert_ne!(updated.current_sha256, updated.default_sha256);

        let reset = registry
            .reset_setting("workflow/turn")
            .expect("prompt should reset");
        assert_eq!(reset.current_sha256, reset.default_sha256);
        assert!(registry.root().starts_with(root));
    }

    #[test]
    fn prompt_settings_reject_missing_required_variables() {
        let root = temp_prompt_root("required-variables");
        let registry = PromptTemplateRegistry::new(root);
        let error = registry
            .update_setting("workflow/turn", "custom instructions")
            .expect_err("contract variables must be preserved");
        assert!(error.to_string().contains("required variable"));
    }

    #[test]
    fn configured_correction_prompt_uses_user_override() {
        let _guard = env_lock::lock();
        let home = temp_prompt_root("configured-correction");
        let root = home.join("prompts");
        let registry = PromptTemplateRegistry::new(root.clone());
        registry
            .materialize_bundled_defaults()
            .expect("defaults should materialize");
        fs::write(
            root.join("workflow").join("run-output-correction.md"),
            "OVERRIDE {{ATTEMPT}} {{MAX_ATTEMPTS}} {{ERROR}}",
        )
        .expect("correction override should write");
        let previous = std::env::var_os("CHARIOX_HOME");
        std::env::set_var("CHARIOX_HOME", &home);
        let rendered = render_configured_prompt(
            "workflow/run-output-correction",
            bundled_workflow_run_output_correction_template(),
            &[("ATTEMPT", "1"), ("MAX_ATTEMPTS", "3"), ("ERROR", "bad")],
        );
        match previous {
            Some(value) => std::env::set_var("CHARIOX_HOME", value),
            None => std::env::remove_var("CHARIOX_HOME"),
        }
        assert_eq!(rendered, "OVERRIDE 1 3 bad");
    }

    #[test]
    fn prompt_registry_concurrent_reads_never_observe_partial_updates() {
        let root = temp_prompt_root("atomic-updates");
        let registry = PromptTemplateRegistry::new(root);
        registry
            .materialize_bundled_defaults()
            .expect("defaults should materialize");
        let default = registry
            .read_setting("workflow/run-output-correction")
            .expect("correction prompt should read")
            .default;
        let writer_registry = registry.clone();
        let reader_registry = registry.clone();
        let writer = std::thread::spawn(move || {
            for index in 0..40 {
                writer_registry
                    .update_setting(
                        "workflow/run-output-correction",
                        &format!("{default}\nmarker-{index}"),
                    )
                    .expect("atomic prompt update should succeed");
            }
        });
        for _ in 0..200 {
            let setting = reader_registry
                .read_setting("workflow/run-output-correction")
                .expect("concurrent prompt read should succeed");
            assert!(!setting.current.is_empty());
            assert!(setting.current.contains("{{ERROR}}"));
        }
        writer.join().expect("writer should finish");
    }

    #[test]
    fn protected_prompt_settings_are_read_only_but_resettable() {
        let root = temp_prompt_root("settings-protected");
        let registry = PromptTemplateRegistry::new(root);
        let error = registry
            .update_setting("runtime/base", "unsafe override")
            .expect_err("protected prompts must not be editable");
        assert!(error.to_string().contains("protected prompt setting"));
        let setting = registry
            .reset_setting("runtime/base")
            .expect("protected prompt should reset");
        assert!(setting.protected);
    }
}
