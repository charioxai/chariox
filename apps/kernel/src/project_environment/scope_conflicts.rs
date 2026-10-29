//! MP-08 / MP-10 / MP-11: specification conflicts are review failures, not target readiness.
use super::*;
use crate::error::DaemonError;

// MP-08 / MP-10 / MP-11: variants restrict a declaration; a Software
// platform further restricts those variants, matching Check's applicability.
fn platforms(r: &Requirement) -> Vec<EnvironmentPlatform> {
    let any = EnvironmentPlatform {
        os: "any".into(),
        architecture: "any".into(),
    };
    let base = match &r.spec {
        RequirementSpec::Software {
            platform: Some(p), ..
        } => p,
        _ => &any,
    };
    if r.platform_variants.is_empty() {
        return vec![base.clone()];
    }
    r.platform_variants
        .iter()
        .filter_map(|variant| {
            if !overlap(base, variant) {
                return None;
            }
            Some(EnvironmentPlatform {
                os: if base.os == "any" {
                    variant.os.clone()
                } else {
                    base.os.clone()
                },
                architecture: if base.architecture == "any" {
                    variant.architecture.clone()
                } else {
                    base.architecture.clone()
                },
            })
        })
        .collect()
}
fn overlap(a: &EnvironmentPlatform, b: &EnvironmentPlatform) -> bool {
    (a.os == "any" || b.os == "any" || a.os == b.os)
        && (a.architecture == "any" || b.architecture == "any" || a.architecture == b.architecture)
}
// MP-08 / MP-10 / MP-11: platform selectors restrict applicability above;
// compatibility compares the requested software on that intersection.
fn compatible(a: &RequirementSpec, b: &RequirementSpec) -> bool {
    if let (
        RequirementSpec::Software {
            version_constraint: a,
            ..
        },
        RequirementSpec::Software {
            version_constraint: b,
            ..
        },
    ) = (a, b)
    {
        if !super::software_constraints::overlap(a.as_deref(), b.as_deref()) {
            return false;
        }
    }
    let mut a = a.clone();
    let mut b = b.clone();
    if let RequirementSpec::Software {
        platform,
        version_constraint,
        ..
    } = &mut a
    {
        *platform = None;
        *version_constraint = None;
    }
    if let RequirementSpec::Software {
        platform,
        version_constraint,
        ..
    } = &mut b
    {
        *platform = None;
        *version_constraint = None;
    }
    a == b
}
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
                && !compatible(&a.spec, &b.spec)
                && platforms(a)
                    .iter()
                    .any(|pa| platforms(b).iter().any(|pb| overlap(pa, pb)))
            {
                return Err(environment_error("Conflict · incompatible effective project/folder requirements; edit the scope or declaration before Save"));
            }
        }
    }
    Ok(())
}
