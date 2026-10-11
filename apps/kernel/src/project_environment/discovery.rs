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
        "MP-08 / MP-10 / MP-11 Project environment discovery. Use only the kernel-provided metadata below. Tools are disabled. Never read files, execute commands, retrieve credentials, or return values. Classify uncertain entries as secret. Return only a JSON object (no Markdown or commentary) with schema_version 1, the exact project_id and evidence_digest, entries, private_files, toolchain_hints, package_hints and service_hints. Each entries item has ONLY reference_index (zero-based index into the supplied references array) and classification (secret or non_secret). The kernel starts every supplied reference as secret. Emit only indexes with a deliberate classification decision; omitted indexes remain secret. Do not enumerate uncertain references. Do not emit or invent names, workspace IDs, kinds, use sites, locators, status or excluded fields: the kernel retains those fields from the indexed reference and resolves actual values. The kernel preserves unchanged previous entries. Decide every private_files candidate in private_files, with workspace_id/path/bring/reason (one line explaining project use or regeneration). Exclude caches, runtime files and secret-looking files from the plain overlay; referenced secrets use entries and Vault. Do not treat a missing configuration value as failed setup. Never claim readiness.\nMetadata:\n{encoded}"
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
    let manifest: ProjectEnvironmentManifest = serde_json::from_str(output)
        .map_err(|_| environment_error("invalid Project environment discovery output"))?;
    validate_discovery_manifest(manifest, input)
}

fn validate_discovery_manifest(
    mut manifest: ProjectEnvironmentManifest,
    input: &ProjectEnvironmentDiscoveryInput,
) -> Result<ProjectEnvironmentManifest, DaemonError> {
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

/// MP-08 / MP-10 / MP-11: the utility classifies indexed kernel references.
pub fn parse_project_environment_discovery_decisions(
    output: &str,
    input: &ProjectEnvironmentDiscoveryInput,
) -> Result<ProjectEnvironmentManifest, DaemonError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Decision {
        reference_index: usize,
        #[serde(default)]
        classification: ProjectEnvironmentClassification,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Decisions {
        schema_version: u32,
        project_id: String,
        evidence_digest: String,
        entries: Vec<Decision>,
        #[serde(default)]
        private_files: Vec<ProjectPrivateFileDecision>,
        #[serde(default)]
        toolchain_hints: Vec<String>,
        #[serde(default)]
        package_hints: Vec<String>,
        #[serde(default)]
        service_hints: Vec<String>,
    }
    if output.len() > 1024 * 1024 {
        return Err(environment_error(
            "environment discovery output exceeds bounds",
        ));
    }
    let decisions: Decisions = serde_json::from_str(output)
        .map_err(|_| environment_error("invalid Project environment discovery output"))?;
    let mut entries = input.references.clone();
    for entry in &mut entries {
        entry.classification = ProjectEnvironmentClassification::Secret;
    }
    let mut seen = std::collections::BTreeSet::new();
    for decision in decisions.entries {
        let entry = entries.get_mut(decision.reference_index).ok_or_else(|| {
            environment_error("discovery introduced unreferenced name or locator")
        })?;
        if !seen.insert(decision.reference_index) {
            return Err(environment_error(
                "invalid Project environment entry binding or evidence",
            ));
        }
        entry.classification = decision.classification;
    }
    validate_discovery_manifest(
        ProjectEnvironmentManifest {
            schema_version: decisions.schema_version,
            project_id: decisions.project_id,
            evidence_digest: decisions.evidence_digest,
            entries,
            private_files: decisions.private_files,
            toolchain_hints: decisions.toolchain_hints,
            package_hints: decisions.package_hints,
            service_hints: decisions.service_hints,
        },
        input,
    )
}
