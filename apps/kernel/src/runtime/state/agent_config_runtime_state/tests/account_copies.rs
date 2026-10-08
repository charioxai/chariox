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
    relay: Arc<RwLock<RelayClientState>>,
    requests: mpsc::Receiver<RelayEnvelope>,
}

impl CopyFixture {
    async fn new() -> Self {
        let mut home = crate::config::DaemonConfig::for_tests();
        home.relay_url = Some("ws://127.0.0.1:1".into());
        home.relay_token = Some("synthetic-relay".into());
        let mut worker = crate::config::DaemonConfig::for_tests();
        worker.daemon_id = "copy-worker".into();
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
                "codex",
                "Selected copy",
            )
            .unwrap();
        let environment = registry
            .resolve_environment(
                crate::session::DEFAULT_LOCAL_USER_ID,
                "codex",
                &profile.profile_id,
            )
            .unwrap();
        std::fs::write(
            std::path::Path::new(&environment["CODEX_HOME"]).join("auth.json"),
            br#"{"tokens":{"refresh_token":"synthetic-copy-login"}}"#,
        )
        .unwrap();
        Self {
            _app: app,
            runtime,
            home,
            worker,
            session,
            agent,
            account: profile.profile_id,
            relay,
            requests,
        }
    }

    fn update(&self) -> tokio::task::JoinHandle<Result<crate::agent::AgentInstance, DaemonError>> {
        let runtime = self.runtime.clone();
        let session = self.session.clone();
        let agent = self.agent.clone();
        let account = self.account.clone();
        tokio::spawn(async move {
            runtime
                .update_agent_profile(
                    &session,
                    &agent,
                    crate::session::DEFAULT_LOCAL_USER_ID,
                    Some("codex".into()),
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
            "copy-lease".into(),
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
