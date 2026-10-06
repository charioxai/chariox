//! MP-05 / MP-08 / MP-10 / MP-11: credential-free owner-to-owner encrypted peer drill.
//! Cloud admission is loopback mocked; this is not hosted/fresh-machine acceptance.
use super::tests::{send_managed_peer_request, ManagedPeerRequestHarness, ScopedEnv};
use super::*;
use crate::config::{DaemonConfig, PersistedCloudRelayProfile};
use crate::managed_context::outbound::*;
use crate::managed_context::outbound_service::{
    ManagedContextTransferTarget, ManagedContextTransferTicket,
};
use crate::managed_context::owner_managed::*;
use crate::managed_context::{development::*, kernel::*, package::*};
use crate::runtime::terminal_pairings::public_key_thumbprint;
use chariox_relay::auth::RelaySubjectKind;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct EncryptedPeer<'a> {
    harness: ManagedPeerRequestHarness<'a>,
    source: &'a DaemonConfig,
    identity: RelayCallerIdentity,
    target_public_key: String,
    disconnect_after_chunk: AtomicBool,
    interrupt_import: Option<(
        &'a DaemonConfig,
        &'a crate::managed_context::transfer::ManagedContextTransferStore,
    )>,
}
impl ManagedContextPeerTransport for EncryptedPeer<'_> {
    fn send(
        &self,
        request: RelayPeerRequest,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<RelayPeerResponse, DaemonError>> + Send + '_>,
    > {
        Box::pin(async move {
            if let (
                Some((target, store)),
                RelayPeerRequest::FinalizeManagedContextImport {
                    transfer_id,
                    capability,
                },
            ) = (self.interrupt_import, &request)
            {
                interrupt_after_kernel_publication(
                    store,
                    self.source,
                    target,
                    &self.identity,
                    transfer_id,
                    &capability.clone().into_inner(),
                )?;
                return Err(DaemonError::ManagedContext {
                    code: "fixture_import_interrupted",
                    operation: "owner peer drill",
                    message: "simulated restart after kernel component publication".into(),
                    retryable: true,
                });
            }
            let chunk = matches!(request, RelayPeerRequest::UploadManagedContextChunk { .. });
            let response = send_managed_peer_request(
                &self.harness,
                &self.source.daemon_id,
                &self.identity,
                &self.source.relay_private_key,
                &self.target_public_key,
                request,
            )
            .await;
            if chunk && self.disconnect_after_chunk.swap(false, Ordering::SeqCst) {
                return Err(DaemonError::ManagedContext {
                    code: "fixture_reconnect",
                    operation: "owner peer drill",
                    message: "simulated acknowledgement loss".into(),
                    retryable: true,
                });
            }
            Ok(response)
        })
    }
}

// MP-08/MP-11: simulate interruption after one real component has published,
// before development import and durable completion. No production failpoint is added.
fn interrupt_after_kernel_publication(
    store: &crate::managed_context::transfer::ManagedContextTransferStore,
    source: &DaemonConfig,
    target: &DaemonConfig,
    identity: &RelayCallerIdentity,
    transfer_id: &str,
    capability: &str,
) -> Result<(), DaemonError> {
    use crate::managed_context::transfer::*;
    let caller = ManagedContextTransferCaller {
        kernel_id: source.daemon_id.clone(),
        key_thumbprint: public_key_thumbprint(&source.relay_public_key),
        owner_user_id: identity.user_id.clone().unwrap(),
        realm_id: identity.realm_id.clone(),
        target_environment_id: None,
        target_destination: Some(OwnerManagedDestination::OwnerManagedMachine {
            machine_id: target.host_machine_id.clone(),
            kernel_id: target.daemon_id.clone(),
        }),
        target_kernel_id: target.daemon_id.clone(),
        target_key_thumbprint: public_key_thumbprint(&target.relay_public_key),
    };
    let ManagedContextImportClaim::Claimed(ready) = store.prepare_and_claim_import(
        transfer_id,
        capability,
        &caller,
        crate::session::unix_epoch_ms(),
    )?
    else {
        panic!("partial import must own its durable claim")
    };
    let extracted = extract_managed_context_package(ManagedContextPackageImportRequest {
        package_path: ready.archive_path.clone(),
        expected_package_sha256: ready.archive_sha256.clone(),
        expected_binding: ManagedContextPackageBinding {
            plan: ready.plan.clone(),
            target_environment_id: ready.target_environment_id.clone(),
            source_kernel_id: ready.source_kernel_id.clone(),
            source_key_thumbprint: ready.source_key_thumbprint.clone(),
            target_kernel_id: ready.target_kernel_id.clone(),
            target_key_thumbprint: ready.target_key_thumbprint.clone(),
        },
    })?;
    let ManagedContextPackageKernel::FromKernel(snapshot) = extracted.kernel_context else {
        panic!("fixture selects kernel context")
    };
    let parent = ready.destination_root.parent().unwrap();
    import_kernel_context(KernelContextImportRequest {
        snapshot: *snapshot,
        expected_source: crate::secret::TransferredVaultSourceBinding {
            context_id: ready.plan.context_id.clone(),
            source_kernel_id: ready.source_kernel_id,
            source_key_thumbprint: ready.source_key_thumbprint,
        },
        target_kernel_id: target.daemon_id.clone(),
        target_private_key: target.relay_private_key.clone(),
        capability_root: parent.join(format!("kernel-context-{}", ready.plan.context_id)),
        vault_path: parent.join("unused-vault"),
        publication_root: crate::mcp::CharioxMcpRegistry::user_root()
            .and_then(|root| root.parent().map(std::path::Path::to_path_buf)),
    })?;
    assert!(store
        .launch_target(&ready.plan.context_id, &ready.plan.plan_digest)
        .is_err());
    Ok(())
}

fn enrolled_profile(config: &DaemonConfig, api_url: &str) -> PersistedCloudRelayProfile {
    PersistedCloudRelayProfile {
        kernel_id: Some(config.daemon_id.clone()),
        kernel_credential: Some("synthetic-owner-kernel-credential".into()),
        kernel_public_key_thumbprint: Some(public_key_thumbprint(&config.relay_public_key)),
        machine_id: Some(config.host_machine_id.clone()),
        account_id: "owner-account".into(),
        user_id: "owner-user".into(),
        realm_id: "owner-realm".into(),
        api_url: api_url.into(),
        ..Default::default()
    }
}

async fn target_router(
    config: DaemonConfig,
) -> (
    Arc<CommandRouter>,
    crate::managed_context::transfer::ManagedContextTransferStore,
) {
    let app = crate::test_support::bootstrap_after_test_owner_exit(config).await;
    let store = app.managed_context_transfer_store();
    (
        Arc::new(CommandRouter::with_interactive_capacity(
            Arc::new(tokio::sync::Mutex::new(app)),
            1,
        )),
        store,
    )
}

#[tokio::test(flavor = "current_thread")]
async fn mp05_mp08_mp11_owner_managed_context_encrypted_peer_drill() {
    crate::test_support::isolated_env_test!();
    let _lock = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!(
        "chariox-owner-peer-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir_all(root.join("target-home/.chariox")).unwrap();
    let _cleanup = Cleanup(root.clone());
    let _home = ScopedEnv::set("HOME", root.join("target-home").as_os_str());
    let _chariox_home = ScopedEnv::set(
        "CHARIOX_HOME",
        root.join("target-home/.chariox").as_os_str(),
    );
    let _codex_home = ScopedEnv::set("CODEX_HOME", root.join("target-home/.codex").as_os_str());
    let _claude_home = ScopedEnv::set(
        "CLAUDE_CONFIG_DIR",
        root.join("target-home/.claude").as_os_str(),
    );
    let _opencode_home = ScopedEnv::set(
        "OPENCODE_CONFIG_DIR",
        root.join("target-home/.opencode").as_os_str(),
    );
    let _xdg = ScopedEnv::set(
        "XDG_CONFIG_HOME",
        root.join("target-home/.config").as_os_str(),
    );
    let _caps = ScopedEnv::set(
        "CHARIOX_CAPABILITY_ISOLATION_ROOT",
        root.join("source-capabilities").as_os_str(),
    );
    let _topology = ScopedEnv::set(
        "CHARIOX_MANAGED_PROVIDER_TOPOLOGY",
        std::ffi::OsStr::new(""),
    );
    std::fs::create_dir_all(root.join("source-capabilities/user")).unwrap();
    std::fs::write(
        root.join("source-capabilities/user/user-rules.md"),
        "Read the project instructions.\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("target-home/.codex")).unwrap();
    std::fs::write(
        root.join("target-home/.codex/auth.json"),
        b"target-provider-canary",
    )
    .unwrap();
    std::fs::write(
        root.join("target-home/.git-credentials"),
        b"target-git-canary",
    )
    .unwrap();
    std::fs::write(
        root.join("target-home/.chariox/vault-fixture"),
        b"target-vault-canary",
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_url = format!("http://{}", listener.local_addr().unwrap());
    let mut source = DaemonConfig::for_tests();
    source.cloud_relay = Some(enrolled_profile(&source, &api_url));
    let mut target =
        DaemonConfig::for_tests().with_session_history_root(root.join("target-history"));
    target.daemon_id = "owner-target-kernel".into();
    target.host_machine_id = "owner-target-machine".into();
    target.user_config_path = root.join("target-config.toml");
    target.user_config.history.operational.path =
        Some(root.join("target-operational.db").display().to_string());
    target.user_config.artifacts.operational.root =
        Some(root.join("target-artifacts").display().to_string());
    target.user_config.artifacts.operational.index_path =
        Some(root.join("target-artifacts.db").display().to_string());
    target.user_config.state.path = Some(
        root.join("target-home/.chariox/state.db")
            .display()
            .to_string(),
    );
    target.cloud_relay = Some(enrolled_profile(&target, &api_url));
    let selection = OwnerManagedTransfer {
        target: ManagedContextTransferTarget {
            relay_realm_id: "owner-realm".into(),
            machine_id: target.host_machine_id.clone(),
            kernel_id: target.daemon_id.clone(),
            relay_public_key: target.relay_public_key.clone(),
            key_thumbprint: public_key_thumbprint(&target.relay_public_key),
        },
        context_selection: OwnerManagedContextSelection {
            kernel_context: OwnerManagedKernelSelection::SourceKernelWithoutCredentials,
            development_setup: OwnerManagedDevelopmentSelection::SourceProject {
                project_id: "owner-project".into(),
                repositories: vec![OwnerManagedRepositorySelection {
                    role: DevelopmentRepositoryRole::Primary,
                    workspace_id: root.join("project").display().to_string(),
                    worktree_id: None,
                }],
            },
        },
    };
    std::fs::create_dir_all(root.join("project")).unwrap();
    std::fs::write(root.join("project/README.md"), b"Copied project content\n").unwrap();
    let context_plan =
        crate::managed_bootstrap::ManagedKernelContextPlan::for_owner_managed(&source, &selection)
            .unwrap();
    let plan = context_plan.package_binding();
    let cloud_ticket = ManagedContextTransferTicket {
        environment_id: String::new(),
        context_plan,
        target: selection.target.clone(),
    };
    let cloud_response = serde_json::to_vec(&cloud_ticket).unwrap();
    let source_id = source.daemon_id.clone();
    let target_id = target.daemon_id.clone();
    let plan_id = plan.context_id.clone();
    let server = tokio::spawn(async move {
        for attempt in 0..9 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let (header, length) = loop {
                let mut chunk = [0u8; 4096];
                let read = stream.read(&mut chunk).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&chunk[..read]);
                assert!(bytes.len() < 128 * 1024);
                if let Some(header) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..header]);
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|s| s.trim().parse().ok())
                        })
                        .unwrap();
                    if bytes.len() >= header + 4 + length {
                        break (header, length);
                    }
                }
            };
            assert!(String::from_utf8_lossy(&bytes[..header])
                .starts_with(&format!("POST {TICKET_ENDPOINT} ")));
            let body: serde_json::Value =
                serde_json::from_slice(&bytes[header + 4..header + 4 + length]).unwrap();
            assert_eq!(body["admission"], "target");
            assert_eq!(body["accountId"], "owner-account");
            assert_eq!(body["kernelId"], target_id);
            assert_eq!(body["source"]["kernelId"], source_id);
            assert_eq!(body["contextId"], plan_id);
            assert!(body.get("environmentId").is_none());
            let mut response_ticket: serde_json::Value =
                serde_json::from_slice(&cloud_response).unwrap();
            let wrong_pin = match attempt {
                1 => Some("machineId"),
                2 => Some("kernelId"),
                3 => Some("keyThumbprint"),
                4 => Some("relayPublicKey"),
                _ => None,
            };
            if let Some(pin) = wrong_pin {
                response_ticket["target"][pin] = serde_json::json!("incorrect-pin");
            }
            let response_bytes = serde_json::to_vec(&response_ticket).unwrap();
            let (status, response) = if attempt == 0 {
                ("403 Forbidden", b"{}".as_slice())
            } else {
                ("200 OK", response_bytes.as_slice())
            };
            let headers = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len());
            stream.write_all(headers.as_bytes()).await.unwrap();
            stream.write_all(response).await.unwrap();
        }
    });
    let development = export_development_context(DevelopmentContextExportRequest {
        project_id: "owner-project".into(),
        repositories: vec![DevelopmentRepositorySelection {
            workspace_id: root.join("project").display().to_string(),
            worktree_id: None,
            worktree_path: root.join("project"),
            role: DevelopmentRepositoryRole::Primary,
        }],
        archive_path: root.join("development.tar.gz"),
    })
    .unwrap();
    crate::managed_context::credential_free::validate_development_archive(
        &development.archive_path,
    )
    .unwrap();
    let snapshot = export_kernel_context_without_credentials(KernelContextExportRequest {
        context_id: plan.context_id.clone(),
        source_kernel_id: source.daemon_id.clone(),
        source_key_thumbprint: public_key_thumbprint(&source.relay_public_key),
        target_kernel_id: target.daemon_id.clone(),
        target_key_thumbprint: selection.target.key_thumbprint.clone(),
        vault: None,
    })
    .unwrap();
    let package = export_managed_context_package(ManagedContextPackageExportRequest {
        plan: plan.clone(),
        target_environment_id: String::new(),
        source_kernel_id: source.daemon_id.clone(),
        source_key_thumbprint: public_key_thumbprint(&source.relay_public_key),
        target_kernel_id: target.daemon_id.clone(),
        target_key_thumbprint: selection.target.key_thumbprint.clone(),
        development: ManagedContextPackageDevelopment::FromSource {
            archive_path: development.archive_path,
            archive_sha256: development.archive_sha256,
        },
        kernel_context: ManagedContextPackageKernel::FromKernel(Box::new(snapshot)),
        provider_accounts: ManagedContextPackageProviderAccounts::None,
        git_credentials: ManagedContextPackageGitCredentials::None,
        package_path: root.join("package.ctx"),
    })
    .unwrap();
    drop(_caps);
    let identity = RelayCallerIdentity {
        realm_id: "owner-realm".into(),
        subject: source.daemon_id.clone(),
        subject_kind: RelaySubjectKind::Kernel,
        expires_at_ms: u64::MAX,
        token_id: Some("synthetic-owner-token-id".into()),
        user_id: Some("owner-user".into()),
        public_key_thumbprint: Some(public_key_thumbprint(&source.relay_public_key)),
    };
    let (router, _store) = target_router(target.clone()).await;
    let state = Arc::new(RwLock::new(RelayClientState::default()));
    let (outgoing_tx, _priority_rx, _event_rx) = RelayOutgoingSender::channel(1);
    let harness = ManagedPeerRequestHarness {
        router: &router,
        state: &state,
        outgoing_tx: &outgoing_tx,
    };
    let arm = RelayPeerRequest::ArmManagedContextImport {
        plan: plan.clone(),
        destination: plan.destination.clone(),
        target_environment_id: String::new(),
        target_kernel_id: target.daemon_id.clone(),
        target_key_thumbprint: selection.target.key_thumbprint.clone(),
        capability: random_managed_context_capability(),
        archive_sha256: package.package_sha256.clone(),
        archive_size_bytes: package.package_size_bytes,
    };
    for change in ["user", "realm", "expired"] {
        let mut foreign = identity.clone();
        match change {
            "user" => foreign.user_id = Some("another-user".into()),
            "realm" => foreign.realm_id = "another-account-realm".into(),
            _ => foreign.expires_at_ms = 1,
        };
        // MP-11: expiry may be rejected before decryption; use the admission seam.
        let response = router
            .relay_arm_managed_context_import(
                crate::runtime::router::RelayManagedContextArmRequest {
                    identity: foreign,
                    source_kernel_id: source.daemon_id.clone(),
                    plan: plan.clone(),
                    destination: plan.destination.clone(),
                    target_environment_id: String::new(),
                    target_kernel_id: target.daemon_id.clone(),
                    target_key_thumbprint: selection.target.key_thumbprint.clone(),
                    capability: "c".repeat(43),
                    archive_sha256: package.package_sha256.clone(),
                    archive_size_bytes: package.package_size_bytes,
                },
            )
            .await;
        assert!(response.is_err());
    }
    for mismatch in [
        "machine",
        "kernel",
        "target_key",
        "source_key",
        "mixed_binding",
        "missing_binding",
    ] {
        let mut request = crate::runtime::router::RelayManagedContextArmRequest {
            identity: identity.clone(),
            source_kernel_id: source.daemon_id.clone(),
            plan: plan.clone(),
            destination: plan.destination.clone(),
            target_environment_id: String::new(),
            target_kernel_id: target.daemon_id.clone(),
            target_key_thumbprint: selection.target.key_thumbprint.clone(),
            capability: random_managed_context_capability().into_inner(),
            archive_sha256: package.package_sha256.clone(),
            archive_size_bytes: package.package_size_bytes,
        };
        match mismatch {
            "machine" => {
                request.destination = Some(OwnerManagedDestination::OwnerManagedMachine {
                    machine_id: "different-machine".into(),
                    kernel_id: target.daemon_id.clone(),
                })
            }
            "kernel" => request.target_kernel_id = "different-kernel".into(),
            "target_key" => request.target_key_thumbprint = "f".repeat(64),
            "source_key" => request.identity.public_key_thumbprint = Some("f".repeat(64)),
            "mixed_binding" => request.target_environment_id = "managed-environment".into(),
            _ => request.destination = None,
        }
        assert!(
            router
                .relay_arm_managed_context_import(request)
                .await
                .is_err(),
            "{mismatch}"
        );
    }
    let refused = send_managed_peer_request(
        &harness,
        &source.daemon_id,
        &identity,
        &source.relay_private_key,
        &target.relay_public_key,
        arm.clone(),
    )
    .await;
    assert!(matches!(
        refused,
        RelayPeerResponse::ManagedContextImportFailed {
            retryable: false,
            ..
        }
    ));
    for _ in 0..4 {
        let response = send_managed_peer_request(
            &harness,
            &source.daemon_id,
            &identity,
            &source.relay_private_key,
            &target.relay_public_key,
            arm.clone(),
        )
        .await;
        assert!(matches!(
            response,
            RelayPeerResponse::ManagedContextImportFailed {
                retryable: false,
                ..
            }
        ));
    }
    let capability = random_managed_context_capability();
    let request = ManagedContextOutboundTransferRequest {
        plan: plan.clone(),
        target_environment_id: String::new(),
        target_kernel_id: target.daemon_id.clone(),
        target_key_thumbprint: selection.target.key_thumbprint.clone(),
        package: package.clone(),
        capability,
    };
    let peer = EncryptedPeer {
        harness,
        source: &source,
        identity: identity.clone(),
        target_public_key: target.relay_public_key.clone(),
        disconnect_after_chunk: AtomicBool::new(true),
        interrupt_import: None,
    };
    assert!(
        transfer_managed_context_package(&peer, request.clone(), |_| {})
            .await
            .is_err()
    );
    drop(peer);
    drop(router);
    let (router, store) = target_router(target.clone()).await;
    let peer = EncryptedPeer {
        harness: ManagedPeerRequestHarness {
            router: &router,
            state: &state,
            outgoing_tx: &outgoing_tx,
        },
        source: &source,
        identity: identity.clone(),
        target_public_key: target.relay_public_key.clone(),
        disconnect_after_chunk: AtomicBool::new(false),
        interrupt_import: Some((&target, &store)),
    };
    assert!(
        transfer_managed_context_package(&peer, request.clone(), |_| {})
            .await
            .is_err()
    );
    drop(peer);
    drop(store);
    drop(router);
    let (router, store) = target_router(target.clone()).await;
    let peer = EncryptedPeer {
        harness: ManagedPeerRequestHarness {
            router: &router,
            state: &state,
            outgoing_tx: &outgoing_tx,
        },
        source: &source,
        identity,
        target_public_key: target.relay_public_key.clone(),
        disconnect_after_chunk: AtomicBool::new(false),
        interrupt_import: None,
    };
    let result = transfer_managed_context_package(&peer, request.clone(), |_| {})
        .await
        .unwrap();
    assert_eq!(result.receipt.destination, plan.destination);
    assert_eq!(result.receipt.plan_digest, plan.plan_digest);
    for foreign in ["another-user", ""] {
        assert!(
            crate::runtime::managed_context_target_control::execute_managed_context_target_request(
                target.clone(),
                None,
                store.clone(),
                foreign,
                crate::local::LocalDaemonRequest::GetManagedContextLaunchTarget(
                    crate::local::GetManagedContextLaunchTargetRequest {
                        context_id: plan.context_id.clone(),
                        plan_digest: plan.plan_digest.clone(),
                    }
                ),
            )
            .is_err()
        );
    }
    assert!(
        crate::runtime::managed_context_target_control::execute_managed_context_target_request(
            target.clone(),
            None,
            store.clone(),
            crate::session::DEFAULT_LOCAL_USER_ID,
            crate::local::LocalDaemonRequest::GetManagedContextLaunchTarget(
                crate::local::GetManagedContextLaunchTargetRequest {
                    context_id: plan.context_id.clone(),
                    plan_digest: plan.plan_digest.clone(),
                }
            ),
        )
        .is_ok()
    );
    let launch =
        crate::runtime::managed_context_target_control::execute_managed_context_target_request(
            target.clone(),
            None,
            store.clone(),
            "owner-user",
            crate::local::LocalDaemonRequest::GetManagedContextLaunchTarget(
                crate::local::GetManagedContextLaunchTargetRequest {
                    context_id: plan.context_id.clone(),
                    plan_digest: plan.plan_digest.clone(),
                },
            ),
        )
        .unwrap();
    let crate::local::LocalDaemonResponse::ManagedContextLaunchTarget { target: launch } = launch
    else {
        panic!("launch receipt missing")
    };
    assert!(launch.environment_id.is_empty());
    assert_eq!(launch.destination, plan.destination);
    let crate::local::ManagedContextDevelopmentLaunchTarget::FromSource { repositories, .. } =
        launch.development
    else {
        panic!("project not imported")
    };
    assert_eq!(
        std::fs::read(PathBuf::from(&repositories[0].workspace_path).join("README.md")).unwrap(),
        b"Copied project content\n"
    );
    for (path, expected) in [
        (".codex/auth.json", b"target-provider-canary".as_slice()),
        (".git-credentials", b"target-git-canary"),
        (".chariox/vault-fixture", b"target-vault-canary"),
    ] {
        assert_eq!(
            std::fs::read(root.join("target-home").join(path)).unwrap(),
            expected
        );
    }
    let package_bytes = std::fs::read(&package.package_path).unwrap();
    for canary in [
        b"target-provider-canary".as_slice(),
        b"target-git-canary",
        b"target-vault-canary",
    ] {
        assert!(!package_bytes
            .windows(canary.len())
            .any(|window| window == canary));
    }
    let replay = transfer_managed_context_package(&peer, request, |_| {})
        .await
        .unwrap();
    assert_eq!(replay.receipt, result.receipt);
    server.await.unwrap();
    // MP-10: synthetic bound identities and mocked admission are local evidence only.
    let _public_package_hash = format!("{:x}", Sha256::digest(package_bytes));
}
