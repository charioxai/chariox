//! Profile updates must use confirmed receiving IDs and cannot cache forged receipts.
use super::*;
use crate::account_profile::*;
use crate::transport::relay_client::{RelayClientState, RelayOutgoingSender};
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use chariox_relay::protocol::RelayEnvelope;
use tokio::sync::{mpsc, RwLock};

struct CopyFixture {
    _app: Arc<Mutex<DaemonApp>>,
    runtime: KernelRuntimeState,
    home: crate::config::DaemonConfig,
    worker: crate::config::DaemonConfig,
    session: String,
    agent: String,
    account: String,
    provider: String,
    relay: Arc<RwLock<RelayClientState>>,
    requests: mpsc::Receiver<RelayEnvelope>,
}

impl CopyFixture {
    async fn new() -> Self {
        Self::for_provider("codex").await
    }

    async fn for_provider(provider: &str) -> Self {
        let mut home = crate::config::DaemonConfig::for_tests();
        home.relay_url = Some("ws://127.0.0.1:1".into());
        home.relay_token = Some("synthetic-relay".into());
        let mut worker = crate::config::DaemonConfig::for_tests();
        worker.daemon_id = "copy-worker".into();
        worker.host_machine_id = "copy-worker-machine".into();
        let (app, runtime, session, agent) = agent_config_runtime_with_config(home.clone()).await;
        app.lock()
            .await
            .agents_mut()
            .bind_remote_execution(
                &agent,
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: worker.daemon_id.clone(),
                    worker_machine_id: worker.host_machine_id.clone(),
                    execution_lease_id: "copy-lease".into(),
                    leased_agent_id: "copy-agent".into(),
                    active_worker_provider_run_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
        let relay = app.lock().await.relay_client_state();
        let (sender, requests, _) = RelayOutgoingSender::channel(8);
        {
            let mut state = relay.write().await;
            state.test_set_connected_sender(sender, home.relay_url.clone().unwrap());
            state.remember_peer_public_key(&worker.daemon_id, worker.relay_public_key.clone());
        }
        let registry = &runtime.owned.provider_account_profiles;
        let profile = registry
            .create_managed(
                crate::session::DEFAULT_LOCAL_USER_ID,
                provider,
                "Selected copy",
            )
            .unwrap();
        let environment = registry
            .resolve_environment(
                crate::session::DEFAULT_LOCAL_USER_ID,
                provider,
                &profile.profile_id,
            )
            .unwrap();
        let (auth, bytes): (_, &[u8]) = if provider == "opencode" {
            (std::path::Path::new(&environment["XDG_DATA_HOME"]).join("opencode/auth.json"),
             br#"{"openai":{"type":"oauth","refresh":"synthetic-copy-login","access":"synthetic","expires":9999999999999}}"#)
        } else {
            (
                std::path::Path::new(&environment["CODEX_HOME"]).join("auth.json"),
                br#"{"tokens":{"refresh_token":"synthetic-copy-login"}}"#,
            )
        };
        std::fs::create_dir_all(auth.parent().unwrap()).unwrap();
        std::fs::write(auth, bytes).unwrap();
        Self {
            _app: app,
            runtime,
            home,
            worker,
            session,
            agent,
            account: profile.profile_id,
            provider: provider.into(),
            relay,
            requests,
        }
    }

    fn update(&self) -> tokio::task::JoinHandle<Result<crate::agent::AgentInstance, DaemonError>> {
        let runtime = self.runtime.clone();
        let session = self.session.clone();
        let agent = self.agent.clone();
        let account = self.account.clone();
        let provider = self.provider.clone();
        tokio::spawn(async move {
            runtime
                .update_agent_profile(
                    &session,
                    &agent,
                    crate::session::DEFAULT_LOCAL_USER_ID,
                    Some(provider),
                    Some(account),
                    Some("copy-model".into()),
                    None,
                )
                .await
        })
    }

    async fn request(&mut self) -> (String, RelayPeerRequest) {
        let envelope =
            tokio::time::timeout(std::time::Duration::from_secs(2), self.requests.recv())
                .await
                .unwrap()
                .unwrap();
        let RelayEnvelope::DaemonPeerRequest {
            request_id,
            encrypted_request,
            ..
        } = envelope
        else {
            panic!("expected peer request")
        };
        let decrypted = crate::transport::relay_crypto::decrypt_payload_for_private_key(
            &self.worker.relay_private_key,
            &encrypted_request,
        )
        .unwrap();
        (
            request_id,
            serde_json::from_slice(&decrypted.plaintext).unwrap(),
        )
    }

    async fn reply(&self, id: String, response: RelayPeerResponse) {
        let encrypted = crate::transport::relay_crypto::encrypt_payload_for_peer(
            &self.worker.relay_private_key,
            &self.home.relay_public_key,
            &serde_json::to_vec(&response).unwrap(),
        )
        .unwrap();
        crate::transport::relay_client::resolve_pending_peer_response_for_test(
            &self.relay,
            id,
            self.worker.daemon_id.clone(),
            encrypted,
        )
        .await;
    }

    fn receipt(
        &self,
        materialization: &ProviderAccountMaterialization,
        target_account: &str,
    ) -> ProviderAccountMaterializationStatus {
        let source = materialization.copy_source.as_ref().unwrap();
        ProviderAccountMaterializationStatus {
            target_kind: ProviderAccountMaterializationTargetKind::Worker,
            target_ref: self.worker.daemon_id.clone(),
            state: ProviderAccountMaterializationState::Materialized,
            observed_at_ms: 1,
            last_error: None,
            copy: Some(ProviderAccountCopyMetadata {
                source_machine_id: source.machine_id.clone(),
                source_kernel_id: source.kernel_id.clone(),
                source_account_id: materialization.profile.profile_id.clone(),
                target_machine_id: self.worker.host_machine_id.clone(),
                target_kernel_id: self.worker.daemon_id.clone(),
                target_account_id: target_account.into(),
                renewable_services: vec!["codex".into()],
                auth_state: ProviderAccountCopyAuthState::Authenticated,
                copied_at_ms: materialization.generated_at_ms,
                warning_seen: false,
            }),
        }
    }

    async fn acknowledge_profile(&self, id: String, request: RelayPeerRequest) {
        let RelayPeerRequest::UpdateLeasedAgentProfile {
            leased_agent_id,
            provider,
            account_profile,
            model,
            effort,
        } = request
        else {
            panic!("expected profile update only after confirmed installation")
        };
        let leased = crate::execution_lease::LeasedAgent::new(
            leased_agent_id,
            self.runtime
                .owned
                .agent_store
                .get_agent(&self.agent)
                .unwrap()
                .remote_execution()
                .unwrap()
                .execution_lease_id
                .clone(),
            self.agent.clone(),
            provider,
            account_profile,
            model,
            effort,
            None,
            None,
            "worker-session".into(),
            "worker-agent".into(),
            "worker-attachment".into(),
        );
        self.reply(
            id,
            RelayPeerResponse::LeasedAgentProfileUpdated {
                leased_agent: leased,
            },
        )
        .await;
    }
}

#[tokio::test]
async fn review_p2_profile_update_uses_the_confirmed_receiving_account() {
    crate::test_support::isolated_env_test!();
    let mut fixture = CopyFixture::new().await;
    let registry = &fixture.runtime.owned.provider_account_profiles;
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let materialization = registry
        .export_materialization(owner, "codex", &fixture.account)
        .unwrap();
    let expected = ProviderAccountCopyExpectation::from_materialization(&materialization).unwrap();
    let receipt = fixture.receipt(&materialization, "receiving-default");
    registry
        .record_confirmed_account_copy(
            owner,
            &expected,
            ProviderAccountMaterializationTargetKind::Worker,
            &fixture.worker.host_machine_id,
            &fixture.worker.daemon_id,
            "receiving-default",
            receipt,
        )
        .unwrap();
    let update = fixture.update();
    let (id, request) = fixture.request().await;
    assert!(
        matches!(&request, RelayPeerRequest::UpdateLeasedAgentProfile { account_profile, .. } if account_profile == "receiving-default"),
        "recorded remap must reach the worker profile update"
    );
    fixture.acknowledge_profile(id, request).await;
    let agent = update
        .await
        .unwrap()
        .expect("receiving-ID acknowledgement must be accepted");
    assert_eq!(
        agent.provider_account_profile(),
        fixture.account,
        "home must retain its source account ID"
    );
}

#[tokio::test]
async fn review_p2_rejected_placement_receipt_is_validated_again_on_retry() {
    crate::test_support::isolated_env_test!();
    rejected_receipt_retry(false).await;
}

#[tokio::test]
async fn review_p2_rejected_generation_receipt_is_validated_again_on_retry() {
    crate::test_support::isolated_env_test!();
    rejected_receipt_retry(true).await;
}

async fn rejected_receipt_retry(stale_generation: bool) {
    let mut fixture = CopyFixture::new().await;
    let before = fixture
        .runtime
        .owned
        .agent_store
        .get_agent(&fixture.agent)
        .unwrap();
    let update = fixture.update();
    let (id, request) = fixture.request().await;
    let RelayPeerRequest::EnsureRemoteProviderAccount {
        materialization, ..
    } = request
    else {
        panic!("expected account transfer")
    };
    let mut receipt = fixture.receipt(&materialization, &fixture.account);
    if stale_generation {
        receipt.copy.as_mut().unwrap().copied_at_ms = 0;
    } else {
        receipt.target_ref = "another-worker".into();
    }
    fixture
        .reply(
            id,
            RelayPeerResponse::RemoteProviderAccountEnsured {
                provider: "codex".into(),
                account_profile: fixture.account.clone(),
                copy: Some(receipt),
            },
        )
        .await;
    assert!(
        update.await.unwrap().is_err(),
        "forged copy receipt must reject the profile change"
    );
    assert_eq!(
        fixture
            .runtime
            .owned
            .agent_store
            .get_agent(&fixture.agent)
            .unwrap()
            .provider_account_profile(),
        before.provider_account_profile()
    );
    let update = fixture.update();
    let (id, retry) = fixture.request().await;
    let RelayPeerRequest::EnsureRemoteProviderAccount {
        materialization, ..
    } = retry
    else {
        panic!("rejected receipt cached successful installation and bypassed validation on retry")
    };
    let registry = fixture.runtime.owned.provider_account_profiles.clone();
    let profile = registry
        .get(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "codex",
            &fixture.account,
        )
        .unwrap();
    assert!(!profile.is_installed_at(
        ProviderAccountMaterializationTargetKind::Worker,
        &fixture.worker.daemon_id
    ));
    let receipt = fixture.receipt(&materialization, &fixture.account);
    fixture
        .reply(
            id,
            RelayPeerResponse::RemoteProviderAccountEnsured {
                provider: "codex".into(),
                account_profile: fixture.account.clone(),
                copy: Some(receipt),
            },
        )
        .await;
    let (id, request) = fixture.request().await;
    fixture.acknowledge_profile(id, request).await;
    assert_eq!(
        update.await.unwrap().unwrap().provider_account_profile(),
        fixture.account
    );
    let profile = registry
        .get(
            crate::session::DEFAULT_LOCAL_USER_ID,
            "codex",
            &fixture.account,
        )
        .unwrap();
    assert!(profile.is_installed_at(
        ProviderAccountMaterializationTargetKind::Worker,
        &fixture.worker.daemon_id
    ));
    assert!(profile
        .materializations
        .iter()
        .any(|status| status.copy.is_some()));
}

#[tokio::test]
async fn review_ack_lost_first_response_reconciles_the_production_receivers_copy() {
    crate::test_support::isolated_env_test!();
    let root = crate::test_support::TestWorktree::new("lost-copy-ack");
    let binary = root.path().join("opencode");
    std::fs::write(&binary, "#!/bin/sh\nprintf 'synthetic-opencode\\n'\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::env::set_var("CHARIOX_OPENCODE_BIN", binary);
    let mut fixture = CopyFixture::for_provider("opencode").await;
    fixture.worker.accept_remote_leases = true;
    let mut receiver = DaemonApp::bootstrap(fixture.worker.clone()).unwrap();
    let owner = crate::session::DEFAULT_LOCAL_USER_ID;
    let lease = crate::app::RemoteLeaseRuntime::new(&mut receiver)
        .create_execution_lease(
            &fixture.home.daemon_id,
            &fixture.session,
            &fixture.agent,
            false,
            owner,
        )
        .unwrap();
    let initial_account = receiver
        .provider_account_profile_registry()
        .get(owner, "codex", "default")
        .unwrap()
        .profile_id;
    let leased = crate::app::RemoteLeaseRuntime::new(&mut receiver)
        .create_leased_agent_from_base_directory(
            root.path(),
            &lease.id,
            "codex",
            &initial_account,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let mut binding = fixture
        .runtime
        .owned
        .agent_store
        .get_agent(&fixture.agent)
        .unwrap()
        .remote_execution()
        .unwrap()
        .clone();
    binding.execution_lease_id = lease.id;
    binding.leased_agent_id = leased.id;
    fixture
        ._app
        .lock()
        .await
        .agents_mut()
        .bind_remote_execution(&fixture.agent, binding)
        .unwrap();
    assert_eq!(
        fixture.runtime.owned.config_projection.snapshot().relay_url,
        fixture.home.relay_url
    );
    assert_eq!(
        fixture.relay.read().await.connected_relay_url(),
        fixture.home.relay_url
    );
    assert!(fixture
        .relay
        .read()
        .await
        .peer_public_key(&fixture.worker.daemon_id)
        .is_some());
    let mut first = fixture.update();
    let (id, request) = tokio::select! {
        response = &mut first => panic!("first profile change rejected before account transfer: {response:?}"),
        request = fixture.request() => request,
    };
    let RelayPeerRequest::EnsureRemoteProviderAccount {
        context,
        materialization,
    } = request
    else {
        panic!("first attempt must transfer the account")
    };
    let generation = materialization.generated_at_ms;
    let received = crate::app::RemoteLeaseRuntime::new(&mut receiver)
        .ensure_remote_provider_account(context, materialization)
        .unwrap();
    assert_eq!(received.auth_state, ProviderAccountAuthState::Authenticated);
    // The receiver committed successfully, but the first response is lost.
    crate::transport::relay_client::resolve_pending_peer_error_for_test(
        &fixture.relay,
        id,
        fixture.worker.daemon_id.clone(),
        chariox_relay::protocol::RelayError {
            code: "request_timeout".into(),
            message: "successful copy acknowledgement lost".into(),
            retryable: true,
        },
    )
    .await;
    assert!(first.await.unwrap().is_err());
    // Failed transport evicts the peer key; the fake relay models authenticated rediscovery.
    fixture.relay.write().await.remember_peer_public_key(
        &fixture.worker.daemon_id,
        fixture.worker.relay_public_key.clone(),
    );
    assert!(!fixture
        .runtime
        .owned
        .provider_account_profiles
        .get(owner, "opencode", &fixture.account)
        .unwrap()
        .is_installed_at(
            ProviderAccountMaterializationTargetKind::Worker,
            &fixture.worker.daemon_id
        ));
    // Retry exports a genuinely newer generation and goes through the real receiver again.
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let mut retry = fixture.update();
    let (id, request) = tokio::select! {
        response = &mut retry => panic!("retry rejected before account confirmation: {response:?}"),
        request = fixture.request() => request,
    };
    let RelayPeerRequest::EnsureRemoteProviderAccount {
        context,
        materialization,
    } = request
    else {
        panic!("lost acknowledgement must retry confirmation")
    };
    assert!(materialization.generated_at_ms > generation);
    let received = crate::app::RemoteLeaseRuntime::new(&mut receiver)
        .ensure_remote_provider_account(context, materialization)
        .unwrap();
    let copy = received
        .materializations
        .iter()
        .find(|status| status.copy.is_some())
        .unwrap()
        .clone();
    assert_eq!(
        copy.copy.as_ref().unwrap().copied_at_ms,
        generation,
        "receiver preserves its first login"
    );
    fixture
        .reply(
            id,
            RelayPeerResponse::RemoteProviderAccountEnsured {
                provider: "opencode".into(),
                account_profile: received.profile_id,
                copy: Some(copy),
            },
        )
        .await;
    let worker_request = tokio::select! {
        response = &mut retry => panic!("home rejected the committed first copy on retry: {response:?}"),
        request = fixture.request() => request,
    };
    let RelayPeerRequest::UpdateLeasedAgentProfile {
        leased_agent_id,
        provider,
        account_profile,
        model,
        effort,
    } = worker_request.1
    else {
        panic!("confirmed copy must proceed to the worker profile update")
    };
    let updated = crate::app::RemoteLeaseRuntime::new(&mut receiver)
        .update_leased_agent_profile(&leased_agent_id, provider, account_profile, model, effort)
        .unwrap();
    assert_eq!(updated.account_profile, fixture.account);
    fixture
        .reply(
            worker_request.0,
            RelayPeerResponse::LeasedAgentProfileUpdated {
                leased_agent: updated,
            },
        )
        .await;
    assert_eq!(
        retry.await.unwrap().unwrap().provider_account_profile(),
        fixture.account
    );
    let profile = fixture
        .runtime
        .owned
        .provider_account_profiles
        .get(owner, "opencode", &fixture.account)
        .unwrap();
    assert!(profile.is_installed_at(
        ProviderAccountMaterializationTargetKind::Worker,
        &fixture.worker.daemon_id
    ));
}
