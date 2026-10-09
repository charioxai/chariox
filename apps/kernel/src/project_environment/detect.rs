//! MP-08 / MP-10 / MP-11: Deterministic read-only proposals over a bounded safe evidence index.
use super::*;
use crate::error::DaemonError;
use std::collections::{BTreeMap, BTreeSet};

pub struct EnvironmentDetection {
    pub evidence_digest: String,
    pub proposals: Vec<EnvironmentProposal>,
    pub skips: Vec<EnvironmentItemResult>,
    pub code_folders: BTreeSet<String>,
}
pub fn detect_environment(
    folders: &[EnvironmentFolder],
    environment_id: &str,
) -> Result<EnvironmentDetection, DaemonError> {
    if folders.len() > 32 {
        return Err(environment_error("detection exceeds folder bounds"));
    }
    let index = super::detect_index::collect(folders)?;
    let mut importer = super::detect_importers::Importer {
        environment_id: environment_id.into(),
        proposals: BTreeMap::new(),
        skips: index.skips,
        code_folders: BTreeSet::new(),
    };
    for file in &index.files {
        importer.import(file)
    }
    for file in &index.files {
        importer.import_file(file)
    }
    if importer.skips.len() > 1024 {
        let omitted = importer.skips.len() - 1024;
        importer.skips.truncate(1024);
        importer.skips.push(EnvironmentItemResult { requirement_id: "detect:additional-skips".into(), status: EnvironmentObservationStatus::NeedsYourInput, reason_code: "skip_summary_limit".into(), safe_summary: format!("{omitted} additional exclusions · evidence or proposal bounds; narrow the selected folders for details"), receipt_ids: vec![] });
    }
    let mut proposals: Vec<_> = importer.proposals.into_values().collect();
    // Protected source contributes sanitized semantic names only, no content length/hash.
    let evidence_digest = metadata_digest(&(
        &proposals,
        &importer.skips,
        &importer.code_folders,
        index
            .files
            .iter()
            .map(|file| (&file.folder_id, &file.path, &file.digest))
            .collect::<Vec<_>>(),
    ));
    for proposal in &mut proposals {
        for origin in &mut proposal.requirement.origins {
            if let RequirementOrigin::Detected {
                evidence_digest: digest,
                ..
            } = origin
            {
                *digest = evidence_digest.clone()
            }
        }
    }
    Ok(EnvironmentDetection {
        evidence_digest,
        proposals,
        skips: importer.skips,
        code_folders: importer.code_folders,
    })
}
impl EnvironmentDetection {
    pub fn model_folders(
        &self,
        previous: Option<&EnvironmentDetectionCache>,
        allowed: &[String],
        selected: &[EnvironmentFolder],
    ) -> BTreeSet<String> {
        if previous.is_some_and(|p| {
            p.evidence_digest == self.evidence_digest
                && p.operation.phase == EnvironmentOperationPhase::Ready
        }) {
            return BTreeSet::new();
        }
        selected
            .iter()
            .filter(|folder| {
                (self.code_folders.contains(&folder.folder_id)
                    || allowed.contains(&folder.folder_id))
                    && !previous.is_some_and(|p| {
                        p.operation.phase == EnvironmentOperationPhase::Ready
                            && folder_fingerprint(&p.proposals, &folder.folder_id)
                                == folder_fingerprint(&self.proposals, &folder.folder_id)
                    })
            })
            .map(|f| f.folder_id.clone())
            .collect()
    }
    pub fn discovery_input(
        &self,
        project_id: &str,
        folders: &[EnvironmentFolder],
        model_folders: &BTreeSet<String>,
    ) -> (ProjectEnvironmentDiscoveryInput, bool) {
        let mut paths: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut references: BTreeMap<(String, String), ProjectEnvironmentEntry> = BTreeMap::new();
        for proposal in &self.proposals {
            for origin in &proposal.requirement.origins {
                let RequirementOrigin::Detected {
                    folder_id,
                    relative_path,
                    line,
                    ..
                } = origin
                else {
                    continue;
                };
                if !model_folders.contains(folder_id) {
                    continue;
                }
                let Some(folder) = folders.iter().find(|f| &f.folder_id == folder_id) else {
                    continue;
                };
                // Utility receives portable folder IDs, filenames and use sites only.
                paths
                    .entry(folder_id.clone())
                    .or_default()
                    .insert(relative_path.clone());
                if let RequirementSpec::Secrets { name, .. } = &proposal.requirement.spec {
                    let entry = references
                        .entry((folder_id.clone(), name.clone()))
                        .or_insert_with(|| ProjectEnvironmentEntry {
                            name: name.clone(),
                            workspace_id: folder.folder_id.clone(),
                            kind: ProjectEnvironmentEntryKind::Variable,
                            classification: ProjectEnvironmentClassification::Secret,
                            excluded: false,
                            uses: vec![],
                            locator: ProjectEnvironmentLocator::Missing,
                            status: ProjectEnvironmentEntryStatus::Missing,
                        });
                    if let Some(line) = line {
                        let usage = ProjectEnvironmentUse {
                            path: relative_path.clone(),
                            line: *line,
                        };
                        if !entry.uses.contains(&usage) && entry.uses.len() < 128 {
                            entry.uses.push(usage)
                        }
                    }
                }
            }
        }
        super::detect_metadata::bound_detect_metadata(ProjectEnvironmentDiscoveryInput {
            project_id: project_id.into(),
            evidence_digest: self.evidence_digest.clone(),
            changed_paths: paths
                .into_iter()
                .map(|(folder, paths)| (folder, paths.into_iter().collect()))
                .collect(),
            previous_manifest: None,
            references: references.into_values().collect(),
            private_files: vec![],
            revision: None,
        })
    }
}

fn folder_fingerprint(proposals: &[EnvironmentProposal], folder_id: &str) -> String {
    let mut selected: Vec<_> = proposals.iter().filter(|p| matches!(&p.requirement.scope, RequirementScope::Folder { folder_id: id } if id == folder_id)).cloned().collect();
    for proposal in &mut selected {
        for origin in &mut proposal.requirement.origins {
            if let RequirementOrigin::Detected {
                evidence_digest, ..
            } = origin
            {
                evidence_digest.clear();
            }
        }
    }
    metadata_digest(&selected)
}
