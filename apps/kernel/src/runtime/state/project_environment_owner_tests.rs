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
    crate::runtime::router::CommandRouter,
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
    (_cleanup, worktree, runtime, session, router)
}

async fn owner_context_review_rejection_inner() {
    let (_cleanup, worktree, runtime, session, _router) = review_fixture().await;
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
            .review_credential_free_project_context(
                &project,
                &[repository],
                "project-rejection",
                "Owner machine",
            )
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
                .find(|interaction| interaction.id().starts_with("owner-context:"))
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

#[test]
fn mp08_mp11_source_project_owner_copy_waits_for_approval_and_cancellation_stops_export() {
    crate::test_support::isolated_env_test!();
    std::thread::Builder::new()
        .name("project-owner-review".into())
        .stack_size(crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(owner_copy_inner(true, false));
        })
        .unwrap()
        .join()
        .unwrap_or_else(|error| std::panic::resume_unwind(error));
}

async fn kernel_only_owner_copy_inner() {
    owner_copy_inner(false, false).await;
}

// MP-08/MP-11: the real enrolled terminal must cancel and continue pre-enrollment copies.
#[test]
fn mp08_mp11_legacy_local_project_copy_terminal_cancel_and_continue() {
    legacy_local_copy_test(true);
}

#[test]
fn mp08_mp11_legacy_local_kernel_copy_terminal_cancel_and_continue() {
    legacy_local_copy_test(false);
}

fn legacy_local_copy_test(source_project: bool) {
    crate::test_support::isolated_env_test!();
    std::thread::Builder::new()
        .name("legacy-local-copy".into())
        .stack_size(crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(owner_copy_inner(source_project, true));
        })
        .unwrap()
        .join()
        .unwrap_or_else(|error| std::panic::resume_unwind(error));
}

async fn owner_copy_inner(source_project: bool, legacy_local: bool) {
    use crate::managed_context::outbound_service::*;
    use crate::managed_context::owner_managed::*;
    use crate::transport::{relay_client::RelayClientState, relay_crypto};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_cleanup, _worktree, runtime, session, router) = review_fixture().await;
    let mut config = runtime.owned.config_projection.snapshot();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let owner = if legacy_local {
        assert_eq!(
            session.owner_user_id(),
            crate::session::DEFAULT_LOCAL_USER_ID
        );
        assert_eq!(
            runtime
                .owned
                .session_store
                .get_project(session.project_id())
                .unwrap()
                .owner_user_id(),
            session.owner_user_id()
        );
        "enrolled-copy-owner"
    } else {
        session.owner_user_id()
    };
    config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
        api_url: format!("http://{}", listener.local_addr().unwrap()),
        account_id: "review-account".into(),
        user_id: owner.into(),
        realm_id: "review-realm".into(),
        machine_id: Some(config.host_machine_id.clone()),
        kernel_id: Some(config.daemon_id.clone()),
        kernel_public_key_thumbprint: Some(
            crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key),
        ),
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
                development_setup: if source_project {
                    OwnerManagedDevelopmentSelection::SourceProject {
                        project_id: session.project_id().into(),
                        repositories: vec![OwnerManagedRepositorySelection {
                            role: crate::managed_context::development::DevelopmentRepositoryRole::Primary,
                            workspace_id: session.workspace_id().into(),
                            worktree_id: None,
                        }],
                    }
                } else {
                    OwnerManagedDevelopmentSelection::Empty
                },
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
        let source = serde_json::json!({
            "relayRealmId": "review-realm", "machineId": config.host_machine_id,
            "kernelId": config.daemon_id, "relayPublicKey": config.relay_public_key,
            "keyThumbprint": crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key),
        });
        let selected = serde_json::to_value(&selection.context_selection).unwrap();
        let binding = serde_json::json!({ "kind": "owner_managed_machine", "sourceTargetId": "review-source-target",
            "source": source, "target": target, "contextSelection": selected });
        let plan = ticket.context_plan.package_binding();
        let cloud = async {
            for step in 0..4 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let (headers, body) = loop {
                    let mut chunk = [0; 4096];
                    let size = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(size, 0);
                    request.extend_from_slice(&chunk[..size]);
                    let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                    else {
                        continue;
                    };
                    let headers = String::from_utf8_lossy(&request[..end]).into_owned();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        let body = if length == 0 {
                            serde_json::json!({})
                        } else {
                            serde_json::from_slice(&request[end + 4..end + 4 + length]).unwrap()
                        };
                        break (headers, body);
                    }
                };
                assert!(body.get("environmentId").is_none());
                let (status, response) = match step {
                    0 => {
                        assert!(headers.starts_with("POST /v1/managed-kernels/context/ticket "));
                        assert_eq!(body["ownerManagedExport"]["contextId"], plan.context_id);
                        (
                            "403 Forbidden",
                            serde_json::json!({"error":{"code":"authorization_expired"}}),
                        )
                    }
                    1 => {
                        assert!(headers.starts_with("GET /v1/owner-managed-context-tickets/peers "));
                        assert!(headers
                            .to_ascii_lowercase()
                            .contains("x-chariox-kernel-credential:"));
                        (
                            "200 OK",
                            serde_json::json!({"peers":[{"targetId":"review-source-target","peer":source}]}),
                        )
                    }
                    2 => {
                        assert!(headers.starts_with("POST /v1/owner-managed-context-tickets "));
                        assert!(headers
                            .to_ascii_lowercase()
                            .contains("x-chariox-kernel-credential:"));
                        assert_eq!(body["sourceTargetId"], binding["sourceTargetId"]);
                        assert_eq!(body["target"], binding["target"]);
                        assert_eq!(body["contextSelection"], selected);
                        let mut issued = binding.clone();
                        issued["ticket"] = serde_json::json!("synthetic-owner-ticket");
                        ("200 OK", issued)
                    }
                    _ => {
                        assert!(headers.starts_with("POST /v1/managed-kernels/context/ticket "));
                        assert_eq!(body["ownerManaged"]["ticket"], "synthetic-owner-ticket");
                        assert_eq!(body["ownerManaged"]["contextId"], plan.context_id);
                        assert_eq!(body["ownerManaged"]["planDigest"], plan.plan_digest);
                        assert_eq!(body["ownerManaged"]["source"], source);
                        assert_eq!(body["ownerManaged"]["contextSelection"], selected);
                        ("200 OK", binding.clone())
                    }
                };
                let response = serde_json::to_vec(&response).unwrap();
                let header = format!("HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", response.len());
                socket.write_all(header.as_bytes()).await.unwrap();
                socket.write_all(&response).await.unwrap();
            }
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
        let request = crate::local::LocalDaemonRequest::RespondToInteraction(
            crate::local::RespondToInteractionRequest {
                session_id: session.id().into(),
                interaction_id: interaction.id().into(),
                choice_id: if approve { "continue" } else { "cancel" }.into(),
                custom_reply: None,
                passkey: None,
                passkey_remember_minutes: None,
            },
        );
        let caller = router
            .local_command_caller(
                crate::runtime::command::KernelCommandSource::LocalCli,
                crate::local::KernelConnectionClass::Terminal,
            )
            .await;
        assert_eq!(caller.user_id.as_deref(), Some(owner));
        let command = crate::runtime::command::KernelCommand::from_local_request_with_caller(
            "owner-copy-answer",
            crate::runtime::command::KernelCommandSource::LocalCli,
            caller,
            None,
            None,
            &request,
        );
        assert!(matches!(
            router.dispatch(command, request).await.unwrap(),
            crate::local::LocalDaemonResponse::InteractionResponded { .. }
        ));
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
