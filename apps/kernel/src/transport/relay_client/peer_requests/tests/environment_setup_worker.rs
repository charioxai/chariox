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
    let worker_app = DaemonApp::bootstrap(config).expect("worker should boot");
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

    // The worker holds no such Project: admission passed and the Environment answered.
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
    assert!(matches!(
        decode_lease_worker_response(&home_private_key, admitted),
        RelayPeerResponse::ProjectEnvironmentRefused { .. }
    ));

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
    assert!(matches!(
        decode_lease_worker_response(&home_private_key, stop),
        RelayPeerResponse::ProjectEnvironmentRefused { .. }
    ));

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
