//! A selected home starts, watches and stops Setup on its leased worker; nothing else the
//! Environment asks of other kernels is worker authority.
use super::*;

#[test]
fn a_leased_worker_runs_setup_for_its_selected_home_only() {
    std::thread::Builder::new()
        .name("environment-setup-worker".to_string())
        .stack_size(crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("environment setup test runtime")
                .block_on(setup_on_worker())
        })
        .expect("environment setup test thread")
        .join()
        .unwrap_or_else(|error| std::panic::resume_unwind(error));
}

async fn setup_on_worker() {
    let home_private_key = relay_crypto::generate_private_key_base64();
    let home_public_key =
        relay_crypto::public_key_from_private_key_base64(&home_private_key).expect("home key");
    let home_thumbprint = public_key_thumbprint(&home_public_key);
    let mut config = lease_worker_config();
    config.lease_worker_home_caller = Some(crate::config::LeaseWorkerHomeCaller {
        kernel_id: "source-kernel-1".to_string(),
        realm_id: "realm-1".to_string(),
        user_id: "user-1".to_string(),
        relay_public_key: home_public_key,
    });
    let workspace = crate::test_support::TestWorktree::new("environment-setup-worker");
    let worker_app = DaemonApp::bootstrap(config.clone()).expect("worker should boot");
    worker_app
        .session_state_store()
        .restore_projects(vec![crate::session::RuntimeProject::new(
            "project-1",
            "user-1",
            workspace.path().display().to_string(),
            "Worker project",
            crate::session::RuntimeProjectKind::Named,
        )]);
    crate::project_environment::ProjectEnvironmentStore::new(&config.private_runtime_state_root())
        .save(
            "project-1",
            0,
            crate::project_environment::ProjectEnvironmentRecord {
                schema: crate::project_environment::PROJECT_ENVIRONMENT_SCHEMA,
                setup_script:
                    "echo $$ > setup.pid; printf 'worker-ready\\n'; sleep 8; touch finished".into(),
                ..Default::default()
            },
        )
        .expect("worker Environment should save");
    let worker_public_key = worker_app.config().relay_public_key.clone();
    let router = Arc::new(CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(worker_app)),
        1,
    ));
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let home = scoped_machine_identity("machine-home-1", Some(home_thumbprint));
    let setup = RelayPeerRequest::RunProjectEnvironmentSetup {
        project_id: "project-1".to_string(),
        operation_id: "operation-1".to_string(),
        checkout_path: None,
    };

    // Exercise the real script after authenticated worker admission.
    let admitted = send_lease_worker_request(
        &router,
        &state,
        "source-kernel-1",
        home.clone(),
        &home_private_key,
        &worker_public_key,
        setup.clone(),
    )
    .await;
    let RelayPeerResponse::ProjectEnvironmentOperation { operation } =
        decode_lease_worker_response(&home_private_key, admitted)
    else {
        panic!("Setup did not start on the worker")
    };
    assert_eq!(operation.operation_id, "operation-1");
    assert_eq!(
        operation.state,
        crate::local::EnvironmentOperationState::Running
    );
    let mut observed_output = false;
    for _ in 0..200 {
        let observed = send_lease_worker_request(
            &router,
            &state,
            "source-kernel-1",
            home.clone(),
            &home_private_key,
            &worker_public_key,
            RelayPeerRequest::GetProjectEnvironmentOperation {
                operation_id: "operation-1".into(),
            },
        )
        .await;
        let RelayPeerResponse::ProjectEnvironmentOperation { operation } =
            decode_lease_worker_response(&home_private_key, observed)
        else {
            panic!("worker observation failed")
        };
        if operation.output_tail.contains("worker-ready") {
            observed_output = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    if !observed_output {
        let _ = send_lease_worker_request(
            &router,
            &state,
            "source-kernel-1",
            home.clone(),
            &home_private_key,
            &worker_public_key,
            RelayPeerRequest::CancelProjectEnvironmentOperation {
                operation_id: "operation-1".into(),
            },
        )
        .await;
    }
    assert!(observed_output, "the worker script never ran");
    let pid = std::fs::read_to_string(workspace.path().join("setup.pid"))
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    let script_identity = crate::runtime::kernel_access::process::inspect(pid)
        .unwrap()
        .0;

    let stop = send_lease_worker_request(
        &router,
        &state,
        "source-kernel-1",
        home.clone(),
        &home_private_key,
        &worker_public_key,
        RelayPeerRequest::CancelProjectEnvironmentOperation {
            operation_id: "operation-1".to_string(),
        },
    )
    .await;
    let RelayPeerResponse::ProjectEnvironmentOperation { operation } =
        decode_lease_worker_response(&home_private_key, stop)
    else {
        panic!("worker cancellation failed")
    };
    assert_eq!(operation.operation_id, "operation-1");
    assert!(matches!(
        operation.state,
        crate::local::EnvironmentOperationState::Running
            | crate::local::EnvironmentOperationState::Cancelled
    ));
    for _ in 0..200 {
        if script_identity.gone() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        script_identity.gone(),
        "the authenticated Stop left the worker script alive"
    );
    assert!(!workspace.path().join("finished").exists());
    let mut settled = false;
    for _ in 0..200 {
        let observed = send_lease_worker_request(
            &router,
            &state,
            "source-kernel-1",
            home.clone(),
            &home_private_key,
            &worker_public_key,
            RelayPeerRequest::GetProjectEnvironmentOperation {
                operation_id: "operation-1".into(),
            },
        )
        .await;
        let RelayPeerResponse::ProjectEnvironmentOperation { operation } =
            decode_lease_worker_response(&home_private_key, observed)
        else {
            panic!("worker observation failed")
        };
        if operation.state == crate::local::EnvironmentOperationState::Cancelled {
            settled = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        settled,
        "the worker's original operation did not settle as cancelled"
    );

    // A kernel that is not the worker's selected home is refused.
    let other_private_key = relay_crypto::generate_private_key_base64();
    let other_public_key =
        relay_crypto::public_key_from_private_key_base64(&other_private_key).expect("other key");
    let other = scoped_machine_identity(
        "machine-other-1",
        Some(public_key_thumbprint(&other_public_key)),
    );
    let refused = send_lease_worker_request(
        &router,
        &state,
        "other-kernel-1",
        other,
        &other_private_key,
        &worker_public_key,
        setup,
    )
    .await;
    assert!(refused.error.is_some(), "an unselected home ran Setup");

    // Reading the worker's Environment is not something a home may ask of it.
    let read = send_lease_worker_request(
        &router,
        &state,
        "source-kernel-1",
        home,
        &home_private_key,
        &worker_public_key,
        RelayPeerRequest::ReadProjectEnvironment {
            project_id: "project-1".to_string(),
        },
    )
    .await;
    assert!(
        read.error.is_some(),
        "a worker answered an Environment read"
    );
}
