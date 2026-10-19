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
/// within its turn budget (a real turn needs 0.3-1 s per file and loses track of long lists). Files in small directories are
/// offered first because bulk output trees are regenerated; the rest are left out unless already
/// decided.
const OFFERED_PRIVATE_FILES: usize = 32;
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

pub fn project_environment_discovery_prompt(
    input: &ProjectEnvironmentDiscoveryInput,
) -> Result<String, DaemonError> {
    let offered_files = offered_private_files(&input.private_files);
    // The utility classifies references and the offered files, so only changed paths one of them
    // names are worth sending; every other tracked path would only dwarf the prompt.
    let relevant: HashSet<(&str, &str)> = input
        .references
        .iter()
        .chain(input.previous_manifest.iter().flat_map(|m| &m.entries))
        .flat_map(|entry| {
            let located = match &entry.locator {
                ProjectEnvironmentLocator::EnvFile { path, .. }
                | ProjectEnvironmentLocator::ConfigFile { path } => Some(path.as_str()),
                _ => None,
            };
            let named = (entry.kind == ProjectEnvironmentEntryKind::ConfigFile)
                .then_some(entry.name.as_str());
            entry
                .uses
                .iter()
                .map(|usage| usage.path.as_str())
                .chain(located)
                .chain(named)
                .map(|path| (entry.workspace_id.as_str(), path))
        })
        .chain(
            offered_files
                .iter()
                .map(|file| (file.workspace_id.as_str(), file.path.as_str())),
        )
        .collect();
    let mut changed_paths = input.changed_paths.clone();
    for (workspace, paths) in &mut changed_paths {
        paths.retain(|path| relevant.contains(&(workspace.as_str(), path.as_str())));
    }
    changed_paths.retain(|_, paths| !paths.is_empty());
    // Earlier decisions for files the utility is not shown stay in kernel state, not in the prompt.
    let previous_manifest = input.previous_manifest.clone().map(|mut manifest| {
        manifest.private_files.retain(|decision| {
            offered_files.iter().any(|file| {
                file.workspace_id == decision.workspace_id && file.path == decision.path
            })
        });
        manifest
    });
    let offered = ProjectEnvironmentDiscoveryInput {
        private_files: offered_files.into_iter().cloned().collect(),
        previous_manifest,
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
        .ok()
        .filter(serde_json::Value::is_object)
        .ok_or_else(|| environment_error("invalid Project environment discovery output"))?;
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
    let offered = offered_private_files(&input.private_files);
    // Only a decision for an offered file counts, once, and never a plain overlay for a
    // secret-looking one; anything else the model wrote is ignored, not an invalid answer.
    let mut decided = HashSet::new();
    manifest.private_files.retain(|decision| {
        offered.iter().any(|candidate| {
            candidate.workspace_id == decision.workspace_id
                && candidate.path == decision.path
                && (!decision.bring || !candidate.secret_looking)
        }) && decided.insert((decision.workspace_id.clone(), decision.path.clone()))
    });
    manifest.validate().map_err(environment_error)?;
    if manifest.project_id != input.project_id || manifest.evidence_digest != input.evidence_digest
    {
        return Err(environment_error(
            "discovery Project or evidence binding mismatch",
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
    // A file without a decision (not offered, or skipped by the utility) keeps its earlier
    // decision, else stays behind as an explicit unreviewed decision the owner can see in the review.
    let decided: HashSet<_> = manifest
        .private_files
        .iter()
        .map(|file| (file.workspace_id.clone(), file.path.clone()))
        .collect();
    for candidate in &input.private_files {
        if decided.contains(&(candidate.workspace_id.clone(), candidate.path.clone())) {
            continue;
        }
        let known = input.previous_manifest.as_ref().and_then(|manifest| {
            manifest.private_files.iter().find(|known| {
                known.workspace_id == candidate.workspace_id && known.path == candidate.path
            })
        });
        manifest.private_files.push(
            known
                .cloned()
                .unwrap_or_else(|| ProjectPrivateFileDecision {
                    workspace_id: candidate.workspace_id.clone(),
                    path: candidate.path.clone(),
                    bring: false,
                    reason: UNREVIEWED_PRIVATE_FILE_REASON.into(),
                    secret_looking: candidate.secret_looking,
                }),
        );
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
