//! MP-08 / MP-10 / MP-11: specification conflicts are review failures, not target readiness.
use super::*;
use crate::error::DaemonError;
pub(super) fn validate_scopes(draft: &EnvironmentRevisionDraft) -> Result<(), DaemonError> {
    let all: Vec<_> = draft
        .project_requirements
        .iter()
        .chain(draft.folders.iter().flat_map(|f| &f.requirements))
        .collect();
    fn key(r: &Requirement) -> Option<(&'static str, &str)> {
        match &r.spec {
            RequirementSpec::Variables { name, .. } => Some(("variable", name)),
            RequirementSpec::Software { identity, .. } => Some(("software", identity)),
            RequirementSpec::Services { identity, .. } => Some(("service", identity)),
            _ => None,
        }
    }
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            let overlaps = a.scope == b.scope
                || a.scope == RequirementScope::Project
                || b.scope == RequirementScope::Project;
            if overlaps
                && a.required
                && b.required
                && key(a).is_some()
                && key(a) == key(b)
                && a.spec != b.spec
            {
                return Err(environment_error("Conflict · incompatible effective project/folder requirements; edit the scope or declaration before Save"));
            }
        }
    }
    Ok(())
}
