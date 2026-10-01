//! MP-08: Utility input contains reference names and locators, never source values.
use super::resolver::environment_error;
use super::*;
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPrivateFileCandidate {
    pub workspace_id: String,
    pub path: String,
    pub bytes: u64,
    pub secret_looking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectEnvironmentDiscoveryInput {
    pub project_id: String,
    pub evidence_digest: String,
    pub changed_paths: BTreeMap<String, Vec<String>>,
    pub previous_manifest: Option<ProjectEnvironmentManifest>,
    pub references: Vec<ProjectEnvironmentEntry>,
    #[serde(default)]
    pub private_files: Vec<ProjectPrivateFileCandidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

/// A tool-free official-provider utility classifies the kernel's name/locator index.
/// It must not receive source files, env files, shell output or a workspace credential binding.
pub fn project_environment_discovery_prompt(
    input: &ProjectEnvironmentDiscoveryInput,
) -> Result<String, DaemonError> {
    let encoded = serde_json::to_string(input)
        .map_err(|_| environment_error("environment discovery input invalid"))?;
    if encoded.len() > 1024 * 1024 {
        return Err(environment_error(
            "environment discovery input exceeds bounds",
        ));
    }
    Ok(format!(
        "MP-08 Project environment discovery. Use only the kernel-provided metadata below. Tools are disabled. Never read files, execute commands, retrieve credentials, or return values. Classify uncertain entries as secret. Return only a JSON object (no Markdown or commentary): a Project environment manifest with schema_version 1, the exact project_id and evidence_digest, entries with name/workspace_id/kind/classification/uses/locator/status, and toolchain_hints/package_hints/service_hints. Names, use sites and locators must come from references or unchanged previous entries. Cover only changed_paths when a previous manifest exists; preserve unchanged entries. Use found/missing/problem status; the kernel resolves actual values. Do not treat a missing Project configuration value as a failed setup recipe: the kernel projects all missing names once through a secret RuntimeInteraction. Decide every private_files candidate in private_files, with workspace_id/path/bring/reason (one line explaining project use or regeneration). Exclude caches, runtime files and secret-looking files from the plain overlay; referenced secrets use entries and Vault. Never claim readiness.\nMetadata:\n{encoded}"
    ))
}

pub fn parse_project_environment_discovery_output(
    output: &str,
    input: &ProjectEnvironmentDiscoveryInput,
) -> Result<ProjectEnvironmentManifest, DaemonError> {
    if output.len() > 1024 * 1024 {
        return Err(environment_error(
            "environment discovery output exceeds bounds",
        ));
    }
    let mut manifest: ProjectEnvironmentManifest = serde_json::from_str(output)
        .map_err(|_| environment_error("invalid Project environment discovery output"))?;
    manifest.validate().map_err(environment_error)?;
    if manifest.project_id != input.project_id || manifest.evidence_digest != input.evidence_digest
    {
        return Err(environment_error(
            "discovery Project or evidence binding mismatch",
        ));
    }
    if manifest.private_files.len() != input.private_files.len()
        || manifest.private_files.iter().any(|decision| {
            !input.private_files.iter().any(|candidate| {
                candidate.workspace_id == decision.workspace_id
                    && candidate.path == decision.path
                    && (!decision.bring || !candidate.secret_looking)
            })
        })
    {
        return Err(environment_error(
            "discovery private file decisions do not match the inventory",
        ));
    }
    for file in &mut manifest.private_files {
        let candidate = input
            .private_files
            .iter()
            .find(|candidate| {
                candidate.workspace_id == file.workspace_id && candidate.path == file.path
            })
            .expect("inventory validated");
        file.secret_looking = candidate.secret_looking;
    }
    let previous = input
        .previous_manifest
        .as_ref()
        .map(|manifest| manifest.entries.as_slice())
        .unwrap_or(&[]);
    for entry in &manifest.entries {
        let supported = input.references.iter().chain(previous).any(|known| {
            entry.workspace_id == known.workspace_id
                && entry.name == known.name
                && entry.kind == known.kind
                && entry.uses == known.uses
                && entry.locator == known.locator
        });
        if !supported {
            return Err(environment_error(
                "discovery introduced unreferenced name or locator",
            ));
        }
    }
    // MP-08 / MP-10 / MP-11: The kernel preserves unchanged decisions. An
    // incremental utility classifies changed metadata, not saved selections or
    // value availability (which the resolver determines after discovery).
    if input.revision.is_none() {
        for known in previous {
            let changed = input.changed_paths.get(&known.workspace_id);
            if known
                .uses
                .iter()
                .all(|usage| changed.is_none_or(|paths| !paths.contains(&usage.path)))
            {
                if let Some(entry) = manifest.entries.iter_mut().find(|entry| {
                    entry.workspace_id == known.workspace_id
                        && entry.name == known.name
                        && entry.kind == known.kind
                }) {
                    *entry = known.clone();
                } else {
                    manifest.entries.push(known.clone());
                }
            }
        }
    }
    for known in &input.references {
        if !manifest.entries.iter().any(|entry| {
            entry.workspace_id == known.workspace_id
                && entry.name == known.name
                && entry.kind == known.kind
        }) {
            return Err(environment_error(
                "discovery omitted a Project environment reference",
            ));
        }
    }
    normalize_project_config_file_decisions(&mut manifest);
    manifest.validate().map_err(environment_error)?;
    Ok(manifest)
}
