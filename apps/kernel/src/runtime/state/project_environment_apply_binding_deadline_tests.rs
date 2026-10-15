//! MP-08 / MP-10 / MP-11: public Apply admission and publication race regression checks.
use super::*;
use crate::project_environment::*;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review0940_binding_publication_deadline_settles_without_metadata() {
    crate::test_support::isolated_env_test!();
    exercise(false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review0940_binding_publication_can_cancel_without_project_lock() {
    crate::test_support::isolated_env_test!();
    exercise(true, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review0940_binding_publication_revalidates_revision_after_read() {
    crate::test_support::isolated_env_test!();
    exercise(false, true).await;
}
async fn exercise(cancel: bool, edit: bool) {
    let _guard = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("apply-binding-deadline");
    std::fs::write(
        root.path().join("settings"),
        "PUBLIC_SETTING=public-value\n",
    )
    .unwrap();
    let config = crate::config::DaemonConfig::for_tests();
    let state = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(DaemonApp::bootstrap(config.clone()).unwrap())),
        1,
    )
    .runtime_state();
    let project = crate::session::RuntimeProject::new(
        "publication",
        "owner",
        root.path().to_string_lossy(),
        "Publication",
        crate::session::RuntimeProjectKind::Named,
    );
    state
        .owned
        .session_store
        .restore_projects(vec![project.clone()]);
    let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
    let current = store.snapshot(&project).unwrap();
    let mut draft = environment_draft(&current);
    draft.project_requirements.push(Requirement {
        requirement_id: "public".into(),
        title: "Public input".into(),
        scope: RequirementScope::Project,
        origins: vec![],
        spec: RequirementSpec::Variables {
            name: "PUBLIC_SETTING".into(),
            locator: ProjectEnvironmentLocator::EnvFile {
                path: "settings".into(),
                key: "PUBLIC_SETTING".into(),
            },
        },
        depends_on: vec![],
        platform_variants: vec![],
        required: true,
        legacy_entry: None,
    });
    let env = store
        .save_revision_locked(
            &current,
            &crate::local::SaveProjectEnvironmentRevisionRequest {
                project_id: project.id().into(),
                expected_revision: current.revision,
                expected_content_digest: current.content_digest.clone(),
                draft,
                accepted_proposal_ids: vec![],
                excluded_proposal_ids: vec![],
            },
            "owner",
        )
        .unwrap()
        .0;
    let target = current_check_target(&config);
    let LocalDaemonResponse::ProjectEnvironmentPlan { plan } = state
        .plan_project_environment(
            crate::local::PlanProjectEnvironmentRequest {
                project_id: project.id().into(),
                expected_revision: env.revision,
                revision_digest: env.content_digest.clone(),
                target: target.clone(),
                selected_items: vec![],
            },
            "owner",
        )
        .await
        .unwrap()
    else {
        panic!("Plan")
    };
    let marker = root.path().join("publication-reached");
    std::env::set_var("CHARIOX_APPLY_BINDING_STALL_MARKER", &marker);
    std::env::set_var(
        "CHARIOX_APPLY_BINDING_BUDGET_MS",
        if cancel || edit { "8000" } else { "500" },
    );
    let began = Instant::now();
    state
        .apply_project_environment(
            crate::local::ApplyProjectEnvironmentRequest {
                project_id: project.id().into(),
                expected_revision: env.revision,
                revision_digest: env.content_digest.clone(),
                target,
                operation_id: "publication".into(),
                plan_id: plan.plan_id,
                selected_items: plan.selected_items,
                per_item_opt_ins: plan
                    .required_opt_ins
                    .into_iter()
                    .map(|id| EnvironmentItemOptIn {
                        requirement_id: id,
                        decision: EnvironmentOptInDecision::Accepted,
                        user_id: "owner".into(),
                        recorded_at_ms: crate::session::unix_epoch_ms(),
                    })
                    .collect(),
            },
            "owner",
        )
        .await
        .unwrap();
    let end = Instant::now() + Duration::from_secs(15);
    while !marker.exists() && Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(10)).await
    }
    let reached = marker.exists();
    let mut action_ok = true;
    let mut action_elapsed = Duration::ZERO;
    if reached && cancel {
        let at = Instant::now();
        action_ok = state
            .cancel_environment_operation(
                crate::local::EnvironmentOperationRequest {
                    project_id: project.id().into(),
                    operation_id: "publication".into(),
                },
                "owner",
            )
            .await
            .is_ok();
        action_elapsed = at.elapsed();
    } else if reached && edit {
        action_ok = match store.lock_briefly_async(project.id()).await {
            Ok(_lock) => {
                let mut draft = environment_draft(&env);
                draft.project_requirements[0].title = "Edited while reading".into();
                store
                    .save_revision_locked(
                        &env,
                        &crate::local::SaveProjectEnvironmentRevisionRequest {
                            project_id: project.id().into(),
                            expected_revision: env.revision,
                            expected_content_digest: env.content_digest.clone(),
                            draft,
                            accepted_proposal_ids: vec![],
                            excluded_proposal_ids: vec![],
                        },
                        "owner",
                    )
                    .is_ok()
            }
            Err(_) => false,
        };
        std::fs::write(marker.with_extension("release"), b"").unwrap();
    }
    loop {
        let journal = store.load_applies(project.id()).unwrap();
        if journal.attempts.iter().any(|a| !apply_active(&a.operation)) {
            break;
        }
        if Instant::now() >= end {
            std::fs::write(marker.with_extension("release"), b"").unwrap();
            panic!("Apply failed to settle")
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    std::env::remove_var("CHARIOX_APPLY_BINDING_STALL_MARKER");
    std::env::remove_var("CHARIOX_APPLY_BINDING_BUDGET_MS");
    assert!(
        reached,
        "fresh publication reader reached after successful Check"
    );
    assert!(
        action_ok,
        "Cancel/Save must acquire Project lock during fresh read"
    );
    if cancel {
        assert!(
            action_elapsed < Duration::from_secs(2),
            "Cancel blocked: {action_elapsed:?}"
        )
    }
    if !cancel && !edit {
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "publication escaped its owned deadline"
        )
    }
    let journal = store.load_applies(project.id()).unwrap();
    let op = &journal.attempts.last().unwrap().operation;
    assert!(
        op.receipts.is_empty(),
        "no binding receipt after cancelled, stale or unproven read"
    );
    if cancel {
        assert_eq!(op.phase, EnvironmentOperationPhase::Cancelled)
    }
    assert!(
        store
            .load(project.id())
            .unwrap()
            .is_none_or(|s| s.manifest.entries.is_empty()),
        "no binding metadata published"
    );
}
