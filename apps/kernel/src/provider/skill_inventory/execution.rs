//! MP-08 / MP-10 / MP-11: one inventory entry point on the execution kernel.
use super::*;
use crate::provider::{AgentExecutionMode, CodexClient, OpenCodeClient, RuntimeProviderRun};

pub const DISCOVERY_ENVIRONMENT: &[&str] = &[
    "HOME",
    "CODEX_HOME",
    "CLAUDE_CONFIG_DIR",
    "OPENCODE_CONFIG_DIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "OPENCODE_DISABLE_CLAUDE_SKILLS",
    "OPENCODE_DISABLE_EXTERNAL_SKILLS",
];

/// Capture only discovery inputs at actual launch, before any later process
/// environment change. Profile/isolation overrides and removals win. Never
/// retain or read authentication variables for native skill discovery.
pub(crate) fn capture_launch_skill_environment(
    overrides: &BTreeMap<String, String>,
    removals: &[String],
) -> BTreeMap<String, String> {
    DISCOVERY_ENVIRONMENT
        .iter()
        .filter_map(|key| {
            let value = overrides.get(*key).cloned().or_else(|| {
                (!removals.iter().any(|removed| removed.as_str() == *key))
                    .then(|| std::env::var(*key).ok())
                    .flatten()
            });
            value
                .filter(|value| !value.is_empty())
                .map(|value| ((*key).into(), value))
        })
        .collect()
}

pub struct NativeSkillDiscoveryRequest<'a> {
    pub provider_version: &'a str,
    pub execution_id: &'a str,
    pub project_id: Option<&'a str>,
    pub catalog_revision: u64,
    pub source_revision: u64,
    pub worktree_root: &'a std::path::Path,
    pub additional_directories: &'a [PathBuf],
    pub enabled_plugin_skill_roots: &'a [PathBuf],
    pub configured_skill_roots: &'a [PathBuf],
}

pub fn inventory_for_run(
    run: &RuntimeProviderRun,
    cache: &mut NativeSkillInventoryCache,
    request: NativeSkillDiscoveryRequest<'_>,
    refresh: bool,
) -> Result<NativeSkillInventory, DaemonError> {
    let cwd = run
        .working_directory()
        .ok_or_else(|| failure("native inventory requires the actual provider cwd"))?;
    let environment = run.native_skill_discovery_environment();
    if environment.is_empty() {
        return Err(failure("restored provider has no resolved launch inventory environment; refresh or reattach the provider"));
    }
    let binding = SkillInventoryBinding {
        provider: run.provider().into(),
        provider_version: request.provider_version.into(),
        account_profile: run.account_profile().into(),
        execution_id: request.execution_id.into(),
        cwd: cwd.clone(),
        environment: environment.clone(),
        project_id: request.project_id.map(str::to_string),
        catalog_revision: request.catalog_revision,
        source_revision: request.source_revision,
    };
    cache.inventory(&binding, refresh, |force_reload| {
        let filesystem = || {
            filesystem_skill_candidates(
                &binding,
                request.worktree_root,
                request.additional_directories,
                request.enabled_plugin_skill_roots,
                request.configured_skill_roots,
            )
        };
        match run.provider() {
            "codex" => CodexClient::new(
                run.id(),
                run.structured_endpoint()
                    .ok_or_else(|| failure("Codex has no official app-server endpoint"))?,
            )?
            .native_skill_candidates(cwd, force_reload),
            "opencode" => {
                let client = OpenCodeClient::new(
                    run.id(),
                    run.structured_endpoint()
                        .ok_or_else(|| failure("OpenCode has no official server endpoint"))?,
                )?;
                let agent = match run.execution_mode() {
                    AgentExecutionMode::Build => "build",
                    AgentExecutionMode::Plan => "plan",
                };
                match client.native_skill_candidates(cwd, agent)? {
                    Some(candidates) => Ok(candidates),
                    None => filesystem(),
                }
            }
            "claude" => filesystem(),
            _ => Err(failure("unsupported skill inventory provider")),
        }
    })
}
