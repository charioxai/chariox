//! MP-08 / MP-10: the actual Detect reply preserves the current Check journal.
use super::*;
use crate::project_environment::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn review0940_checked_requirements_remain_ready_in_detect_reply_and_get() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = crate::test_support::TestWorktree::new("detect-checked");
    std::fs::write(
        root.path().join("README.md"),
        "Public project documentation.\n",
    )
    .unwrap();
    let config = crate::config::DaemonConfig::for_tests();
    let app = DaemonApp::bootstrap(config.clone()).unwrap();
    let state = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(app)),
        1,
    )
    .runtime_state();
    let project = crate::session::RuntimeProject::new(
        "detect-checked",
        "owner",
        root.path().to_string_lossy(),
        "Checked",
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
        requirement_id: "cpu".into(),
        title: "CPU".into(),
        scope: RequirementScope::Project,
        origins: vec![],
        spec: RequirementSpec::MachineNeeds {
            platforms: vec![],
            minimum_cpus: Some(1),
            minimum_memory_bytes: None,
            minimum_free_disk_bytes: None,
            gpu: None,
        },
        depends_on: vec![],
        platform_variants: vec![],
        required: true,
        legacy_entry: None,
    });
    let saved = store
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
                revision_digest: saved.content_digest,
                operation_id: "check-before-detect".into(),
                selected_items: vec![],
                target: target.clone(),
            },
            "owner",
        )
        .await
        .unwrap();
    let read = || {
        state.get_project_environment(
            crate::local::GetProjectEnvironmentRequest {
                project_id: project.id().into(),
                view: None,
            },
            "owner",
        )
    };
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let LocalDaemonResponse::ProjectEnvironment { environment } = read().await.unwrap() else {
            panic!("full read")
        };
        if environment.operations.iter().any(|o| {
            o.operation_id == "check-before-detect" && o.phase == EnvironmentOperationPhase::Ready
        }) {
            assert_eq!(
                environment.observations[0].status,
                EnvironmentObservationStatus::Ready
            );
            break;
        }
        assert!(std::time::Instant::now() < end, "real Check must settle");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    for id in ["detect-fresh", "detect-unchanged"] {
        let LocalDaemonResponse::ProjectEnvironment {
            environment: detected,
        } = state
            .detect_project_environment(
                crate::local::DetectProjectEnvironmentRequest {
                    project_id: project.id().into(),
                    operation_id: id.into(),
                    folder_ids: vec![],
                    provider: None,
                    allow_model_folders: vec![],
                    target: EnvironmentTargetBinding {
                        machine_id: config.host_machine_id.clone(),
                        target_instance_generation: config.daemon_id.clone(),
                        slice_ref: None,
                    },
                },
                "owner",
            )
            .await
            .unwrap()
        else {
            panic!("Detect response")
        };
        let LocalDaemonResponse::ProjectEnvironment {
            environment: current,
        } = read().await.unwrap()
        else {
            panic!("full read")
        };
        assert_eq!(
            detected.observations, current.observations,
            "Detect must expose the same checked facts as Get"
        );
        assert_eq!(detected.observations.len(), 1);
        assert!(!detected.observations[0].stale);
        assert_eq!(
            detected.observations[0].status,
            EnvironmentObservationStatus::Ready
        );
        assert!(detected
            .operations
            .iter()
            .any(|o| o.operation_id == "check-before-detect"));
    }
}
