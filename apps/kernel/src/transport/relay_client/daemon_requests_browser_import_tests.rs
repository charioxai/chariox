use super::*;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::config::DaemonConfig;
use crate::session::{CreateSessionRequest, DEFAULT_LOCAL_USER_ID};
use crate::DaemonApp;
use base64::Engine as _;
use chariox_relay::auth::RelaySubjectKind;
use serde_json::json;
use tokio::sync::Mutex;

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

struct ImportClient {
    router: CommandRouter,
    sequence: AtomicU64,
    cache: Arc<CommandResultCache>,
    identity: RelayCallerIdentity,
    private_key: String,
    selection: Value,
    _scratch: Scratch,
}

impl ImportClient {
    fn new() -> Self {
        let config = DaemonConfig::for_tests();
        let scratch = Scratch(
            std::path::PathBuf::from(config.user_config.state.path.as_ref().unwrap())
                .parent()
                .unwrap()
                .to_path_buf(),
        );
        let mut app = DaemonApp::bootstrap(config).unwrap();
        let session = crate::app::KernelSessionService::new(&mut app)
            .create_session(CreateSessionRequest::new("import-relay", "worktree"))
            .unwrap()
            .0;
        let attachment = crate::app::KernelSessionService::new(&mut app)
            .attach(AttachRequest::new(
                session.id(),
                "import-client",
                ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 2);
        let state = router.runtime_state();
        state
            .start_room_environment(
                session.id(),
                crate::session::CanonicalViewport::new(800, 600, 1, 800, 600).unwrap(),
            )
            .unwrap();
        state
            .transition_room_environment(session.id(), crate::session::EnvironmentLifecycle::Ready)
            .unwrap();
        let environment = state
            .reconcile_room_environment_controller_tabs(
                session.id(),
                vec![crate::session::EnvironmentTabObservation {
                    runtime_target_id: "target-1".into(),
                    document_id: "doc-1".into(),
                    url: "https://example.com/".into(),
                    title: "fixture".into(),
                }],
                Some("target-1"),
            )
            .unwrap();
        let private_key = relay_crypto::generate_private_key_base64();
        let public_key = relay_crypto::public_key_from_private_key_base64(&private_key).unwrap();
        Self {
            router,
            sequence: AtomicU64::new(0),
            cache: Arc::new(CommandResultCache::default()),
            identity: RelayCallerIdentity {
                realm_id: "test-realm".into(),
                subject: "import-client".into(),
                subject_kind: RelaySubjectKind::Client,
                expires_at_ms: crate::session::unix_epoch_ms() + 120_000,
                token_id: None,
                user_id: Some(DEFAULT_LOCAL_USER_ID.into()),
                public_key_thumbprint: Some(
                    crate::runtime::terminal_pairings::public_key_thumbprint(&public_key),
                ),
            },
            private_key,
            selection: json!({"session_id": session.id(), "attachment_id": attachment.id(),
                "environment_id": environment.environment_id, "runtime_generation": environment.runtime_generation,
                "tab_id": environment.tabs[0].tab_id, "document_revision": environment.tabs[0].document_revision,
                "source_store_id": "0", "domains": ["example.com"], "partition_sites": [], "overwrite": false}),
            _scratch: scratch,
        }
    }

    async fn send(&self, command_id: &str, request: Value) -> Result<Value, RelayError> {
        self.send_with_identity(command_id, request, Some(self.identity.clone()))
            .await
    }

    async fn send_with_identity(
        &self,
        command_id: &str,
        request: Value,
        identity: Option<RelayCallerIdentity>,
    ) -> Result<Value, RelayError> {
        let is_import = [
            "PrepareBrowserImport",
            "ApproveBrowserImport",
            "ClaimBrowserImportSource",
            "AuthorizeBrowserImportSource",
            "CancelBrowserImport",
        ]
        .iter()
        .any(|kind| request.get(kind).is_some());
        let peer =
            relay_crypto::public_key_from_private_key_base64(&self.router.relay_private_key())
                .unwrap();
        let encrypted = relay_crypto::encrypt_payload_for_peer(
            &self.private_key,
            &peer,
            &serde_json::to_vec(&json!({"command_id": command_id, "request": request})).unwrap(),
        )
        .unwrap();
        let request_nonce = encrypted.nonce.clone();
        let result = handle_daemon_request(
            &self.router,
            &self.sequence,
            identity,
            encrypted,
            &self.cache,
        )
        .await;
        if let Some(error) = result.error {
            return Err(error);
        }
        let decrypted = relay_crypto::decrypt_payload_for_private_key(
            &self.private_key,
            &result.encrypted_response.unwrap(),
        )
        .unwrap();
        let response: Value = serde_json::from_slice(&decrypted.plaintext).unwrap();
        if is_import {
            assert_eq!(response["request_nonce"], request_nonce);
            assert!(response.get("response").is_some());
            Ok(response["response"].clone())
        } else {
            assert!(response.get("request_nonce").is_none());
            Ok(response)
        }
    }

    async fn start_read(&self) -> String {
        let result = self
            .send(
                "prepare",
                json!({"PrepareBrowserImport": {"selection": self.selection}}),
            )
            .await
            .unwrap();
        let id = result["BrowserImportConsent"]["request_id"]
            .as_str()
            .unwrap()
            .to_string();
        for (command, kind, status) in [
            ("approve", "ApproveBrowserImport", "approved"),
            ("claim", "ClaimBrowserImportSource", "source_claimed"),
        ] {
            let result = self
                .send(
                    command,
                    json!({kind: {"request_id": id, "selection": self.selection}}),
                )
                .await
                .unwrap();
            assert_eq!(result["BrowserImportConsent"]["status"], status);
        }
        id
    }
}

fn run(test: impl AsyncFnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(test());
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn browser_import_relay_replay_cannot_authorize_after_cancellation() {
    run(async || {
        let client = ImportClient::new();
        let id = client.start_read().await;
        let authorize = json!({"AuthorizeBrowserImportSource": {"request_id": id, "selection": client.selection}});
        assert_eq!(
            client.send("authorize", authorize.clone()).await.unwrap()["BrowserImportConsent"]
                ["status"],
            "source_authorized"
        );
        client.send("cancel", json!({"CancelBrowserImport": {"request_id": id,
            "session_id": client.selection["session_id"], "attachment_id": client.selection["attachment_id"]}})).await.unwrap();
        assert!(
            client
                .send("fresh-authorize", authorize.clone())
                .await
                .is_err(),
            "fresh request observes cancellation"
        );
        assert!(
            client.send("authorize", authorize).await.is_err(),
            "replaying the same encrypted command must also observe cancellation"
        );
    });
}

#[test]
fn browser_import_relay_replay_cannot_authorize_after_navigation() {
    run(async || {
        let client = ImportClient::new();
        let id = client.start_read().await;
        let authorize = json!({"AuthorizeBrowserImportSource": {"request_id": id, "selection": client.selection}});
        client.send("authorize", authorize.clone()).await.unwrap();
        client
            .router
            .runtime_state()
            .reconcile_room_environment_controller_tabs(
                client.selection["session_id"].as_str().unwrap(),
                vec![crate::session::EnvironmentTabObservation {
                    runtime_target_id: "target-1".into(),
                    document_id: "doc-2".into(),
                    url: "https://example.com/next".into(),
                    title: "next".into(),
                }],
                Some("target-1"),
            )
            .unwrap();
        assert!(
            client.send("authorize", authorize).await.is_err(),
            "navigation invalidates even a previously successful command ID"
        );
    });
}

#[test]
fn browser_import_relay_replay_cannot_repeat_claim_or_change_caller() {
    run(async || {
        let mut client = ImportClient::new();
        let id = client.start_read().await;
        for (command, kind) in [
            ("approve", "ApproveBrowserImport"),
            ("claim", "ClaimBrowserImportSource"),
        ] {
            assert!(
                client
                    .send(
                        command,
                        json!({kind: {"request_id": id, "selection": client.selection}})
                    )
                    .await
                    .is_err(),
                "{kind} is a one-use transition even with the same command ID"
            );
        }
        let authorize = json!({"AuthorizeBrowserImportSource": {"request_id": id, "selection": client.selection}});
        client.send("authorize", authorize.clone()).await.unwrap();
        client.identity.user_id = Some("another-user".into());
        assert!(
            client.send("authorize", authorize).await.is_err(),
            "another user must not obtain a cached authorization"
        );
    });
}

#[test]
fn browser_import_relay_requires_possession_of_the_bound_sender_key() {
    run(async || {
        let mut client = ImportClient::new();
        let id = client.start_read().await;
        let authorize = json!({"AuthorizeBrowserImportSource": {"request_id": id, "selection": client.selection}});
        client
            .send("owner-authorize", authorize.clone())
            .await
            .unwrap();
        // Keep the authenticated relay identity, but encrypt with another key.
        // This models a copied client token without its bound private key.
        let owner_key = client.private_key.clone();
        client.private_key = relay_crypto::generate_private_key_base64();
        assert!(
            client.send("other-key", authorize.clone()).await.is_err(),
            "a token without its bound private key must not authorize cookie reads"
        );
        client.private_key = owner_key;
        client.send("owner-again", authorize).await.unwrap();
    });
}

#[test]
fn browser_import_relay_requires_live_client_identity_before_reserving_consent() {
    run(async || {
        let client = ImportClient::new();
        let prepare = json!({"PrepareBrowserImport": {"selection": client.selection}});
        let mut invalid = vec![None];
        for expiry in [0, crate::session::unix_epoch_ms().saturating_sub(1)] {
            let mut identity = client.identity.clone();
            identity.expires_at_ms = expiry;
            invalid.push(Some(identity));
        }
        for kind in [
            RelaySubjectKind::Service,
            RelaySubjectKind::Machine,
            RelaySubjectKind::Kernel,
        ] {
            let mut identity = client.identity.clone();
            identity.subject_kind = kind;
            invalid.push(Some(identity));
        }
        for thumbprint in [None, Some(String::new())] {
            let mut identity = client.identity.clone();
            identity.public_key_thumbprint = thumbprint;
            invalid.push(Some(identity));
        }
        for identity in invalid {
            let error = client
                .send_with_identity("prepare", prepare.clone(), identity)
                .await
                .expect_err("invalid identity cannot reserve consent");
            assert_eq!(error.code, "unauthorized");
        }
        client.start_read().await;
    });
}

#[test]
fn browser_import_relay_does_not_require_bound_keys_for_ordinary_client_reads() {
    run(async || {
        let mut client = ImportClient::new();
        client.identity.public_key_thumbprint = None;
        client
            .send(
                "list",
                serde_json::to_value(LocalDaemonRequest::ListSessions(
                    crate::local::ListSessionsRequest,
                ))
                .unwrap(),
            )
            .await
            .unwrap();
    });
}

#[test]
fn browser_import_encrypted_delivery_revalidates_sender_and_quarantines_before_controller_route() {
    run(async || {
        let client = ImportClient::new();
        let id = client.start_read().await;
        client
            .send(
                "authorize",
                json!({"AuthorizeBrowserImportSource": {"request_id": id, "selection": client.selection}}),
            )
            .await
            .unwrap();
        let generated_value = format!("generated-{}", crate::session::unix_epoch_ms());
        let payload = json!([{"name":"session","value":generated_value}]).to_string();
        let plaintext = serde_json::to_vec(&json!({"browser_import_delivery":{
            "request_id":id,"selection":client.selection,
            "payload_base64":base64::engine::general_purpose::STANDARD.encode(payload.as_bytes())
        }}))
        .unwrap();
        let peer =
            relay_crypto::public_key_from_private_key_base64(&client.router.relay_private_key())
                .unwrap();

        let mut service = client.identity.clone();
        service.subject_kind = RelaySubjectKind::Service;
        let denied = handle_daemon_request(
            &client.router,
            &client.sequence,
            Some(service),
            relay_crypto::encrypt_payload_for_peer(&client.private_key, &peer, &plaintext).unwrap(),
            &client.cache,
        )
        .await;
        assert_eq!(denied.error.unwrap().code, "unauthorized");
        assert!(client
            .router
            .runtime_state()
            .ensure_browser_import_execution_allowed(
                client.selection["session_id"].as_str().unwrap()
            )
            .is_ok());

        let routed = handle_daemon_request(
            &client.router,
            &client.sequence,
            Some(client.identity.clone()),
            relay_crypto::encrypt_payload_for_peer(&client.private_key, &peer, &plaintext).unwrap(),
            &client.cache,
        )
        .await;
        assert_eq!(routed.error.unwrap().code, "browser_import_failed");
        assert!(client
            .router
            .runtime_state()
            .ensure_browser_import_execution_allowed(
                client.selection["session_id"].as_str().unwrap()
            )
            .is_err());
    });
}
