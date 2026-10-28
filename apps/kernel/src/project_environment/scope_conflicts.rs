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
    let mut a = a.clone();
    let mut b = b.clone();
    if let (
        RequirementSpec::Software {
            platform: pa,
            version_constraint: va,
            ..
        },
        RequirementSpec::Software {
            platform: pb,
            version_constraint: vb,
            ..
        },
    ) = (&mut a, &mut b)
    {
        *pa = None;
        *pb = None;
        let versions_overlap =
            super::check_inventory::constraints_overlap(va.as_deref(), vb.as_deref())
                != Some(false);
        *va = None;
        *vb = None;
        return versions_overlap && a == b;
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

#[cfg(test)]
mod review2148 {
    use super::*;
    fn software(id: &str, version: Option<&str>) -> Requirement {
        serde_json::from_value(serde_json::json!({"requirement_id":id,"title":id,"scope":{"kind":"project"},"origins":[],"spec":{"kind":"software","identity":"node","version_constraint":version,"platform":null,"install_scope":"project","install_source":"mise","detect_only":false},"depends_on":[],"platform_variants":[],"required":true,"legacy_entry":null})).unwrap()
    }
    #[test]
    fn review2148_compatible_manifest_constraints_can_be_saved() {
        for (a, b) in [
            (None, Some("22.3.1")),
            (Some(">=22 <23"), Some("22.3.1")),
            (Some("^22.1"), Some("~22.3")),
        ] {
            let draft = EnvironmentRevisionDraft {
                project_requirements: vec![software("a", a), software("b", b)],
                folders: vec![],
            };
            assert!(
                validate_scopes(&draft).is_ok(),
                "overlapping software declarations must be admitted: {a:?}, {b:?}"
            );
        }
        let draft = EnvironmentRevisionDraft {
            project_requirements: vec![
                software("a", Some(">=22 <23")),
                software("b", Some(">=24")),
            ],
            folders: vec![],
        };
        assert!(
            validate_scopes(&draft).is_err(),
            "disjoint declarations must still be rejected"
        );
    }
}
