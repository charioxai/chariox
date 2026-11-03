//! MP-08 / MP-10 / MP-11: Check-driven, target-bound planning. No installer authority.
use super::*;
use crate::error::DaemonError;
use std::collections::BTreeSet;

pub(crate) fn environment_requirements(
    env: &ProjectEnvironment,
) -> impl Iterator<Item = &Requirement> {
    env.project_requirements
        .iter()
        .chain(env.folders.iter().flat_map(|f| &f.requirements))
}
pub(crate) fn environment_dependencies(r: &Requirement) -> Vec<String> {
    let mut ids = r.depends_on.clone();
    if let RequirementSpec::Services { depends_on, .. } = &r.spec {
        ids.extend(depends_on.iter().cloned());
    }
    ids
}
pub(crate) fn apply_selection(
    env: &ProjectEnvironment,
    requested: &[String],
) -> Result<Vec<String>, DaemonError> {
    let all: Vec<_> = environment_requirements(env).collect();
    if requested.len() > 4096
        || requested
            .iter()
            .any(|id| !all.iter().any(|r| &r.requirement_id == id))
    {
        return Err(environment_error(
            "Apply selection does not belong to the saved revision",
        ));
    }
    fn visit(
        id: &str,
        all: &[&Requirement],
        visiting: &mut BTreeSet<String>,
        ordered: &mut Vec<String>,
    ) -> Result<(), DaemonError> {
        if ordered.iter().any(|done| done == id) {
            return Ok(());
        }
        if !visiting.insert(id.into()) {
            return Err(environment_error("Apply dependency cycle"));
        }
        let r = all
            .iter()
            .find(|r| r.requirement_id == id)
            .ok_or_else(|| environment_error("Apply dependency is absent"))?;
        for dependency in environment_dependencies(r) {
            visit(&dependency, all, visiting, ordered)?;
        }
        visiting.remove(id);
        ordered.push(id.into());
        Ok(())
    }
    let mut ordered = vec![];
    for r in &all {
        if (requested.is_empty() && r.required) || requested.contains(&r.requirement_id) {
            visit(&r.requirement_id, &all, &mut BTreeSet::new(), &mut ordered)?;
        }
    }
    Ok(ordered)
}
pub(crate) fn setup_opt_in(r: &Requirement) -> bool {
    matches!(&r.spec, RequirementSpec::SetupChecks { commands, legacy, .. } if !commands.is_empty() || legacy.is_some())
}
pub(crate) fn apply_item_result(
    r: &Requirement,
    status: EnvironmentObservationStatus,
    reason: &str,
    summary: &str,
) -> EnvironmentItemResult {
    EnvironmentItemResult {
        requirement_id: r.requirement_id.clone(),
        status,
        reason_code: reason.into(),
        safe_summary: summary.into(),
        receipt_ids: vec![],
    }
}
pub(crate) fn build_apply_plan(
    env: &ProjectEnvironment,
    target: EnvironmentTargetBinding,
    requested: &[String],
    observations: &[MachineObservation],
) -> Result<EnvironmentApplyPlan, DaemonError> {
    let selected = apply_selection(env, requested)?;
    let mut conflicts = vec![];
    let mut unready = BTreeSet::new();
    for id in &selected {
        let r = environment_requirements(env)
            .find(|r| &r.requirement_id == id)
            .expect("validated selection");
        let observed = observations.iter().find(|o| {
            o.requirement_id == *id
                && o.target == target
                && o.revision_digest == env.content_digest
                && !o.stale
        });
        if environment_dependencies(r)
            .iter()
            .any(|d| unready.contains(d))
        {
            conflicts.push(apply_item_result(
                r,
                EnvironmentObservationStatus::Conflict,
                "dependency_blocked",
                "A prerequisite cannot be applied; this item stays untouched",
            ));
            unready.insert(id.clone());
        } else if observed.is_some_and(|o| o.status == EnvironmentObservationStatus::Conflict) {
            let o = observed.unwrap();
            conflicts.push(apply_item_result(
                r,
                o.status.clone(),
                &o.reason_code,
                &o.safe_summary,
            ));
            unready.insert(id.clone());
        } else if !observed.is_some_and(|o| o.status == EnvironmentObservationStatus::Ready)
            && !setup_opt_in(r)
        {
            // Missing software/services are Check-only in P05a. Scoped bindings
            // require a resolvable locator; Apply never invents a value or grant.
            unready.insert(id.clone());
        }
    }
    Ok(EnvironmentApplyPlan {
        plan_id: format!("apply-plan-{:032x}", rand::random::<u128>()),
        project_id: env.local_project_id.clone(),
        revision_digest: env.content_digest.clone(),
        target,
        selected_items: selected.clone(),
        required_opt_ins: environment_requirements(env)
            .filter(|r| selected.contains(&r.requirement_id) && setup_opt_in(r))
            .map(|r| r.requirement_id.clone())
            .collect(),
        conflicts,
        expires_at_ms: crate::session::unix_epoch_ms() + 300_000,
    })
}
