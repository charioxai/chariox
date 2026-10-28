//! MP-08 / MP-10 / MP-11: Kernel-owned immutable review history and compare-and-save.
use super::*;
use crate::error::DaemonError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EnvironmentRevisionHistory {
    revisions: Vec<ProjectEnvironment>,
    decisions: BTreeMap<String, EnvironmentOptInDecision>,
    legacy_digests: Vec<String>,
}

pub fn environment_draft(environment: &ProjectEnvironment) -> EnvironmentRevisionDraft {
    EnvironmentRevisionDraft {
        project_requirements: environment.project_requirements.clone(),
        folders: environment
            .folders
            .iter()
            .map(|f| EnvironmentFolderSpecification {
                folder_id: f.folder_id.clone(),
                portable_folder_key: f.portable_folder_key.clone(),
                label: f.label.clone(),
                optional_git: f.optional_git.clone(),
                requirements: f.requirements.clone(),
            })
            .collect(),
    }
}
fn requirements(draft: &EnvironmentRevisionDraft) -> impl Iterator<Item = &Requirement> {
    draft
        .project_requirements
        .iter()
        .chain(draft.folders.iter().flat_map(|f| &f.requirements))
}
fn draft_digest(lineage: &EnvironmentLineage, draft: &EnvironmentRevisionDraft) -> String {
    metadata_digest(&(
        lineage,
        draft
            .project_requirements
            .iter()
            .map(requirement_specification)
            .collect::<Vec<_>>(),
        draft
            .folders
            .iter()
            .map(|f| {
                (
                    &f.folder_id,
                    &f.portable_folder_key,
                    &f.label,
                    &f.optional_git,
                    f.requirements
                        .iter()
                        .map(requirement_specification)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>(),
    ))
}
fn text(value: &str, limit: usize) -> Result<(), DaemonError> {
    if value.is_empty() || value.len() > limit || value.chars().any(char::is_control) {
        return Err(environment_error("invalid Environment text or bounds"));
    }
    Ok(())
}

// MP-11: Editing specifications never grants capabilities, probes a target or reads a locator.
fn validate_draft(
    current: &ProjectEnvironment,
    draft: &EnvironmentRevisionDraft,
) -> Result<(), DaemonError> {
    if draft.folders.len() != current.folders.len()
        || requirements(draft).count() > 4096
        || serde_json::to_vec(draft)
            .map_err(|_| environment_error("draft encoding failed"))?
            .len()
            > 8 * 1024 * 1024
    {
        return Err(environment_error(
            "Environment draft topology or bounds invalid",
        ));
    }
    let mut folder_ids = BTreeSet::new();
    for folder in &draft.folders {
        let original = current
            .folders
            .iter()
            .find(|f| f.folder_id == folder.folder_id)
            .ok_or_else(|| environment_error("folder does not belong to Project"))?;
        if !folder_ids.insert(&folder.folder_id)
            || folder.portable_folder_key != original.portable_folder_key
            || folder.optional_git != original.optional_git
        {
            return Err(environment_error("folder identity cannot be edited"));
        }
        if original.local_workspace_binding.is_empty()
            && environment_draft(current)
                .folders
                .iter()
                .find(|saved| saved.folder_id == folder.folder_id)
                != Some(folder)
        {
            return Err(environment_error(
                "Environment revision conflict; folder is no longer attached; refresh and review",
            ));
        }
        text(&folder.label, 256)?;
    }
    let before = environment_draft(current);
    let known: BTreeMap<_, _> = requirements(&before)
        .chain(current.proposals.iter().map(|p| &p.requirement))
        .map(|r| (r.requirement_id.as_str(), r))
        .collect();
    let mut ids = BTreeSet::new();
    for (scope, rows) in std::iter::once((RequirementScope::Project, &draft.project_requirements))
        .chain(draft.folders.iter().map(|f| {
            (
                RequirementScope::Folder {
                    folder_id: f.folder_id.clone(),
                },
                &f.requirements,
            )
        }))
    {
        for r in rows {
            text(&r.requirement_id, 256)?;
            text(&r.title, 1024)?;
            if r.scope != scope
                || !ids.insert(&r.requirement_id)
                || r.depends_on.len() > 128
                || r.platform_variants.len() > 32
                || r.origins.len() > 128
            {
                return Err(environment_error(
                    "invalid requirement identity, scope or bounds",
                ));
            }
            // Origins and legacy evidence are authoritative. New user items receive server provenance.
            if let Some(old) = known.get(r.requirement_id.as_str()) {
                if r.origins != old.origins || r.legacy_entry != old.legacy_entry {
                    return Err(environment_error("requirement provenance cannot be edited"));
                }
            } else if !r.origins.is_empty() || r.legacy_entry.is_some() {
                return Err(environment_error(
                    "new requirement cannot invent provenance",
                ));
            }
            // Uniform recursive string/list bounds on the tagged specification.
            fn bounded(value: &serde_json::Value) -> bool {
                match value {
                    serde_json::Value::String(s) => {
                        s.len() <= 4096 && !s.chars().any(char::is_control)
                    }
                    serde_json::Value::Array(a) => a.len() <= 4096 && a.iter().all(bounded),
                    serde_json::Value::Object(o) => o.values().all(bounded),
                    _ => true,
                }
            }
            if !bounded(
                &serde_json::to_value(&r.spec)
                    .map_err(|_| environment_error("invalid specification"))?,
            ) {
                return Err(environment_error(
                    "requirement specification exceeds bounds",
                ));
            }
            match &r.spec {
                RequirementSpec::Software {
                    identity,
                    version_constraint,
                    install_source,
                    ..
                } => {
                    text(identity, 256)?;
                    if let Some(version) = version_constraint {
                        text(version, 256)?;
                    }
                    if install_source
                        .as_ref()
                        .is_some_and(|s| s.contains('@') || s.contains('?') || s.contains('#'))
                    {
                        return Err(environment_error(
                            "software source must not contain credentials",
                        ));
                    }
                }
                RequirementSpec::Variables { name, locator } => {
                    if !environment_variable_name(name) {
                        return Err(environment_error("invalid variable name"));
                    }
                    match locator {
                        ProjectEnvironmentLocator::EnvFile { path, key } => {
                            relative_environment_path(path).map_err(environment_error)?;
                            if !environment_variable_name(key) {
                                return Err(environment_error("invalid variable key"));
                            }
                        }
                        ProjectEnvironmentLocator::ConfigFile { path, .. } => {
                            relative_environment_path(path).map_err(environment_error)?
                        }
                        ProjectEnvironmentLocator::WorkspaceEnvironment { name } => {
                            if !environment_variable_name(name) {
                                return Err(environment_error("invalid variable reference"));
                            }
                        }
                        ProjectEnvironmentLocator::Vault { .. } => {
                            return Err(environment_error("Vault references must be Secrets"))
                        }
                        ProjectEnvironmentLocator::Missing => {}
                    }
                }
                RequirementSpec::Files { folder_id, entries } => {
                    if scope
                        != (RequirementScope::Folder {
                            folder_id: folder_id.clone(),
                        })
                    {
                        return Err(environment_error("File scope mismatch"));
                    }
                    // P03 owns new selection and credential scanning. P02b can review detected Files only.
                    if !known
                        .get(r.requirement_id.as_str())
                        .is_some_and(|old| old.spec == r.spec)
                    {
                        return Err(environment_error("manual File editing is not delivered"));
                    }
                    for entry in entries {
                        relative_environment_path(&entry.relative_path)
                            .map_err(environment_error)?;
                    }
                }
                RequirementSpec::SetupChecks { .. }
                | RequirementSpec::AgentTools { .. }
                | RequirementSpec::CharioxApps { .. }
                | RequirementSpec::Accounts { .. }
                | RequirementSpec::Services { .. } => {
                    // References/commands require the later typed editor and admission reviews.
                    if !known
                        .get(r.requirement_id.as_str())
                        .is_some_and(|old| old.spec == r.spec)
                    {
                        return Err(environment_error("this reference edit is not delivered"));
                    }
                }
                RequirementSpec::Secrets { name, vault } => {
                    text(name, 128)?;
                    if let Some(v) = vault {
                        text(&v.service, 256)?;
                        text(&v.key, 256)?;
                    }
                }
                RequirementSpec::MachineNeeds { .. } => {}
            }
        }
    }
    for r in requirements(draft) {
        if r.depends_on
            .iter()
            .any(|id| id == &r.requirement_id || !ids.contains(id))
        {
            return Err(environment_error("invalid requirement dependency"));
        }
    }
    Ok(())
}

pub fn environment_revision_diff(
    current: &ProjectEnvironment,
    draft: &EnvironmentRevisionDraft,
) -> Result<EnvironmentRevisionDiff, DaemonError> {
    validate_draft(current, draft)?;
    let before = environment_draft(current);
    let old: BTreeMap<_, _> = requirements(&before)
        .map(|r| (&r.requirement_id, r))
        .collect();
    let new: BTreeMap<_, _> = requirements(draft)
        .map(|r| (&r.requirement_id, r))
        .collect();
    let ids: BTreeSet<_> = old.keys().chain(new.keys()).copied().collect();
    Ok(EnvironmentRevisionDiff {
        project_id: current.local_project_id.clone(),
        expected_revision: current.revision,
        source_digest: current.content_digest.clone(),
        target_digest: draft_digest(&current.lineage, draft),
        requirements: ids
            .into_iter()
            .map(|id| {
                let before = old.get(id).copied().cloned();
                let after = new.get(id).copied().cloned();
                let kind = match (&before, &after) {
                    (None, Some(_)) => EnvironmentDiffKind::Added,
                    (Some(_), None) => EnvironmentDiffKind::Removed,
                    (Some(a), Some(b)) if a == b => EnvironmentDiffKind::Unchanged,
                    _ => EnvironmentDiffKind::Changed,
                };
                EnvironmentRequirementDiff {
                    requirement_id: id.clone(),
                    kind,
                    before,
                    after,
                }
            })
            .collect(),
        folder_map: current
            .folders
            .iter()
            .map(|f| EnvironmentFolderBinding {
                folder_id: f.folder_id.clone(),
                local_workspace_binding: f.local_workspace_binding.clone(),
            })
            .collect(),
    })
}

impl ProjectEnvironmentStore {
    pub(crate) fn load_revisions(
        &self,
        project: &str,
    ) -> Result<Option<EnvironmentRevisionHistory>, DaemonError> {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = match options.open(self.path(project).with_extension("revisions.json")) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(environment_error("revision history unavailable")),
        };
        if !file
            .metadata()
            .map_err(|_| environment_error("revision metadata unavailable"))?
            .is_file()
        {
            return Err(environment_error("revision history must be regular"));
        }
        let mut bytes = vec![];
        file.take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| environment_error("revision read failed"))?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(environment_error("revision history exceeds bounds"));
        }
        let history: EnvironmentRevisionHistory = serde_json::from_slice(&bytes)
            .map_err(|_| environment_error("invalid revision history"))?;
        if history.revisions.len() < 2
            || history
                .revisions
                .iter()
                .enumerate()
                .any(|(i, r)| r.local_project_id != project || r.revision != i as u64)
        {
            return Err(environment_error("revision history binding mismatch"));
        }
        Ok(Some(history))
    }
    pub(crate) fn attach_revision(
        &self,
        snapshot: &mut ProjectEnvironment,
    ) -> Result<(), DaemonError> {
        if let Some(history) = self.load_revisions(&snapshot.local_project_id)? {
            let saved = history.revisions.last().expect("validated history");
            if saved.lineage != snapshot.lineage {
                return Err(environment_error("revision lineage mismatch"));
            }
            snapshot.revision = saved.revision;
            snapshot.content_digest = saved.content_digest.clone();
            snapshot.parent_revision_digest = saved.parent_revision_digest.clone();
            snapshot.reviewed_at_ms = saved.reviewed_at_ms;
            snapshot.reviewed_by = saved.reviewed_by.clone();
            snapshot.project_requirements = saved.project_requirements.clone();
            // Revision contents remain immutable; attachment is a separate local projection.
            // A detached saved folder has an empty local binding.
            let bindings: BTreeMap<_, _> = snapshot
                .folders
                .iter()
                .map(|folder| {
                    (
                        folder.folder_id.clone(),
                        folder.local_workspace_binding.clone(),
                    )
                })
                .collect();
            snapshot.folders = saved.folders.clone();
            for folder in &mut snapshot.folders {
                folder.local_workspace_binding =
                    bindings.get(&folder.folder_id).cloned().unwrap_or_default();
            }
            snapshot.proposals.retain(|p| {
                !history
                    .decisions
                    .contains_key(&p.requirement.requirement_id)
            });
        }
        // Opaque review tokens bind the content shown to the client. Decisions remain
        // keyed by stable requirement identity so a later Detect preserves user choices.
        for proposal in &mut snapshot.proposals {
            proposal.proposal_id =
                metadata_digest(&requirement_specification(&proposal.requirement));
        }
        Ok(())
    }
    pub(crate) fn save_revision_locked(
        &self,
        current: &ProjectEnvironment,
        request: &crate::local::SaveProjectEnvironmentRevisionRequest,
        user: &str,
    ) -> Result<(ProjectEnvironment, EnvironmentRevisionDiff), DaemonError> {
        if current.revision != request.expected_revision
            || current.content_digest != request.expected_content_digest
        {
            return Err(environment_error("Environment revision conflict; refresh and review the current diff before resubmitting"));
        }
        let mut history = self
            .load_revisions(&current.local_project_id)?
            .unwrap_or_else(|| EnvironmentRevisionHistory {
                revisions: vec![current.clone()],
                decisions: BTreeMap::new(),
                legacy_digests: [
                    current.legacy_manifest.as_ref().map(metadata_digest),
                    current
                        .legacy_reviewed_manifest
                        .as_ref()
                        .map(metadata_digest),
                    current
                        .project_requirements
                        .iter()
                        .find_map(|r| match &r.spec {
                            RequirementSpec::SetupChecks {
                                legacy: Some(l), ..
                            } => Some(l.definition_digest.clone()),
                            _ => None,
                        }),
                ]
                .into_iter()
                .flatten()
                .collect(),
            });
        let mut draft = request.draft.clone();
        validate_draft(current, &draft)?;
        let mut selected = BTreeSet::new();
        let mut accepted_requirements = BTreeSet::new();
        if request.accepted_proposal_ids.len() + request.excluded_proposal_ids.len() > 4096 {
            return Err(environment_error("proposal decisions exceed bounds"));
        }
        for (ids, decision) in [
            (
                &request.accepted_proposal_ids,
                EnvironmentOptInDecision::Accepted,
            ),
            (
                &request.excluded_proposal_ids,
                EnvironmentOptInDecision::Excluded,
            ),
        ] {
            for id in ids {
                if !selected.insert(id) {
                    return Err(environment_error("duplicate proposal decision"));
                }
                let proposal = current
                    .proposals
                    .iter()
                    .find(|p| &p.proposal_id == id)
                    .ok_or_else(|| {
                        environment_error("Environment revision conflict; proposal is no longer pending; refresh and review")
                    })?;
                if requirements(&draft)
                    .any(|r| r.requirement_id == proposal.requirement.requirement_id)
                {
                    return Err(environment_error("proposal already present in draft"));
                }
                if decision == EnvironmentOptInDecision::Accepted {
                    accepted_requirements.insert(proposal.requirement.requirement_id.clone());
                    let mut requirement = proposal.requirement.clone();
                    requirement.required = true;
                    match &requirement.scope {
                        RequirementScope::Project => draft.project_requirements.push(requirement),
                        RequirementScope::Folder { folder_id } => draft
                            .folders
                            .iter_mut()
                            .find(|f| &f.folder_id == folder_id)
                            .ok_or_else(|| environment_error("proposal folder unavailable"))?
                            .requirements
                            .push(requirement),
                    }
                }
                history.decisions.insert(
                    proposal.requirement.requirement_id.clone(),
                    decision.clone(),
                );
            }
        }
        validate_draft(current, &draft)?;
        for r in draft
            .project_requirements
            .iter_mut()
            .chain(draft.folders.iter_mut().flat_map(|f| &mut f.requirements))
        {
            if r.origins.is_empty() {
                r.origins.push(RequirementOrigin::UserAdded {
                    user_id: user.into(),
                    revision: current.revision + 1,
                });
            }
        }
        let mut saved = current.clone();
        saved.project_requirements = draft.project_requirements.clone();
        for f in &mut saved.folders {
            let d = draft
                .folders
                .iter()
                .find(|d| d.folder_id == f.folder_id)
                .expect("validated folder");
            f.label = d.label.clone();
            f.requirements = d.requirements.clone();
        }
        saved.revision = current
            .revision
            .checked_add(1)
            .ok_or_else(|| environment_error("revision overflow"))?;
        saved.parent_revision_digest = Some(current.content_digest.clone());
        saved.content_digest = draft_digest(&saved.lineage, &environment_draft(&saved));
        saved.reviewed_at_ms = Some(crate::session::unix_epoch_ms());
        saved.reviewed_by = Some(user.into());
        saved.proposals.retain(|p| {
            !history
                .decisions
                .contains_key(&p.requirement.requirement_id)
        });
        let mut diff = environment_revision_diff(current, &request.draft)?;
        diff.target_digest = saved.content_digest.clone();
        // Include server-accepted proposals in the same review diff as field edits.
        for r in requirements(&draft).filter(|r| accepted_requirements.contains(&r.requirement_id))
        {
            diff.requirements.push(EnvironmentRequirementDiff {
                requirement_id: r.requirement_id.clone(),
                kind: EnvironmentDiffKind::Added,
                before: None,
                after: Some(r.clone()),
            });
        }
        let mut persisted = saved.clone();
        persisted.proposals.clear();
        persisted.operations.clear();
        persisted.observations.clear();
        history.revisions.push(persisted);
        let bytes = serde_json::to_vec(&history)
            .map_err(|_| environment_error("revision encoding failed"))?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(environment_error("revision history exceeds bounds"));
        }
        crate::config::write_private_file(
            &self
                .path(&current.local_project_id)
                .with_extension("revisions.json"),
            &bytes,
        )
        .map_err(|_| environment_error("revision commit failed"))?;
        Ok((saved, diff))
    }
}
