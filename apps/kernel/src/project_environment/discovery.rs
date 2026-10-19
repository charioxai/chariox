//! MP-08: Utility input contains reference names and locators, never source values.
use super::resolver::environment_error;
use super::*;
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

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
/// Gitignored files have no count ceiling, but the utility must decide each one it is offered
/// within its turn budget (a real turn needs ~0.3 s per file). Files in small directories are
/// offered first because bulk output trees are regenerated; the rest are left out unless already
/// decided.
const OFFERED_PRIVATE_FILES: usize = 128;
const OFFERED_PRIVATE_FILE_BYTES: usize = 512 * 1024;

pub(crate) fn offered_private_files(
    candidates: &[ProjectPrivateFileCandidate],
) -> Vec<&ProjectPrivateFileCandidate> {
    let directory = |candidate: &ProjectPrivateFileCandidate| {
        (
            candidate.workspace_id.clone(),
            candidate
                .path
                .rsplit_once('/')
                .map_or(String::new(), |(directory, _)| directory.to_owned()),
        )
    };
    let mut fan_out: BTreeMap<(String, String), usize> = BTreeMap::new();
    for candidate in candidates {
        *fan_out.entry(directory(candidate)).or_default() += 1;
    }
    let mut ordered: Vec<_> = candidates.iter().collect();
    ordered.sort_by_key(|candidate| fan_out[&directory(candidate)]);
    let mut used = 0;
    ordered
        .into_iter()
        .take(OFFERED_PRIVATE_FILES)
        .take_while(|candidate| {
            used += candidate.workspace_id.len() + candidate.path.len() + 64;
            used <= OFFERED_PRIVATE_FILE_BYTES
        })
        .collect()
}

/// The candidates the utility is not shown (and so never decides this round).
fn unoffered_private_files<'a>(
    candidates: &'a [ProjectPrivateFileCandidate],
    offered: &[&ProjectPrivateFileCandidate],
) -> Vec<&'a ProjectPrivateFileCandidate> {
    let offered: HashSet<_> = offered
        .iter()
        .map(|candidate| (candidate.workspace_id.as_str(), candidate.path.as_str()))
        .collect();
    candidates
        .iter()
        .filter(|candidate| {
            !offered.contains(&(candidate.workspace_id.as_str(), candidate.path.as_str()))
        })
        .collect()
}

pub fn project_environment_discovery_prompt(
    input: &ProjectEnvironmentDiscoveryInput,
) -> Result<String, DaemonError> {
    let offered_files = offered_private_files(&input.private_files);
    // Inventory files the utility is not shown stay out of changed_paths too; listing every
    // one of them again would dwarf the prompt.
    let mut changed_paths = input.changed_paths.clone();
    let left_out: HashSet<_> = unoffered_private_files(&input.private_files, &offered_files)
        .into_iter()
        .map(|candidate| (candidate.workspace_id.as_str(), candidate.path.as_str()))
        .collect();
    for (workspace, paths) in &mut changed_paths {
        paths.retain(|path| !left_out.contains(&(workspace.as_str(), path.as_str())));
    }
    let offered = ProjectEnvironmentDiscoveryInput {
        private_files: offered_files.into_iter().cloned().collect(),
        changed_paths,
        ..input.clone()
    };
    let encoded = serde_json::to_string(&offered)
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
    let mut value: serde_json::Value = serde_json::from_str(output)
        .map_err(|_| environment_error("invalid Project environment discovery output"))?;
    // Decisions are best effort: a model that derails partway through a long list (an entry
    // without `bring`) or echoes the candidate's size beside a decision must not fail the export.
    // An undecided file is simply left out, like one the utility was not shown.
    let decisions: Vec<ProjectPrivateFileDecision> = value
        .get_mut("private_files")
        .map(serde_json::Value::take)
        .and_then(|decisions| match decisions {
            serde_json::Value::Array(items) => Some(items),
            _ => None,
        })
        .unwrap_or_default()
        .into_iter()
        .filter_map(|mut decision| {
            decision.as_object_mut()?.remove("bytes");
            serde_json::from_value(decision).ok()
        })
        .collect();
    value["private_files"] = serde_json::Value::Array(Vec::new());
    let mut manifest: ProjectEnvironmentManifest = serde_json::from_value(value)
        .map_err(|_| environment_error("invalid Project environment discovery output"))?;
    manifest.private_files = decisions;
    manifest.validate().map_err(environment_error)?;
    if manifest.project_id != input.project_id || manifest.evidence_digest != input.evidence_digest
    {
        return Err(environment_error(
            "discovery Project or evidence binding mismatch",
        ));
    }
    let offered = offered_private_files(&input.private_files);
    let mut decided = HashSet::new();
    if manifest.private_files.iter().any(|decision| {
        !decided.insert((decision.workspace_id.as_str(), decision.path.as_str()))
            || !offered.iter().any(|candidate| {
                candidate.workspace_id == decision.workspace_id
                    && candidate.path == decision.path
                    && (!decision.bring || !candidate.secret_looking)
            })
    }) {
        return Err(environment_error(
            "discovery private file decisions do not match the inventory",
        ));
    }
    for file in &mut manifest.private_files {
        let candidate = offered
            .iter()
            .find(|candidate| {
                candidate.workspace_id == file.workspace_id && candidate.path == file.path
            })
            .expect("inventory validated");
        file.secret_looking = candidate.secret_looking;
    }
    // A file the utility was not shown keeps its earlier decision and is otherwise
    // left out: only an explicit decision brings a private file.
    for candidate in unoffered_private_files(&input.private_files, &offered) {
        if let Some(known) = input.previous_manifest.as_ref().and_then(|manifest| {
            manifest.private_files.iter().find(|known| {
                known.workspace_id == candidate.workspace_id && known.path == candidate.path
            })
        }) {
            manifest.private_files.push(known.clone());
        }
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
