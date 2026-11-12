use super::*;
use crate::durable_state::{
    app_installation_staging::AppVerifiedInstallationMutation as Install,
    app_publishers::AppPublisherMutation,
    apps::{AppRegistryMutation, AppRegistryOutcome},
};
use chariox_app_runtime::{
    installation::{CapabilityApproval, CapabilityDecision, VerifiedInstallCandidate},
    publisher_trust::TrustDecision,
    release_store::StageBudget,
};

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn setup() -> (Scratch, DurableKernelStateStore) {
    let root =
        std::env::temp_dir().join(format!("client-app-view-{:032x}", rand::random::<u128>()));
    std::fs::create_dir(&root).unwrap();
    let store = DurableKernelStateStore::open_owned(root.join("kernel.db")).unwrap();
    install(&store);
    (Scratch(root), store)
}

fn install(store: &DurableKernelStateStore) {
    let (bytes, publisher) = crate::runtime::app_package_preparation::test_package::package();
    store
        .mutate_app_publisher(
            "alice",
            AppPublisherMutation::Enroll {
                publisher: publisher.clone(),
                expected_revision: 0,
                decision: decision("enroll"),
                now_ms: 1,
            },
        )
        .unwrap();
    let trust = store
        .trusted_app_publisher("alice", "com.example", "developer-1")
        .unwrap();
    let verified = chariox_app_package::verify_app(
        &bytes,
        &chariox_app_package::VerificationPolicy::new(
            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
            vec![publisher],
        ),
    )
    .unwrap();
    ReleaseStore::open_or_create(store.path())
        .unwrap()
        .stage(
            &verified,
            &bytes,
            StageBudget {
                max_stage_bytes: 1024 * 1024,
                reserved_bytes: 1024 * 1024,
                host_reserve_bytes: 1024 * 1024,
            },
        )
        .unwrap();
    let candidate = VerifiedInstallCandidate::from_verified(&verified, &trust).unwrap();
    let AppRegistryOutcome::Update(update) = store
        .mutate_verified_app_installation(
            "alice",
            Install::CreateAndStage {
                installation_id: "installed".into(),
                candidate,
                now_ms: 2,
            },
        )
        .unwrap()
    else {
        panic!("staged")
    };
    for mutation in [
        AppRegistryMutation::Decide {
            token: update.token.clone(),
            decision: CapabilityDecision::Approved {
                approval: CapabilityApproval {
                    decision_id: "approve".into(),
                    authority_ref: "kernel-test".into(),
                },
            },
            now_ms: 3,
        },
        AppRegistryMutation::Quiesce {
            token: update.token.clone(),
            now_ms: 4,
        },
        AppRegistryMutation::MarkPrepared {
            token: update.token.clone(),
            now_ms: 5,
        },
    ] {
        store.mutate_app_installation("alice", mutation).unwrap();
    }
    store
        .mutate_verified_app_installation(
            "alice",
            Install::Commit {
                token: update.token,
                now_ms: 6,
            },
        )
        .unwrap();
}
fn decision(id: &str) -> TrustDecision {
    TrustDecision {
        decision_id: id.into(),
        authority_ref: "kernel-test".into(),
    }
}
fn open(views: &ClientAppViews, store: &DurableKernelStateStore) -> Descriptor {
    let Response::Opened(view) = views
        .handle(
            store,
            "alice",
            Request::Open {
                installation_id: "installed".into(),
            },
        )
        .unwrap()
    else {
        panic!("opened")
    };
    *view
}
fn read(id: &str, offset: u32, max_bytes: u32) -> Request {
    Request::Read {
        view_id: id.into(),
        offset,
        max_bytes,
    }
}

#[tokio::test]
async fn client_app_view_delivery_uses_signed_controls_and_bounded_generation_lease() {
    let (_scratch, store) = setup();
    let views = ClientAppViews::default();
    let view = open(&views, &store);
    let subject = serde_json::json!({ "schema": "chariox.package-signature.v2",
        "manifest": serde_json::from_str::<serde_json::Value>(&view.bundle.manifest).unwrap(),
        "inventory": serde_json::from_str::<serde_json::Value>(&view.bundle.inventory).unwrap() });
    use ed25519_dalek::Verifier;
    let key: [u8; 32] = BASE64
        .decode(&view.trust.public_key)
        .unwrap()
        .try_into()
        .unwrap();
    let signature = BASE64
        .decode(view.bundle.signature["value"].as_str().unwrap())
        .unwrap();
    ed25519_dalek::VerifyingKey::from_bytes(&key)
        .unwrap()
        .verify(
            &serde_json::to_vec(&subject).unwrap(),
            &ed25519_dalek::Signature::from_slice(&signature).unwrap(),
        )
        .unwrap();
    let mut bytes = Vec::new();
    loop {
        let Response::Chunk {
            offset,
            body_base64,
            eof,
            ..
        } = views
            .handle(&store, "alice", read(&view.view_id, bytes.len() as u32, 7))
            .unwrap()
        else {
            panic!("chunk")
        };
        assert_eq!(offset as usize, bytes.len());
        let part = BASE64.decode(body_base64).unwrap();
        assert!(part.len() <= 7);
        bytes.extend(part);
        if eof {
            break;
        }
    }
    assert_eq!(bytes, b"<!doctype html><title>prepared</title>");
    assert!(matches!(
        views.handle(&store, "alice", read(&view.view_id, 0, 0)),
        Err(Error::InvalidRequest)
    ));
    assert!(matches!(
        views.handle(&store, "alice", read(&view.view_id, 0, MAX_CHUNK_BYTES + 1)),
        Err(Error::InvalidRequest)
    ));
    assert!(matches!(
        views.handle(&store, "alice", read(&view.view_id, bytes.len() as u32, 1)),
        Err(Error::InvalidRequest)
    ));
    assert!(matches!(
        views.handle(&store, "bob", read(&view.view_id, 0, 1)),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        views.handle(
            &store,
            "bob",
            Request::Close {
                view_id: view.view_id.clone()
            }
        ),
        Err(Error::NotFound)
    ));
    views
        .handle(
            &store,
            "alice",
            Request::Check {
                view_id: view.view_id.clone(),
            },
        )
        .unwrap();
    let expiration = views
        .0
        .lock()
        .unwrap()
        .get(&view.view_id)
        .unwrap()
        .expiration
        .clone();
    views
        .handle(
            &store,
            "alice",
            Request::Close {
                view_id: view.view_id.clone(),
            },
        )
        .unwrap();
    tokio::task::yield_now().await;
    assert!(
        expiration.is_finished(),
        "Close cancels the lease timer instead of accumulating tasks"
    );
    assert!(views.0.lock().unwrap().is_empty());
    assert!(matches!(
        views.handle(&store, "alice", read(&view.view_id, 0, 1)),
        Err(Error::NotFound)
    ));
}

#[tokio::test]
async fn client_app_view_delivery_revocation_uninstall_and_expiry_reject_cached_authority() {
    for cause in ["revoke", "uninstall", "expiry"] {
        let (_scratch, store) = setup();
        let views = ClientAppViews::default();
        let view = open(&views, &store);
        match cause {
            "revoke" => {
                store
                    .mutate_app_publisher(
                        "alice",
                        AppPublisherMutation::Revoke {
                            publisher_id: "com.example".into(),
                            key_id: "developer-1".into(),
                            expected_revision: 1,
                            decision: decision("revoke"),
                            now_ms: 9,
                        },
                    )
                    .unwrap();
            }
            "uninstall" => {
                store
                    .mutate_app_installation(
                        "alice",
                        AppRegistryMutation::Uninstall {
                            installation_id: "installed".into(),
                            expected_generation: view.generation.parse().unwrap(),
                            now_ms: 9,
                        },
                    )
                    .unwrap();
            }
            _ => {
                let mut registry = views.0.lock().unwrap();
                Arc::get_mut(registry.get_mut(&view.view_id).unwrap())
                    .unwrap()
                    .expires = Instant::now();
            }
        }
        assert!(
            matches!(
                views.handle(&store, "alice", read(&view.view_id, 0, 1)),
                Err(Error::Conflict)
            ),
            "{cause}"
        );
        assert!(
            views.0.lock().unwrap().is_empty(),
            "{cause} drops its release lease"
        );
    }
}

#[tokio::test]
async fn client_app_view_router_uses_terminal_owner_and_refuses_agents() {
    use crate::{
        app::DaemonApp,
        config::DaemonConfig,
        local::{KernelConnectionClass, LocalDaemonRequest, LocalDaemonResponse},
        runtime::{
            command::{KernelCaller, KernelCallerKind, KernelCommand, KernelCommandSource},
            router::CommandRouter,
        },
    };
    let root = std::env::temp_dir().join(format!("cv-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&root).unwrap();
    let _scratch = Scratch(root.clone());
    let mut config = DaemonConfig::for_tests().with_session_history_root(root.join("history"));
    config.user_config_path = root.join("config.toml");
    let socket = crate::test_support::TestSocketDirectory::new();
    config.local_socket_path = socket.path().join("kernel.sock");
    config.user_config.state.path = Some(root.join("kernel.db").display().to_string());
    config.user_config.history.operational.path =
        Some(root.join("events.db").display().to_string());
    config.user_config.artifacts.operational.root =
        Some(root.join("artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(root.join("artifacts.db").display().to_string());
    config.user_config.credential_vault.backend =
        crate::config::CredentialVaultBackend::ProcessMemory;
    let app = DaemonApp::bootstrap(config).unwrap();
    let store = app.durable_state_store();
    install(&store);
    let router =
        CommandRouter::with_interactive_capacity(Arc::new(tokio::sync::Mutex::new(app)), 2);
    for (owner, class, admitted) in [
        ("alice", KernelConnectionClass::Terminal, true),
        ("bob", KernelConnectionClass::Terminal, false),
        ("alice", KernelConnectionClass::KernelAgent, false),
    ] {
        let request = LocalDaemonRequest::ClientAppView(Request::Open {
            installation_id: "installed".into(),
        });
        let command = KernelCommand::from_local_request_with_caller(
            format!("client-view-{:032x}", rand::random::<u128>()),
            KernelCommandSource::RelayClient,
            KernelCaller {
                caller_kind: KernelCallerKind::RemoteClient,
                caller_id: "client".into(),
                client_id: Some("client".into()),
                user_id: Some(owner.into()),
                connection_class: Some(class),
                ..Default::default()
            },
            None,
            None,
            &request,
        );
        let response = router.dispatch(command, request).await;
        if admitted {
            assert!(
                matches!(
                    response,
                    Ok(LocalDaemonResponse::ClientAppView(Response::Opened(_)))
                ),
                "{response:?}"
            );
        } else {
            assert!(
                !matches!(response, Ok(LocalDaemonResponse::ClientAppView(_))),
                "{response:?}"
            );
        }
    }
}
