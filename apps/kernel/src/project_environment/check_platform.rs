//! MP-08 / MP-10 / MP-11: named platform probes; Check does not repair or authorize.
use super::check::{outcome, workspace, EnvironmentCheckContext};
use super::check_inventory as inventory;
use super::*;
use std::path::PathBuf;
use EnvironmentObservationStatus::{Conflict, Missing, NeedsYourInput, Outdated, Ready};
pub(super) fn check_platform(
    ctx: &EnvironmentCheckContext,
    env: &ProjectEnvironment,
    r: &Requirement,
    roots: &[PathBuf],
    facts: &mut EnvironmentMeasuredFacts,
    deadline: std::time::Instant,
) -> (EnvironmentObservationStatus, String, String) {
    match &r.spec {
        RequirementSpec::Software {
            identity,
            version_constraint,
            platform,
            install_source,
            ..
        } => {
            if platform.as_ref().is_some_and(|p| {
                !facts
                    .platform
                    .as_ref()
                    .is_some_and(|a| inventory::platform_matches(p, a))
            }) {
                return outcome(
                    Missing,
                    "platform_gap",
                    "Software is unavailable on this platform; GUI programs are Check-only in V1",
                );
            }
            if let Some(_name) = install_source
                .is_none()
                .then(|| super::check_gui_identity::application_name(identity))
                .flatten()
            {
                #[cfg(target_os = "macos")]
                {
                    let present = PathBuf::from("/Applications")
                        .join(format!("{}.app", _name))
                        .is_dir()
                        || std::env::var_os("HOME").is_some_and(|h| {
                            PathBuf::from(h)
                                .join("Applications")
                                .join(format!("{}.app", _name))
                                .is_dir()
                        });
                    facts.present = Some(present);
                    return if present && version_constraint.is_none() {
                        outcome(Ready,"gui_present","Desktop program is present; Check-only in V1; sign in inside the program")
                    } else if present {
                        outcome(
                            NeedsYourInput,
                            "gui_version_unverified",
                            "Desktop version needs an actual platform probe; Check-only in V1",
                        )
                    } else {
                        outcome(
                            Missing,
                            "gui_absent",
                            "Desktop program is absent; Check-only in V1; no substitute installed",
                        )
                    };
                }
                #[cfg(not(target_os = "macos"))]
                {
                    facts.present = Some(false);
                    return outcome(Missing,"gui_platform_gap","Desktop program is absent or unsupported on this platform; Check-only in V1");
                }
            }
            let version = if install_source.as_deref() == Some("npm package") {
                super::check_files::npm(&ctx.config, workspace(env, r), identity, deadline)
            } else {
                inventory::software_version_until(
                    identity,
                    install_source.as_deref(),
                    workspace(env, r),
                    roots,
                    deadline,
                )
            };
            match version {
                Ok(Some(v)) => {
                    facts.present = Some(true);
                    facts.version = Some(v.clone());
                    match version_constraint
                        .as_deref()
                        .map(|c| inventory::satisfies(&v, c))
                    {
                        Some(Some(false)) => {
                            let owned = inventory::executable(identity)
                                .and_then(|p| p.canonicalize().ok())
                                .is_some_and(|p| {
                                    p.starts_with(ctx.config.private_runtime_state_root())
                                });
                            outcome(if owned{Outdated}else{Conflict},"version_mismatch","Installed version does not meet the requirement; Check never replaces or upgrades it")
                        }
                        Some(None) => outcome(
                            NeedsYourInput,
                            "version_constraint_unsupported",
                            "Version constraint requires review; readiness is unproven",
                        ),
                        _ => outcome(Ready, "software_present", "Named inventory probe passed"),
                    }
                }
                Ok(None) => {
                    facts.present = Some(false);
                    outcome(Missing, "software_absent", "Required executable is absent")
                }
                Err(_) => outcome(
                    NeedsYourInput,
                    "inventory_unproven",
                    "No safe successful named inventory probe; review this Software requirement",
                ),
            }
        }
        RequirementSpec::MachineNeeds {
            platforms,
            minimum_cpus,
            minimum_memory_bytes,
            minimum_free_disk_bytes,
            gpu,
        } => {
            // A capacity syscall can block independently of directory preflight.
            // CPU/memory-only requirements never query the workspace filesystem.
            let disk_root = minimum_free_disk_bytes.map(|_| {
                workspace(env, r)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| ctx.config.private_runtime_state_root())
            });
            *facts =
                match super::check_files::machine(&ctx.config, disk_root.as_deref(), deadline) {
                    Ok(measured) => measured,
                    Err(_) => return outcome(
                        NeedsYourInput,
                        "capacity_unproven",
                        "Required machine capacity could not be measured within the probe budget",
                    ),
                };
            if !platforms.is_empty()
                && !platforms.iter().any(|p| {
                    facts
                        .platform
                        .as_ref()
                        .is_some_and(|a| inventory::platform_matches(p, a))
                })
            {
                return outcome(
                    Missing,
                    "platform_gap",
                    "Target platform does not meet Machine needs; no provisioning performed",
                );
            }
            for (need, actual) in [
                (minimum_cpus.map(u64::from), facts.cpus.map(u64::from)),
                (*minimum_memory_bytes, facts.memory_bytes),
                (*minimum_free_disk_bytes, facts.free_disk_bytes),
            ] {
                if let Some(need) = need {
                    let Some(actual) = actual else {
                        return outcome(
                            NeedsYourInput,
                            "capacity_unproven",
                            "Required capacity could not be measured; no provisioning performed",
                        );
                    };
                    if actual < need {
                        return outcome(Missing,"capacity_gap","Measured target capacity does not meet Machine needs; no resize or billing action performed");
                    }
                }
            }
            if gpu.is_some() {
                return outcome(
                    if facts.gpu.is_none() {
                        Missing
                    } else {
                        NeedsYourInput
                    },
                    "gpu_unproven",
                    "GPU presence alone does not prove the required GPU/runtime suitability",
                );
            }
            outcome(
                Ready,
                "machine_suitable",
                "Measured target platform and capacity meet Machine needs",
            )
        }
        _ => unreachable!("named Check dispatch"),
    }
}
