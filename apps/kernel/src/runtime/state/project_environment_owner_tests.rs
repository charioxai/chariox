//! MP-08/MP-11: owner copy review is native, compulsory and cancellable.
use super::*;
use crate::config::DaemonConfig;
struct ReviewCleanup(std::path::PathBuf);
impl Drop for ReviewCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn mp08_mp11_owner_context_review_rejection_prevents_preparation() {
    crate::test_support::isolated_env_test!();
    std::thread::Builder::new()
        .name("owner-context-review".into())
        .stack_size(crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(owner_context_review_rejection_inner());
        })
        .unwrap()
        .join()
        .unwrap_or_else(|error| std::panic::resume_unwind(error));
}

async fn review_fixture() -> (
    ReviewCleanup,
    crate::test_support::TestWorktree,
    KernelRuntimeState,
    crate::session::RuntimeSession,
) {
    let _guard = crate::env_lock::lock();
    let worktree = crate::test_support::TestWorktree::new("owner-context-review");
    let root = std::env::temp_dir().join(format!(
        "chariox-owner-review-state-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let _cleanup = ReviewCleanup(root.clone());
    let mut config = DaemonConfig::for_tests();
    config.user_config.state.path = Some(root.join("state.db").display().to_string());
    config.user_config_path = root.join("config.toml");
    config.user_config.history.operational.path =
        Some(root.join("history.db").display().to_string());
    config.user_config.artifacts.operational.root =
        Some(root.join("artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(root.join("artifacts.db").display().to_string());
    config = config.with_session_history_root(root.join("sessions"));
    let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity(
        Arc::new(tokio::sync::Mutex::new(app)),
        8,
    );
    let runtime = router.runtime_state();
    (_cleanup, worktree, runtime, session)
}

async fn owner_context_review_rejection_inner() {
    let (_cleanup, worktree, runtime, session) = review_fixture().await;
    let project = session.project_id().to_string();
    let repository = crate::managed_context::development::DevelopmentRepositorySelection {
        workspace_id: worktree.path().display().to_string(),
        worktree_id: None,
        worktree_path: worktree.path().to_path_buf(),
        role: crate::managed_context::development::DevelopmentRepositoryRole::Primary,
    };
    let task_runtime = runtime.clone();
    let task = tokio::spawn(async move {
        task_runtime
            .review_credential_free_project_context(&project, &[repository], "Owner machine")
            .await
    });
    let interaction = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let current = runtime
                .owned
                .session_store
                .get_session(session.id())
                .unwrap();
            if let Some(interaction) = current
                .active_interactions()
                .iter()
                .find(|interaction| interaction.id().starts_with("project-environment:"))
            {
                break interaction.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(interaction.title().unwrap().starts_with("Ready to move "));
    assert!(interaction.title().unwrap().ends_with(" to Owner machine"));
    assert!(interaction
        .choices()
        .iter()
        .any(|choice| choice.label() == "Looks good, continue"));
    runtime
        .resolve_terminal_runtime_interaction(
            session.id(),
            interaction.id(),
            "cancel",
            None,
            Some(session.owner_user_id()),
        )
        .await
        .unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .is_err());
}

// MP-08/MP-11: exercise source start, not just the review helper.
#[test]
fn mp08_mp11_kernel_only_owner_copy_waits_for_approval_and_cancellation_stops_export() {
    crate::test_support::isolated_env_test!();
    std::thread::Builder::new()
        .name("kernel-only-owner-review".into())
        .stack_size(crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(kernel_only_owner_copy_inner());
        })
        .unwrap()
        .join()
        .unwrap_or_else(|error| std::panic::resume_unwind(error));
}

async fn kernel_only_owner_copy_inner() {
    use crate::managed_context::outbound_service::*;
    use crate::managed_context::owner_managed::*;
    use crate::transport::{relay_client::RelayClientState, relay_crypto};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_cleanup, _worktree, runtime, session) = review_fixture().await;
    let mut config = runtime.owned.config_projection.snapshot();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        api_url: format!("http://{}", listener.local_addr().unwrap()),
        account_id: "review-account".into(),
        user_id: session.owner_user_id().into(),
        realm_id: "review-realm".into(),
        machine_id: Some("source-machine-test".into()),
        kernel_credential: Some("synthetic-review-fixture".into()),
        ..Default::default()
    });
    runtime.owned.config_projection.update(config.clone());
    let target_key = relay_crypto::public_key_from_private_key_base64(
        &relay_crypto::generate_private_key_base64(),
    )
    .unwrap();
    let target = ManagedContextTransferTarget {
        relay_realm_id: "review-realm".into(),
        machine_id: "review-target-machine".into(),
        kernel_id: "review-target-kernel".into(),
        key_thumbprint: crate::runtime::terminal_pairings::public_key_thumbprint(&target_key),
        relay_public_key: target_key,
    };
    let root = _cleanup.0.join("outbound");
    let store = ManagedContextOutboundOperationStore::open(root.clone()).unwrap();
    for approve in [false, true] {
        let selection = OwnerManagedTransfer {
            target: target.clone(),
            context_selection: OwnerManagedContextSelection {
                kernel_context: OwnerManagedKernelSelection::SourceKernelWithoutCredentials,
                development_setup: OwnerManagedDevelopmentSelection::Empty,
            },
        };
        let ticket = ManagedContextTransferTicket {
            environment_id: String::new(),
            target: target.clone(),
            context_plan: crate::managed_bootstrap::ManagedKernelContextPlan::for_owner_managed(
                &config, &selection,
            )
            .unwrap(),
        };
        let context = ticket.context_plan.context_id().to_string();
        let response = serde_json::to_vec(&ticket).unwrap();
        let cloud = async {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                let size = socket.read(&mut chunk).await.unwrap();
                assert_ne!(size, 0);
                request.extend_from_slice(&chunk[..size]);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let length = String::from_utf8_lossy(&request[..end])
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let header = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", response.len());
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(&response).await.unwrap();
        };
        start_managed_context_outbound_operation(
            config.clone(),
            Arc::new(tokio::sync::RwLock::new(RelayClientState::default())),
            store.clone(),
            runtime.owned.provider_account_profiles.clone(),
            ticket,
            Some(runtime.clone()),
            true,
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), cloud)
            .await
            .unwrap();
        let interaction = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let current = runtime
                    .owned
                    .session_store
                    .get_session(session.id())
                    .unwrap();
                if let Some(review) = current
                    .active_interactions()
                    .iter()
                    .find(|review| review.id().starts_with("owner-context:"))
                {
                    break review.clone();
                }
                let status = store.get(&context).unwrap();
                assert_eq!(
                    status.package_size_bytes, 0,
                    "MP-11 must not package before owner approval"
                );
                assert_eq!(
                    status.phase,
                    ManagedContextOutboundOperationPhase::Preparing,
                    "MP-11 must not transfer before owner approval"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(interaction
            .title()
            .unwrap()
            .contains("review-target-machine"));
        tokio::time::sleep(Duration::from_millis(100)).await;
        let status = store.get(&context).unwrap();
        assert_eq!(
            status.phase,
            ManagedContextOutboundOperationPhase::Preparing
        );
        assert_eq!(status.package_size_bytes, 0);
        assert_eq!(status.accepted_bytes, 0);
        assert!(status.receipt.is_none());
        assert!(!root.join(&context).exists());
        runtime
            .resolve_terminal_runtime_interaction(
                session.id(),
                interaction.id(),
                if approve { "continue" } else { "cancel" },
                None,
                Some(session.owner_user_id()),
            )
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let status = store.get(&context).unwrap();
                if approve && status.package_size_bytes > 0 {
                    break;
                }
                if !approve && status.phase == ManagedContextOutboundOperationPhase::Failed {
                    assert_eq!(
                        status.failure_code.as_deref(),
                        Some("managed_context_review_cancelled")
                    );
                    assert!(!status.retryable);
                    assert_eq!(status.package_size_bytes, 0);
                    assert_eq!(status.accepted_bytes, 0);
                    assert!(status.receipt.is_none());
                    assert!(!root.join(&context).exists());
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}
