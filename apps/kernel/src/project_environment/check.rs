//! MP-08 / MP-10 / MP-11: named Check policy; values and opaque command execution stay out of observations.
use super::check_inventory as inventory;
use super::*;
use crate::config::DaemonConfig;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;
use zeroize::Zeroizing;

pub(crate) struct EnvironmentCheckContext {
    pub config: DaemonConfig,
    pub owner: String,
    pub profiles: crate::account_profile::ProviderAccountProfileRegistry,
    pub apps: crate::durable_state::DurableKernelStateStore,
}
pub(super) fn outcome(
    status: EnvironmentObservationStatus,
    reason: &'static str,
    summary: &'static str,
) -> (EnvironmentObservationStatus, String, String) {
    (status, reason.into(), summary.into())
}
use EnvironmentObservationStatus::{Missing, NeedsYourInput, Ready};
pub(super) fn vault_available(config: &DaemonConfig, refs: &[EnvironmentVaultReference]) -> bool {
    let Ok(vault) = crate::secret::project_environment_vault(config) else {
        return false;
    };
    refs.iter().all(|v| {
        vault
            .find_secret(&v.service, &v.key)
            .ok()
            .flatten()
            .map(Zeroizing::new)
            .is_some_and(|v| !v.is_empty())
    })
}
pub(super) fn workspace<'a>(env: &'a ProjectEnvironment, r: &Requirement) -> Option<&'a Path> {
    match &r.scope {
        RequirementScope::Folder { folder_id } => env
            .folders
            .iter()
            .find(|f| &f.folder_id == folder_id && !f.local_workspace_binding.is_empty())
            .map(|f| Path::new(&f.local_workspace_binding)),
        RequirementScope::Project => env
            .folders
            .first()
            .filter(|f| !f.local_workspace_binding.is_empty())
            .map(|f| Path::new(&f.local_workspace_binding)),
    }
}
pub(crate) fn check_requirement(
    ctx: &EnvironmentCheckContext,
    env: &ProjectEnvironment,
    r: &Requirement,
    target: &EnvironmentTargetBinding,
    operation: &str,
    baseline: &mut BTreeMap<String, String>,
    deadline: Instant,
) -> MachineObservation {
    check_requirement_in_workspace(
        ctx,
        env,
        r,
        target,
        operation,
        baseline,
        deadline,
        &mut super::check_workspace::CheckWorkspace::default(),
    )
}
// Keep the operation identity, deadline and mutable probe state explicit.
#[allow(clippy::too_many_arguments)]
pub(crate) fn check_requirement_in_workspace(
    ctx: &EnvironmentCheckContext,
    env: &ProjectEnvironment,
    r: &Requirement,
    target: &EnvironmentTargetBinding,
    operation: &str,
    baseline: &mut BTreeMap<String, String>,
    deadline: Instant,
    workspace: &mut super::check_workspace::CheckWorkspace,
) -> MachineObservation {
    let mut facts = inventory::facts();
    let budget = || {
        outcome(
            NeedsYourInput,
            "check_budget_exceeded",
            "Check reached its bounded probe budget; select fewer items and check again",
        )
    };
    let result = (|| {
        if Instant::now() >= deadline {
            return budget();
        }
        if !r.platform_variants.is_empty()
            && !r.platform_variants.iter().any(|p| {
                facts
                    .platform
                    .as_ref()
                    .is_some_and(|a| inventory::platform_matches(p, a))
            })
        {
            return outcome(
                Missing,
                "platform_gap",
                "This requirement is unavailable on this platform",
            );
        }
        let (roots, folder_accessible) = match workspace.for_requirement(&ctx.config, env, r, deadline) {
            Ok(measured) => measured,
            Err(()) => return outcome(NeedsYourInput, "workspace_preflight_unavailable", "Workspace metadata could not be checked within the probe budget; retry when the folder is available"),
        };
        if !folder_accessible {
            return outcome(
                Missing,
                "folder_unavailable",
                "The required folder is absent or inaccessible",
            );
        }
        match &r.spec {
            RequirementSpec::Files { .. } => super::check_assets::check_assets(
                ctx, env, r, target, baseline, &mut facts, deadline,
            ),
            RequirementSpec::Software { .. } => {
                super::check_platform::check_platform(ctx, env, r, &roots, &mut facts, deadline)
            }
            RequirementSpec::Services {
                target_binding,
                probe,
                health_expectation,
                ..
            } => {
                if target_binding.as_ref().is_some_and(|t| t != target) {
                    return outcome(
                        Missing,
                        "service_target_gap",
                        "Service binding is for another target generation",
                    );
                }
                let Some(p) = probe else {
                    return outcome(
                        NeedsYourInput,
                        "health_probe_required",
                        "A named service health probe is required; a PID never proves health",
                    );
                };
                match inventory::health_until(p,health_expectation,&roots,deadline){Ok(true)=>outcome(Ready,"service_healthy","Named service health probe passed; Check does not start or restart services"),Ok(false)=>outcome(Missing,"service_unhealthy","Service health probe failed; no repair performed"),Err("worker_required")=>outcome(NeedsYourInput,"worker_required","Executable health script requires confirmed disposable-worker execution; no host command ran"),Err(_)=>outcome(NeedsYourInput,"probe_unproven","Named health probe is unavailable or invalid; review its target and probe")}
            }
            RequirementSpec::Variables { .. } => super::check_assets::check_assets(
                ctx, env, r, target, baseline, &mut facts, deadline,
            ),
            RequirementSpec::Secrets { .. } => super::check_assets::check_assets(
                ctx, env, r, target, baseline, &mut facts, deadline,
            ),
            RequirementSpec::MachineNeeds { .. } => {
                super::check_platform::check_platform(ctx, env, r, &roots, &mut facts, deadline)
            }
            RequirementSpec::Accounts { .. } => {
                super::check_admission::check_admission(ctx, r, deadline)
            }
            RequirementSpec::AgentTools { .. } => {
                super::check_tools::check_tool(ctx, r, &roots, operation, deadline)
            }
            RequirementSpec::CharioxApps { .. } => {
                super::check_admission::check_admission(ctx, r, deadline)
            }
            RequirementSpec::SetupChecks {
                commands,
                probes,
                legacy,
                ..
            } => {
                if !commands.is_empty() || legacy.is_some() {
                    return outcome(NeedsYourInput,"worker_required","Opaque setup/check requires the confirmed worker boundary; legacy readiness is not current evidence");
                }
                if probes.is_empty() {
                    return outcome(
                        NeedsYourInput,
                        "check_probe_required",
                        "No named check probe is declared",
                    );
                }
                for p in probes {
                    if Instant::now() >= deadline {
                        return budget();
                    }
                    match inventory::health_until(p, &None, &roots, deadline) {
                        Ok(true) => {}
                        Ok(false) => {
                            return outcome(
                                Missing,
                                "check_failed",
                                "Named check probe failed; no repair performed",
                            )
                        }
                        Err(_) => {
                            return outcome(
                                NeedsYourInput,
                                "worker_required",
                                "Opaque check requires confirmed disposable-worker execution",
                            )
                        }
                    }
                }
                outcome(
                    Ready,
                    "checks_passed",
                    "All declared named probes passed; no Setup command ran",
                )
            }
        }
    })();
    let result = if Instant::now() >= deadline {
        budget()
    } else {
        result
    };
    let stale = result.1 == "check_budget_exceeded";
    MachineObservation {
        environment_id: env.lineage.environment_id.clone(),
        requirement_id: r.requirement_id.clone(),
        target: target.clone(),
        revision_digest: env.content_digest.clone(),
        status: result.0,
        checked_at_ms: Some(crate::session::unix_epoch_ms()),
        measured_facts: facts,
        reason_code: result.1,
        safe_summary: result.2,
        check_operation_id: Some(operation.into()),
        stale,
    }
}

pub(crate) fn empty_environment_facts() -> EnvironmentMeasuredFacts {
    inventory::facts()
}
pub(crate) fn invalidate_check_dependencies(
    env: &ProjectEnvironment,
    observations: &mut [MachineObservation],
    target: &EnvironmentTargetBinding,
    operation: Option<&str>,
) {
    let blocked = super::dependency_graph::blocked(
        env.project_requirements
            .iter()
            .chain(env.folders.iter().flat_map(|folder| &folder.requirements)),
    );
    for _ in 0..observations.len() {
        let good: std::collections::BTreeSet<_> = observations
            .iter()
            .filter(|o| {
                o.target == *target
                    && o.revision_digest == env.content_digest
                    && !o.stale
                    && o.status == Ready
            })
            .map(|o| o.requirement_id.clone())
            .collect();
        let mut changed = false;
        for o in observations.iter_mut().filter(|o| {
            o.target == *target && o.revision_digest == env.content_digest && o.status == Ready
        }) {
            let Some(r) = env
                .project_requirements
                .iter()
                .chain(env.folders.iter().flat_map(|f| &f.requirements))
                .find(|r| r.requirement_id == o.requirement_id)
            else {
                continue;
            };
            let mut dependencies = r.depends_on.clone();
            if let RequirementSpec::Services { depends_on, .. } = &r.spec {
                dependencies.extend(depends_on.iter().cloned());
            }
            let cyclic = blocked.contains(r.requirement_id.as_str());
            if cyclic || dependencies.iter().any(|id| !good.contains(id)) {
                o.stale |= operation
                    .is_none_or(|operation| o.check_operation_id.as_deref() != Some(operation));
                o.status = NeedsYourInput;
                o.reason_code = if cyclic {
                    "dependency_cycle"
                } else {
                    "dependency_unready"
                }
                .into();
                o.safe_summary = if cyclic {
                    "A dependency cycle or absent prerequisite prevents readiness; review the dependencies"
                } else {
                    "A required dependency is missing, stale or not checked; no repair performed"
                }.into();
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}
