use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::DaemonError;
use crate::provider::RuntimeProviderRun;
use crate::session::PromptAttachment;

const PROMPT_REGISTRY_VERSION: &str = "1";

const RUNTIME_BASE: &str = include_str!("provider/runtime_instructions.md");
const RUNTIME_WORKSPACE_LIVE_SYNC: &str =
    include_str!("provider/workspace_live_sync_instructions.md");
const RUNTIME_NATIVE_PERMISSIONS: &str = include_str!("provider/native_permission_instructions.md");
const RUNTIME_SLICE: &str = include_str!("provider/slice_runtime_instructions.md");

const WORKFLOW_TURN: &str = "You are an agent participating in an Arroba workflow turn.\n\n{{NODE_INSTRUCTION_REFERENCE_BLOCK}}Your node-level instructions are in the referenced markdown file above. If you do not remember them exactly, read that file before continuing.\n\n{{WORKFLOW_HANDOFF_PAYLOADS_BLOCK}}{{OUTGOING_EDGE_CONTRACTS_BLOCK}}{{CONTROL_MAILBOX_BLOCK}}For the proper behavior of the workflow, you MUST acknowledge that you have successfully read the current input from the queue by calling the Arroba runtime MCP tool `ack_workflow_turn` exactly once with this JSON argument object:\n{\"delivery_token\":\"{{DELIVERY_TOKEN}}\"}\n\nIf an outgoing edge contract for this turn includes a `handoff_schema_ref`, you MUST validate your proposed `output.message` before finalizing by calling the Arroba runtime MCP tool `validate_workflow_handoff` with the delivery token above, that `handoff_schema_ref`, and your proposed `output.message` JSON. If no `handoff_schema_ref` is present for this turn, do not call `validate_workflow_handoff`.\n\nIf your node-level instructions require shared console output or inspection, you MUST use the Arroba runtime MCP tools `workflow_console_read`, `workflow_console_write`, and `workflow_console_clear` for that work.\n\nAt the end of this workflow turn, return exactly one fenced ```json block with this shape:\n{\"summary\":\"human-facing summary\",\"output\":{\"message\":\"explicit downstream handoff message\"}}\nDo not output any prose before or after that fenced block. Do not mention acknowledgments, tool calls, or workflow mechanics in the summary unless the task explicitly requires it. The downstream handoff payload is only output.message plus any workflow-owned artifacts.\n\nIf a Control mailbox is present, resolve every listed issue before finalizing and do not repeat the invalid payload. When this turn includes a `handoff_schema_ref`, validation is a gate, not a suggestion. If `validate_workflow_handoff` returns `valid: false` or any warning, do not finalize the turn yet. Revise the proposed handoff, call `validate_workflow_handoff` again, and only finalize once the tool returns `valid: true` with no warning. A single failed validation call does not satisfy this turn's completion requirements.";

const WORKFLOW_RUN_COMPLETION: &str = "System node-level prompt:\nThis node is authorized to complete the workflow run.\nIf you consider that the workflow is complete and the run should stop, or will stop by design at this node, generate final workflow run output and submit it by calling the Arroba runtime MCP tool `validate_and_submit_workflow_run_output`.\nWhen you are generating final workflow run output, normal node-to-node output is not necessary and does not need `validate_workflow_handoff`.\nDo not finalize the turn until `validate_and_submit_workflow_run_output` returns `valid: true` with no warning.\n\n";

const WORKFLOW_RUN_INTERMEDIATE_OUTPUT: &str = "System node-level prompt:\nThis node is authorized to emit intermediate workflow run outputs.\nIf you want to send an intermediate output to the endpoint without terminating the workflow run, call the Arroba runtime MCP tool `validate_and_submit_intermediate_workflow_run_output`.\nIntermediate workflow run output does not terminate the workflow run. You may still need to produce normal node-to-node output for downstream workflow edges in the same turn, and downstream output validation rules still apply.\nDo not finalize the turn until `validate_and_submit_intermediate_workflow_run_output` returns `valid: true` with no warning.\n\n";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PromptEnvelope {
    pub(crate) visible_user_prompt: String,
    pub(crate) hidden_system_context: String,
    pub(crate) attachments: Vec<PromptAttachment>,
    pub(crate) manifest: PromptManifest,
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
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PromptManifest {
    pub(crate) version: String,
    pub(crate) entries: Vec<PromptManifestEntry>,
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
    WorkflowNodeTurn,
    UtilityTurn,
    McpSkillContinuationTurn,
}

#[derive(Debug, Clone)]
pub(crate) struct PromptTemplateRegistry {
    root: PathBuf,
}

impl PromptTemplateRegistry {
    pub(crate) fn from_env() -> Self {
        let arroba_home = std::env::var_os("ARROBA_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|value| !value.is_empty())
                    .map(|home| PathBuf::from(home).join(".arroba"))
            })
            .unwrap_or_else(|| std::env::temp_dir().join("arroba"));
        Self::new(arroba_home.join("prompts"))
    }

    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn materialize_bundled_defaults(&self) -> Result<(), DaemonError> {
        for template in bundled_templates() {
            let path = self.path_for(template.id);
            if path.exists() {
                continue;
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| prompt_io_error("create", &path, error))?;
            }
            fs::write(&path, template.body.trim_end())
                .map_err(|error| prompt_io_error("write", &path, error))?;
        }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PromptTemplate {
    pub(crate) id: String,
    pub(crate) body: String,
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
        let mut manifest = PromptManifest {
            version: PROMPT_REGISTRY_VERSION.to_string(),
            entries: Vec::new(),
        };

        self.push_template("runtime/base", &mut hidden_fragments, &mut manifest)?;
        if current_kernel_is_slice() {
            self.push_template("runtime/slice", &mut hidden_fragments, &mut manifest)?;
        }
        let execution_template = if run.requires_workspace_live_sync() {
            "runtime/workspace-live-sync"
        } else {
            "runtime/native-permissions"
        };
        self.push_template(execution_template, &mut hidden_fragments, &mut manifest)?;
        if let Some(additional_hidden_context) = additional_hidden_context
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            hidden_fragments.push(additional_hidden_context.to_string());
        }

        if matches!(
            mode,
            PromptAssemblyMode::McpSkillContinuationTurn
                | PromptAssemblyMode::NativeTuiProviderTurn
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
        let mut manifest = PromptManifest {
            version: PROMPT_REGISTRY_VERSION.to_string(),
            entries: Vec::new(),
        };
        for template_id in template_ids {
            self.push_template(template_id, &mut hidden_fragments, &mut manifest)?;
        }
        Ok((hidden_fragments.join("\n\n"), manifest))
    }

    fn push_template(
        &self,
        template_id: &str,
        fragments: &mut Vec<String>,
        manifest: &mut PromptManifest,
    ) -> Result<(), DaemonError> {
        let template = self.registry.read_required(template_id)?;
        manifest.entries.push(PromptManifestEntry {
            template_id: template.id,
            sha256: sha256_hex(&template.body),
        });
        if !template.body.trim().is_empty() {
            fragments.push(template.body);
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

fn current_kernel_is_slice() -> bool {
    std::env::var("ARROBA_MACHINE_ID")
        .ok()
        .is_some_and(|machine_id| machine_id.starts_with("slice:"))
        || std::env::var("ARROBA_SLICE_MACHINE_ID")
            .ok()
            .is_some_and(|machine_id| machine_id.starts_with("slice:"))
}

fn bundled_templates() -> Vec<BundledPromptTemplate> {
    vec![
        BundledPromptTemplate::new("runtime/base", RUNTIME_BASE),
        BundledPromptTemplate::new("runtime/workspace-live-sync", RUNTIME_WORKSPACE_LIVE_SYNC),
        BundledPromptTemplate::new("runtime/native-permissions", RUNTIME_NATIVE_PERMISSIONS),
        BundledPromptTemplate::new("runtime/slice", RUNTIME_SLICE),
        BundledPromptTemplate::new("workflow/turn", WORKFLOW_TURN),
        BundledPromptTemplate::new("workflow/run-completion", WORKFLOW_RUN_COMPLETION),
        BundledPromptTemplate::new(
            "workflow/run-intermediate-output",
            WORKFLOW_RUN_INTERMEDIATE_OUTPUT,
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
            .join("arroba-prompt-assembly-tests")
            .join(format!("{}-{}-{index}", name, std::process::id()))
    }

    fn test_run(workspace_live_sync: bool) -> RuntimeProviderRun {
        let request = LaunchProviderRequest::new("session", "agent", "codex", "default", "gpt-5.4");
        let request = if workspace_live_sync {
            request.with_workspace_live_sync_managed()
        } else {
            request
        };
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
        assert!(root.join("workflow").join("turn.md").exists());
        let base = fs::read_to_string(root.join("runtime").join("base.md"))
            .expect("base prompt should read");
        assert!(base.contains("arroba.list_extensions"));
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

        let template = registry
            .read_required("runtime/base")
            .expect("template should read");

        assert_eq!(template.body, "USER EDITED TOKEN");
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
        assert!(!envelope
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "runtime/native-permissions"));
    }

    #[test]
    fn slice_kernels_include_slice_template() {
        let _guard = env_lock::lock();
        std::env::set_var("ARROBA_MACHINE_ID", "slice:test");
        std::env::remove_var("ARROBA_SLICE_MACHINE_ID");
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
        std::env::remove_var("ARROBA_MACHINE_ID");

        assert!(envelope
            .manifest
            .entries
            .iter()
            .any(|entry| entry.template_id == "runtime/slice"));
    }
}
