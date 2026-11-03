//! MP-08 / MP-10 / MP-11: production Check/Plan/Apply concurrency with a bounded probe gate.
use super::*;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review0034_check_preserves_concurrent_apply_observation_and_drift_baseline() {
    use std::io::{Read, Write};
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("check-apply-race");
    std::env::set_var("PUBLIC_REVIEW_SETTING", "first-public-value");
    let config = crate::config::DaemonConfig::for_tests();
    let app = DaemonApp::bootstrap(config.clone()).unwrap();
    let state = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        1,
    )
    .runtime_state();
    let project = crate::session::RuntimeProject::new(
        "concurrency",
        "owner",
        root.path().to_string_lossy(),
        "Concurrency",
        crate::session::RuntimeProjectKind::Named,
    );
    state
        .owned
        .session_store
        .restore_projects(vec![project.clone()]);
    let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
    let current = store.snapshot(&project).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let (entered_tx, entered) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let end = Instant::now() + Duration::from_secs(15);
        let mut stream = loop {
            if let Ok((stream, _)) = listener.accept() {
                break stream;
            }
            if Instant::now() > end {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let _ = stream.read(&mut [0; 4096]);
        let _ = entered_tx.send(());
        let _ = release_rx.recv_timeout(Duration::from_secs(15));
        let _ =
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    });
    let requirement = |id: &str, spec: RequirementSpec| Requirement {
        requirement_id: id.into(),
        title: id.into(),
        scope: RequirementScope::Project,
        origins: vec![],
        spec,
        depends_on: vec![],
        platform_variants: vec![],
        required: true,
        legacy_entry: None,
    };
    let mut draft = environment_draft(&current);
    draft.project_requirements = vec![
        requirement(
            "x",
            RequirementSpec::Variables {
                name: "PUBLIC_REVIEW_SETTING".into(),
                locator: ProjectEnvironmentLocator::WorkspaceEnvironment {
                    name: "PUBLIC_REVIEW_SETTING".into(),
                },
            },
        ),
        requirement(
            "y",
            RequirementSpec::Services {
                identity: "Bounded slow service".into(),
                target_binding: None,
                probe: Some(EnvironmentProbe::Http {
                    url,
                    expected_status: 200,
                    timeout_ms: 15000,
                }),
                health_expectation: None,
                depends_on: vec![],
            },
        ),
    ];
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
    state
        .check_project_environment(
            crate::local::CheckProjectEnvironmentRequest {
                project_id: project.id().into(),
                revision_digest: env.content_digest.clone(),
                target: target.clone(),
                operation_id: "slow-check-y".into(),
                selected_items: vec!["y".into()],
            },
            "owner",
        )
        .await
        .unwrap();
    tokio::task::spawn_blocking(move || entered.recv_timeout(Duration::from_secs(10)).unwrap())
        .await
        .unwrap();
    let LocalDaemonResponse::ProjectEnvironmentPlan { plan } = state
        .plan_project_environment(
            crate::local::PlanProjectEnvironmentRequest {
                project_id: project.id().into(),
                expected_revision: env.revision,
                revision_digest: env.content_digest.clone(),
                target: target.clone(),
                selected_items: vec!["x".into()],
            },
            "owner",
        )
        .await
        .unwrap()
    else {
        panic!("plan expected")
    };
    state
        .apply_project_environment(
            crate::local::ApplyProjectEnvironmentRequest {
                project_id: project.id().into(),
                expected_revision: env.revision,
                revision_digest: env.content_digest.clone(),
                target: target.clone(),
                operation_id: "concurrent-apply-x".into(),
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
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if store
            .load_applies(project.id())
            .unwrap()
            .attempts
            .iter()
            .any(|a| {
                a.operation.operation_id == "concurrent-apply-x" && !apply_active(&a.operation)
            })
        {
            break;
        }
        assert!(Instant::now() < end, "Apply settles while Y is probing");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let before = store.load_checks(project.id()).unwrap();
    assert_eq!(
        before
            .observations
            .iter()
            .find(|o| o.requirement_id == "x")
            .unwrap()
            .check_operation_id
            .as_deref(),
        Some("concurrent-apply-x")
    );
    let _ = release.send(());
    server.join().unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if store
            .load_checks(project.id())
            .unwrap()
            .operations
            .iter()
            .any(|o| {
                o.operation_id == "slow-check-y"
                    && o.recovery_state == EnvironmentRecoveryState::Settled
            })
        {
            break;
        }
        assert!(Instant::now() < end, "Check settles");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let after = store.load_checks(project.id()).unwrap();
    assert_eq!(
        after.public_bindings, before.public_bindings,
        "Check must retain first baseline published by Plan/Apply"
    );
    assert_eq!(
        after.observations.iter().find(|o| o.requirement_id == "x"),
        before.observations.iter().find(|o| o.requirement_id == "x"),
        "Check Y must retain newer Apply X observation"
    );
    std::env::set_var("PUBLIC_REVIEW_SETTING", "second-public-value");
    let mut baseline = after.public_bindings;
    let ctx = EnvironmentCheckContext {
        config,
        owner: "owner".into(),
        profiles: state.owned.provider_account_profiles.clone(),
        apps: state.owned.durable_state_store.clone(),
    };
    let changed = check_requirement(
        &ctx,
        &env,
        &env.project_requirements[0],
        &target,
        "drift",
        &mut baseline,
        Instant::now() + Duration::from_secs(5),
    );
    assert_eq!(changed.reason_code, "binding_drift");
    std::env::remove_var("PUBLIC_REVIEW_SETTING");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn r4_apply_probe_releases_project_lock_for_cancellation_and_preview() {
    use std::io::{Read, Write};
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("r4-apply-probe");
    let config = crate::config::DaemonConfig::for_tests();
    let state = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(DaemonApp::bootstrap(config.clone()).unwrap())),
        1,
    )
    .runtime_state();
    let project = crate::session::RuntimeProject::new(
        "slow-apply",
        "owner",
        root.path().to_string_lossy(),
        "Slow Apply",
        crate::session::RuntimeProjectKind::Named,
    );
    state
        .owned
        .session_store
        .restore_projects(vec![project.clone()]);
    let store = ProjectEnvironmentStore::new(&config.private_runtime_state_root());
    let current = store.snapshot(&project).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let (entered_tx, entered) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        for index in 0..2 {
            let end = Instant::now() + Duration::from_secs(15);
            let mut stream = loop {
                if let Ok((stream, _)) = listener.accept() {
                    break stream;
                }
                if Instant::now() > end {
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let _ = stream.read(&mut [0; 4096]);
            if index == 1 {
                let _ = entered_tx.send(());
                let _ = release_rx.recv_timeout(Duration::from_secs(10));
            }
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        }
    });
    let mut draft = environment_draft(&current);
    draft.project_requirements.push(Requirement {
        requirement_id: "service".into(),
        title: "Slow service".into(),
        scope: RequirementScope::Project,
        origins: vec![],
        spec: RequirementSpec::Services {
            identity: "slow-service".into(),
            target_binding: None,
            probe: Some(EnvironmentProbe::Http {
                url,
                expected_status: 200,
                timeout_ms: 5000,
            }),
            health_expectation: None,
            depends_on: vec![],
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
        panic!("Plan expected")
    };
    state
        .apply_project_environment(
            crate::local::ApplyProjectEnvironmentRequest {
                project_id: project.id().into(),
                expected_revision: env.revision,
                revision_digest: env.content_digest.clone(),
                target,
                operation_id: "slow-probe".into(),
                plan_id: plan.plan_id,
                selected_items: plan.selected_items,
                per_item_opt_ins: vec![],
            },
            "owner",
        )
        .await
        .unwrap();
    tokio::task::spawn_blocking(move || entered.recv_timeout(Duration::from_secs(10)).unwrap())
        .await
        .unwrap();
    let began = Instant::now();
    let (cancelled, previewed) = tokio::join!(
        state.cancel_environment_operation(
            crate::local::EnvironmentOperationRequest {
                project_id: project.id().into(),
                operation_id: "slow-probe".into()
            },
            "owner"
        ),
        state.preview_environment_diff(
            crate::local::PreviewEnvironmentDiffRequest {
                project_id: project.id().into(),
                expected_revision: env.revision,
                draft: environment_draft(&env)
            },
            "owner"
        ),
    );
    let action_elapsed = began.elapsed();
    // Keep the admitted probe pending longer than lock_briefly_async's three-second budget.
    tokio::time::sleep(Duration::from_millis(3500).saturating_sub(began.elapsed())).await;
    let _ = release.send(());
    server.join().unwrap();
    let end = Instant::now() + Duration::from_secs(10);
    let operation = loop {
        let journal = store.load_applies(project.id()).unwrap();
        let operation = journal.attempts.last().unwrap().operation.clone();
        if !apply_active(&operation) {
            break operation;
        }
        assert!(
            Instant::now() < end,
            "Apply must settle after its probe returns"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert!(
        cancelled.is_ok(),
        "Cancel must record cancellation during a probe: {cancelled:?}"
    );
    assert!(
        previewed.is_ok(),
        "Preview must stay available during a probe: {previewed:?}"
    );
    assert!(action_elapsed < Duration::from_secs(3));
    assert_eq!(operation.phase, EnvironmentOperationPhase::Cancelled);
    assert!(operation.cancellation.unwrap().settled_at_ms.is_some());
    assert!(
        operation.receipts.is_empty(),
        "late probe facts cannot publish a Ready receipt after cancellation"
    );
    assert!(!store
        .load_checks(project.id())
        .unwrap()
        .observations
        .iter()
        .any(|o| o.check_operation_id.as_deref() == Some("slow-probe")));
}
