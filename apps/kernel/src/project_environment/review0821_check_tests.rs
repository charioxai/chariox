//! MP-08 / MP-10 / MP-11: review regressions supplement hosted user-flow drills.
use super::*;
fn context(root: &std::path::Path) -> EnvironmentCheckContext {
    EnvironmentCheckContext {
        config: crate::config::DaemonConfig::for_tests(),
        owner: "owner".into(),
        profiles: crate::account_profile::ProviderAccountProfileRegistry::open(
            root.join("profiles"),
        )
        .unwrap(),
        apps: crate::durable_state::DurableKernelStateStore::open(root.join("apps.db")).unwrap(),
    }
}
fn operation(
    env: &ProjectEnvironment,
    target: &EnvironmentTargetBinding,
    selected: Vec<&str>,
) -> EnvironmentOperation {
    EnvironmentOperation {
        operation_id: "review0212-check".into(),
        attempt: 1,
        local_project_id: env.local_project_id.clone(),
        revision_digest: env.content_digest.clone(),
        target: target.clone(),
        kind: EnvironmentOperationKind::Check,
        phase: EnvironmentOperationPhase::Requested,
        selected_items: selected.into_iter().map(str::to_owned).collect(),
        per_item_opt_ins: vec![],
        per_item_results: vec![],
        created_at_ms: crate::session::unix_epoch_ms(),
        updated_at_ms: crate::session::unix_epoch_ms(),
        cancellation: None,
        receipts: vec![],
        recovery_state: EnvironmentRecoveryState::Pending,
    }
}
fn project(root: &std::path::Path, id: &str) -> crate::session::RuntimeProject {
    crate::session::RuntimeProject::new(
        id,
        "owner",
        root.to_str().unwrap(),
        "Review",
        crate::session::RuntimeProjectKind::Named,
    )
}

#[path = "review0821_budget_tests.rs"]
mod budget;
#[path = "review0821_dns_tests.rs"]
mod dns;
#[path = "review0821_excluded_tests.rs"]
mod excluded;
#[path = "review0821_wildcard_tests.rs"]
mod wildcard;
