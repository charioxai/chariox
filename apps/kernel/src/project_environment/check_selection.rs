//! MP-08 / MP-10 / MP-11: saved selection policy shared by Check and planning.
use super::*;

pub(crate) fn check_selection(
    env: &ProjectEnvironment,
    requested: &[String],
) -> Result<Vec<String>, crate::error::DaemonError> {
    let requirements: Vec<_> = env
        .project_requirements
        .iter()
        .chain(env.folders.iter().flat_map(|f| &f.requirements))
        .collect();
    if requested
        .iter()
        .any(|id| !requirements.iter().any(|r| &r.requirement_id == id))
    {
        return Err(environment_error(
            "Check selection does not belong to the saved revision",
        ));
    }
    let mut pending: Vec<_> = requirements
        .iter()
        .filter(|r| {
            if requested.is_empty() {
                r.required
            } else {
                requested.contains(&r.requirement_id)
            }
        })
        .map(|r| r.requirement_id.clone())
        .collect();
    let mut selected = std::collections::BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !selected.insert(id.clone()) {
            continue;
        }
        let Some(requirement) = requirements.iter().find(|r| r.requirement_id == id) else {
            return Err(environment_error(
                "Check dependency does not belong to the saved revision",
            ));
        };
        pending.extend(requirement.depends_on.iter().cloned());
        if let RequirementSpec::Services { depends_on, .. } = &requirement.spec {
            pending.extend(depends_on.iter().cloned());
        }
    }
    // Probe each prerequisite once in saved order, without selecting unrelated optional items.
    Ok(requirements
        .into_iter()
        .filter(|r| selected.contains(&r.requirement_id))
        .map(|r| r.requirement_id.clone())
        .collect())
}
