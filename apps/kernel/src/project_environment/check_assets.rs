//! MP-08 / MP-10 / MP-11: named assets probes; Check does not repair or authorize.
use super::check::{outcome, vault_available, workspace, EnvironmentCheckContext};
use super::*;
use std::collections::BTreeMap;
use zeroize::Zeroizing;
use EnvironmentObservationStatus::{Missing, NeedsYourInput, Outdated, Ready};
pub(super) fn check_assets(
    ctx: &EnvironmentCheckContext,
    env: &ProjectEnvironment,
    r: &Requirement,
    target: &EnvironmentTargetBinding,
    baseline: &mut BTreeMap<String, String>,
    facts: &mut EnvironmentMeasuredFacts,
    deadline: std::time::Instant,
) -> (EnvironmentObservationStatus, String, String) {
    match &r.spec {
        RequirementSpec::Files { entries, .. } => {
            if entries.iter().any(|e| {
                e.secret_looking
                    || matches!(
                        e.credential_filter_verdict,
                        EnvironmentCredentialFilterVerdict::NeedsVault
                            | EnvironmentCredentialFilterVerdict::Rejected
                    )
            }) {
                return outcome(
                    NeedsYourInput,
                    "vault_required",
                    "Protected selection requires Vault; no plaintext digest exposed",
                );
            }
            if entries.is_empty()
                || entries
                    .iter()
                    .any(|e| e.transfer_inclusion != EnvironmentTransferInclusion::Include)
            {
                return outcome(NeedsYourInput, "files_unmeasured", "Required Files include an excluded or unreviewed selection; no presence or digest was measured");
            }
            let mut current = env.clone();
            current.proposals.clear();
            current.project_requirements.clear();
            for f in &mut current.folders {
                f.requirements.clear();
            }
            let mut draft = environment_draft(&current);
            let mut selected = r.clone();
            if let RequirementSpec::Files { entries, .. } = &mut selected.spec {
                for e in entries {
                    e.user_selected = true;
                }
            }
            match &r.scope {
                RequirementScope::Project => draft.project_requirements.push(selected),
                RequirementScope::Folder { folder_id } => {
                    let Some(f) = draft.folders.iter_mut().find(|f| &f.folder_id == folder_id)
                    else {
                        return outcome(
                            Missing,
                            "folder_unavailable",
                            "The required folder is unavailable",
                        );
                    };
                    f.requirements.push(selected);
                }
            }
            if super::check_files::measure(&ctx.config, &current, &mut draft, deadline).is_err() {
                return outcome(
                    Missing,
                    "file_unavailable",
                    "A required file or folder is absent, inaccessible or changed during Check",
                );
            }
            let measured = draft
                .project_requirements
                .iter()
                .chain(draft.folders.iter().flat_map(|f| &f.requirements))
                .next();
            let Some(Requirement {
                spec:
                    RequirementSpec::Files {
                        entries: actual, ..
                    },
                ..
            }) = measured
            else {
                return outcome(Missing, "file_unavailable", "Files could not be checked");
            };
            facts.present = Some(true);
            if actual.iter().any(|e| e.secret_looking) {
                return outcome(
                    NeedsYourInput,
                    "vault_required",
                    "Protected content requires Vault; no plaintext digest exposed",
                );
            }
            if entries.iter().zip(actual).any(|(e, a)| {
                e.content_digest
                    .as_ref()
                    .is_some_and(|d| Some(d) != a.content_digest.as_ref())
            }) {
                return outcome(
                    Outdated,
                    "file_drift",
                    "File contents changed since the saved revision",
                );
            }
            if actual.len() == 1 {
                facts.content_digest = actual[0].content_digest.clone();
            }
            outcome(
                Ready,
                "files_present",
                "Required Files are accessible and saved digests match",
            )
        }
        RequirementSpec::Variables { name, locator } => {
            if project_environment_protected_name(name)
                || project_environment_source_name(locator)
                    .is_some_and(project_environment_protected_name)
            {
                return outcome(
                    NeedsYourInput,
                    "protected_binding",
                    "Protected kernel/provider binding cannot be probed",
                );
            }
            let binding = match locator {
                ProjectEnvironmentLocator::WorkspaceEnvironment { name } => super::check_binding::summarize(std::env::var(name).ok().map(Zeroizing::new)),
                _ => match super::check_files::binding(&ctx.config, workspace(env, r), locator, deadline) {
                    Ok(binding) => binding,
                    Err(_) => return outcome(NeedsYourInput, "binding_unproven", "Binding could not be checked within the probe budget; retry when the folder is available"),
                },
            };
            if binding.needs_vault {
                return outcome(
                    NeedsYourInput,
                    "vault_required",
                    "Binding requires Vault classification; no value or digest exposed",
                );
            }
            if !binding.present {
                facts.present = Some(false);
                return outcome(
                    Missing,
                    "binding_absent",
                    "Named public binding is absent or ambiguous",
                );
            }
            facts.present = Some(true);
            let key = format!(
                "{}:{}:{}",
                env.content_digest, target.machine_id, r.requirement_id
            );
            let hash = binding.digest.expect("present public binding has a digest");
            if baseline.get(&key).is_some_and(|b| b != &hash) {
                return outcome(
                    Outdated,
                    "binding_drift",
                    "Public binding changed since its first Check for this revision",
                );
            }
            baseline.entry(key).or_insert(hash);
            outcome(
                Ready,
                "binding_available",
                "Named public binding is available; value remains hidden",
            )
        }
        RequirementSpec::Secrets { vault, .. } => match vault {
            Some(v) if vault_available(&ctx.config, std::slice::from_ref(v)) => outcome(
                Ready,
                "vault_available",
                "Named Vault reference is authorized and available; value remains hidden",
            ),
            _ => outcome(
                NeedsYourInput,
                "vault_required",
                "Choose or unlock the required named Vault reference",
            ),
        },
        _ => unreachable!("named Check dispatch"),
    }
}
