//! MP-08/MP-09/MP-10/MP-11: real source router + authenticated encrypted relay
//! regression drill with synthetic credentials and a fake official CLI.
use super::support::*;
use crate::config::{CredentialVaultBackend, PersistedCloudRelayProfile};
use crate::runtime::router::CommandRouter;
use chariox_relay::auth::{
    RelayAction, RelayAuthVerifier, RelaySubjectKind, RelayTokenClaims, ScopedTokenVerifier,
};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

struct Environment(Vec<(&'static str, Option<std::ffi::OsString>)>);
impl Drop for Environment {
    fn drop(&mut self) {
        for (name, value) in &self.0 {
            if let Some(value) = value {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
    }
}
fn config(root: &Path, name: &str) -> DaemonConfig {
    let mut config = DaemonConfig::for_tests();
    config.daemon_id = format!("account-copy-{name}");
    config.host_machine_id = format!("account-copy-machine-{name}");
    config.user_config_path = root.join(name).join("config.toml");
    config.user_config.state.path = Some(root.join(name).join("state.db").display().to_string());
    config.user_config.history.operational.path =
        Some(root.join(name).join("history.db").display().to_string());
    config.user_config.credential_vault.backend = CredentialVaultBackend::CharioxEncrypted;
    config.user_config.credential_vault.path =
        root.join(name).join("vault.enc").display().to_string();
    config.relay_token = Some(format!("synthetic-{name}"));
    config.relay_heartbeat_ms = 50;
    config.cloud_relay = Some(PersistedCloudRelayProfile {
        api_url: "https://cloud.invalid".into(),
        user_id: "copy-owner".into(),
        realm_id: "copy-realm".into(),
        kernel_id: Some(config.daemon_id.clone()),
        machine_id: Some(config.host_machine_id.clone()),
        kernel_credential: Some("synthetic-cloud-enrollment".into()),
        kernel_public_key_thumbprint: Some(
            crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key),
        ),
        ..Default::default()
    });
    config.with_session_history_root(root.join(name).join("sessions"))
}
fn claim(config: &DaemonConfig) -> RelayTokenClaims {
    RelayTokenClaims {
        issuer: "account-copy-fixture".into(),
        subject: config.daemon_id.clone(),
        subject_kind: RelaySubjectKind::Kernel,
        realm_id: "copy-realm".into(),
        allowed_actions: vec![
            RelayAction::DaemonRegister,
            RelayAction::DaemonHeartbeat,
            RelayAction::ClientMetadataRead,
            RelayAction::PeerRequest,
            RelayAction::PacketRoute,
        ],
        allowed_targets: None,
        issued_at_ms: crate::session::unix_epoch_ms(),
        expires_at_ms: crate::session::unix_epoch_ms() + 3_600_000,
        token_id: config.daemon_id.clone(),
        account_id: None,
        organization_id: None,
        user_id: Some("copy-owner".into()),
        device_id: None,
        machine_id: Some(config.host_machine_id.clone()),
        client_id: None,
        session_id: None,
        public_key_thumbprint: Some(crate::runtime::terminal_pairings::public_key_thumbprint(
            &config.relay_public_key,
        )),
        entitlements_version: None,
    }
}
async fn wait_for_copy_peer(registry: Arc<RwLock<chariox_relay::server::RelayRegistry>>, id: &str) {
    for _ in 0..200 {
        if registry
            .read()
            .await
            .daemon_in_realm("copy-realm", id)
            .is_some()
        {
            return;
        }
        sleep(Duration::from_millis(25)).await;
    }
    panic!("account-copy peer did not register in its enrolled realm");
}
async fn source_request(
    router: &CommandRouter,
    request: serde_json::Value,
) -> Result<serde_json::Value, DaemonError> {
    let request: LocalDaemonRequest =
        serde_json::from_value(request).expect("public account copy must parse");
    let command = KernelCommand::from_local_request(
        format!("account-copy-test-{}", rand::random::<u64>()),
        None,
        None,
        &request,
    );
    router
        .dispatch(command, request)
        .await
        .map(|response| serde_json::to_value(response).unwrap())
}
fn assert_code(result: Result<serde_json::Value, DaemonError>, code: &str, canary: &str) {
    let error = result.expect_err(code);
    let public = crate::transport::kernel_protocol::map_kernel_error(&error);
    assert_eq!(public.code, code, "unexpected safe error: {error}");
    assert!(!format!("{error:?} {error} {public:?}").contains(canary));
}
fn assert_native_files_clean(path: &Path, canary: &str) {
    if !path.exists() {
        return;
    }
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            assert_native_files_clean(&path, canary);
        } else {
            assert_ne!(
                path.file_name().unwrap(),
                ".credentials.json",
                "a Vault copy must never create native token files"
            );
            assert!(
                !std::fs::read(path)
                    .unwrap()
                    .windows(canary.len())
                    .any(|bytes| bytes == canary.as_bytes()),
                "credential reached native profile/history"
            );
        }
    }
}

#[test]
fn account_copy_source_and_encrypted_install_drill() {
    crate::test_support::isolated_env_test!();
    run_async_with_large_test_stack("account-copy", || async {
        let _relay = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let root = PathBuf::from(std::env::var_os("CHARIOX_HOME").unwrap());
        let names = ["HOME", "CHARIOX_CLAUDE_BIN", "CHARIOX_LOG_DIR"];
        let _restore = Environment(
            names
                .iter()
                .map(|name| (*name, std::env::var_os(name)))
                .collect(),
        );
        std::env::set_var("HOME", root.join("native-home"));
        std::env::set_var("CHARIOX_LOG_DIR", root.join("logs"));
        let fake = root.join("claude");
        std::fs::write(&fake, format!(r#"#!/bin/sh
if [ "$1" = --version ]; then echo 2.2.0; exit 0; fi
if [ -f '{root}/interrupt' ]; then
 rm '{root}/interrupt'
 mv '{root}/target/provider-accounts.json' '{root}/target/provider-accounts.json.backup'
 mkdir '{root}/target/provider-accounts.json'
fi
if [ -f '{root}/reject' ]; then
 printf '{{"type":"result","is_error":true,"api_error_status":401,"result":"%s"}}\n' "$CLAUDE_CODE_OAUTH_TOKEN"
 exit 1
fi
printf '%s\n' '{{"type":"result","subtype":"success","is_error":false,"num_turns":1,"usage":{{"output_tokens":7}},"result":"CHARIOX_AUTH_OK"}}'
"#, root=root.display())).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("CHARIOX_CLAUDE_BIN", &fake);
        let mut source = config(&root, "source");
        let mut target = config(&root, "target");
        let cloud = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cloud_url = format!("http://{}", cloud.local_addr().unwrap());
        source.cloud_relay.as_mut().unwrap().api_url = cloud_url.clone();
        target.cloud_relay.as_mut().unwrap().api_url = cloud_url;
        let peers:Vec<_>=[&source,&target].into_iter().map(|config|serde_json::json!({
            "targetId":config.daemon_id, "peer":{
                "relayRealmId":"copy-realm", "machineId":config.host_machine_id,
                "kernelId":config.daemon_id,"relayPublicKey":config.relay_public_key,
                "keyThumbprint":crate::runtime::terminal_pairings::public_key_thumbprint(&config.relay_public_key)
            }
        })).collect();
        let omit_target = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let omit_for_server = omit_target.clone();
        let (stop_cloud, mut stopped_cloud) = watch::channel(false);
        let cloud_task = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            loop {
                tokio::select! {
                    _=stopped_cloud.changed()=>break,
                    accepted=cloud.accept()=>{
                        let (mut socket,_)=accepted.unwrap();
                        let mut buffer=[0u8;8192];let _=socket.read(&mut buffer).await.unwrap();
                        let peers=if omit_for_server.load(std::sync::atomic::Ordering::SeqCst) { &peers[..1] } else { &peers[..] };
                        let body=serde_json::json!({"peers":peers}).to_string();
                        let reply=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body);
                        socket.write_all(reply.as_bytes()).await.unwrap();
                    }
                }
            }
        });
        let verifier = ScopedTokenVerifier::new(
            BTreeMap::from([
                (source.relay_token.clone().unwrap(), claim(&source)),
                (target.relay_token.clone().unwrap(), claim(&target)),
            ]),
            BTreeMap::new(),
            None,
        );
        let relay = RelayServer::with_auth_verifier(
            RelayConfig {
                host: "127.0.0.1".into(),
                port: 0,
                shared_token: None,
            },
            RelayAuthVerifier::ScopedToken(verifier),
        );
        let listener = relay.bind_listener().await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let registry = relay.registry();
        let (stop_relay, stopped_relay) = oneshot::channel();
        let relay_task = tokio::spawn(async move {
            relay
                .run_listener_until(listener, async {
                    let _ = stopped_relay.await;
                })
                .await
                .unwrap();
        });
        source.relay_url = Some(url.clone());
        target.relay_url = Some(url.clone());
        let source_app = Arc::new(Mutex::new(DaemonApp::bootstrap(source.clone()).unwrap()));
        let target_app = Arc::new(Mutex::new(DaemonApp::bootstrap(target.clone()).unwrap()));
        let source_state = source_app.lock().await.relay_client_state();
        let target_state = target_app.lock().await.relay_client_state();
        let source_profiles = source_app
            .lock()
            .await
            .provider_account_profile_registry()
            .clone();
        let target_profiles = target_app
            .lock()
            .await
            .provider_account_profile_registry()
            .clone();
        let source_router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
            source_app, 16,
        ));
        let target_router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
            target_app, 16,
        ));
        let (stop_source, stopped_source) = watch::channel(false);
        let (stop_target, stopped_target) = watch::channel(false);
        let source_task = tokio::spawn(run_daemon_relay_connector_with_router_and_static_relay(
            source_router.clone(),
            source_state,
            stopped_source,
            url.clone(),
            source.relay_token.clone().unwrap(),
        ));
        let target_task = tokio::spawn(run_daemon_relay_connector_with_router_and_static_relay(
            target_router.clone(),
            target_state,
            stopped_target,
            url,
            target.relay_token.clone().unwrap(),
        ));
        wait_for_copy_peer(registry.clone(), &source.daemon_id).await;
        wait_for_copy_peer(registry, &target.daemon_id).await;
        let source_vault = PathBuf::from(&source.user_config.credential_vault.path);
        let target_vault = PathBuf::from(&target.user_config.credential_vault.path);
        crate::secret::unlock_chariox_encrypted_vault(
            &source_vault,
            "fixture-pass",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
        crate::secret::unlock_chariox_encrypted_vault(
            &target_vault,
            "fixture-pass",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
        let canary = "sk-ant-oat01-SYNTHETIC-ACCOUNT-COPY-NEVER-PUBLISH";
        let profile = source_profiles
            .create_managed("local", "claude", "Source")
            .unwrap();
        crate::provider::store_provider_account_credential(
            &source,
            "local",
            "claude",
            &profile.profile_id,
            canary,
            false,
        )
        .unwrap();
        source_profiles
            .update_observation(
                "local",
                "claude",
                &profile.profile_id,
                crate::account_profile::ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let second = source_profiles
            .create_managed("local", "claude", "Second")
            .unwrap();
        crate::provider::store_provider_account_credential(
            &source,
            "local",
            "claude",
            &second.profile_id,
            canary,
            false,
        )
        .unwrap();
        source_profiles
            .update_observation(
                "local",
                "claude",
                &second.profile_id,
                crate::account_profile::ProviderAccountAuthState::Authenticated,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let source_before = std::fs::read(source.account_profile_registry_path()).unwrap();
        let mut request = serde_json::json!({"CopyProviderAccountToKernel":{
            "provider":"claude", "account_profile":profile.profile_id, "operation_id":"copy-1",
            "target_kernel_id":target.daemon_id, "target_key_thumbprint":crate::runtime::terminal_pairings::public_key_thumbprint(&target.relay_public_key),"replace":false
        }});
        let denied_request: LocalDaemonRequest = serde_json::from_value(request.clone()).unwrap();
        let mut denied_command =
            KernelCommand::from_local_request("foreign-owner", None, None, &denied_request);
        denied_command.caller.user_id = Some("another-owner".into());
        assert_code(
            source_router
                .dispatch(denied_command, denied_request)
                .await
                .map(|response| serde_json::to_value(response).unwrap()),
            "provider_account_copy_unauthorized",
            canary,
        );
        let result = source_request(&source_router, request.clone())
            .await
            .unwrap();
        assert!(!result.to_string().contains(canary));
        let target_id = result["ProviderAccountCopied"]["receipt"]["account_profile"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(target_id, profile.profile_id);
        let target_handle =
            crate::provider::provider_account_credential_id("local", "claude", &target_id);
        assert_ne!(
            target_handle,
            crate::provider::provider_account_credential_id("local", "claude", &profile.profile_id)
        );
        let policy = crate::credential::CharioxCredentialRegistry::user()
            .unwrap()
            .get(&target_handle)
            .unwrap()
            .unwrap();
        assert_eq!(
            policy.allowed_uses,
            vec![crate::config::UserCredentialUse::Provider]
        );
        assert_eq!(
            policy.injection,
            crate::config::UserCredentialInjectionConfig::Provider
        );

        assert_eq!(
            target_profiles
                .get("local", "claude", &target_id)
                .unwrap()
                .auth_state,
            crate::account_profile::ProviderAccountAuthState::Authenticated
        );
        assert!(crate::provider::resolve_provider_account_credentials(
            &target, "local", "claude", &target_id
        )
        .unwrap()
        .iter()
        .any(|(_, value)| value == canary));
        let mut interrupted = request.clone();
        interrupted["CopyProviderAccountToKernel"]["account_profile"] =
            second.profile_id.clone().into();
        interrupted["CopyProviderAccountToKernel"]["operation_id"] = "interrupted-copy".into();
        let count = target_profiles.list("local", Some("claude")).unwrap().len();
        std::fs::write(root.join("interrupt"), b"interrupt").unwrap();
        assert_code(
            source_request(&source_router, interrupted.clone()).await,
            "provider_account_copy_install_failed",
            canary,
        );
        assert_eq!(
            target_profiles.list("local", Some("claude")).unwrap().len(),
            count,
            "failed publication exposed an authenticated profile"
        );
        std::fs::remove_dir(target.account_profile_registry_path()).unwrap();
        std::fs::rename(
            target
                .account_profile_registry_path()
                .with_extension("json.backup"),
            target.account_profile_registry_path(),
        )
        .unwrap();
        assert!(
            source_request(&source_router, interrupted).await.is_ok(),
            "same operation must recover after Vault storage but before profile publication"
        );
        assert_eq!(
            source_request(&source_router, request.clone())
                .await
                .unwrap(),
            result,
            "lost acknowledgement retry must be idempotent"
        );
        request["CopyProviderAccountToKernel"]["operation_id"] = "copy-2".into();
        assert_code(
            source_request(&source_router, request.clone()).await,
            "provider_account_copy_conflict",
            canary,
        );
        request["CopyProviderAccountToKernel"]["replace"] = true.into();
        assert!(source_request(&source_router, request.clone())
            .await
            .is_ok());
        request["CopyProviderAccountToKernel"]["operation_id"] = "copy-3".into();
        std::fs::write(root.join("reject"), b"reject").unwrap();
        assert_code(
            source_request(&source_router, request.clone()).await,
            "provider_account_copy_rejected",
            canary,
        );
        std::fs::remove_file(root.join("reject")).unwrap();
        crate::secret::lock_chariox_encrypted_vault(&target_vault).unwrap();
        assert_code(
            source_request(&source_router, request.clone()).await,
            "provider_account_copy_vault_locked",
            canary,
        );
        crate::secret::unlock_chariox_encrypted_vault(
            &target_vault,
            "fixture-pass",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
        assert!(source_request(&source_router, request.clone())
            .await
            .is_ok());
        // Target admission also sees authenticated wrong-owner/realm and forged binding cases.
        let base_binding = serde_json::json!({
            "operation_id":"direct-peer", "provider":"claude", "source_profile_id":profile.profile_id,
            "source_kernel_id":source.daemon_id, "target_kernel_id":target.daemon_id,
            "source_key_thumbprint":crate::runtime::terminal_pairings::public_key_thumbprint(&source.relay_public_key),
            "target_key_thumbprint":crate::runtime::terminal_pairings::public_key_thumbprint(&target.relay_public_key), "replace":true
        });
        for case in [
            "owner",
            "realm",
            "target-key",
            "source-key",
            "target-id",
            "unbound",
            "expired",
        ] {
            let mut binding = base_binding.clone();
            let mut identity = chariox_relay::protocol::RelayCallerIdentity {
                realm_id: "copy-realm".into(),
                subject: source.daemon_id.clone(),
                subject_kind: RelaySubjectKind::Kernel,
                expires_at_ms: crate::session::unix_epoch_ms() + 3_600_000,
                token_id: Some("fixture".into()),
                user_id: Some("copy-owner".into()),
                public_key_thumbprint: Some(
                    crate::runtime::terminal_pairings::public_key_thumbprint(
                        &source.relay_public_key,
                    ),
                ),
            };
            match case {
                "owner" => identity.user_id = Some("another-owner".into()),
                "realm" => identity.realm_id = "another-realm".into(),
                "target-key" => binding["target_key_thumbprint"] = "c".repeat(64).into(),
                "source-key" => binding["source_key_thumbprint"] = "c".repeat(64).into(),
                "target-id" => binding["target_kernel_id"] = "another-target".into(),
                "unbound" => identity.public_key_thumbprint = None,
                "expired" => identity.expires_at_ms = 1,
                _ => unreachable!(),
            }
            let peer:RelayPeerRequest=serde_json::from_value(serde_json::json!({"kind":"install_provider_account_copy","binding":binding,"credential":canary})).unwrap();
            assert!(!format!("{peer:?}").contains(canary));
            let (outgoing, _priority, _events) = RelayOutgoingSender::channel(8);
            let response=crate::transport::relay_client::peer_requests::send_authenticated_peer_request_for_test(
                &target_router,&Arc::new(RwLock::new(RelayClientState::default())),&outgoing,
                &source.daemon_id,identity,&source.relay_private_key,&target.relay_public_key,peer).await;
            let public = match response {
                Ok(response) => serde_json::to_value(response).unwrap(),
                Err(error) => serde_json::to_value(error).unwrap(),
            };
            assert!(!public.to_string().contains(canary));
            assert!(
                public.to_string().contains("unauthorized"),
                "case {case}: {public}"
            );
        }
        omit_target.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_code(
            source_request(&source_router, request.clone()).await,
            "provider_account_copy_unauthorized",
            canary,
        );
        omit_target.store(false, std::sync::atomic::Ordering::SeqCst);
        let good = request.clone();
        request["CopyProviderAccountToKernel"]["target_key_thumbprint"] = "c".repeat(64).into();
        assert_code(
            source_request(&source_router, request).await,
            "provider_account_copy_unauthorized",
            canary,
        );
        let mut missing = good.clone();
        missing["CopyProviderAccountToKernel"]["account_profile"] = "missing".into();
        assert_code(
            source_request(&source_router, missing).await,
            "provider_account_copy_profile",
            canary,
        );
        assert_eq!(
            std::fs::read(source.account_profile_registry_path()).unwrap(),
            source_before,
            "copy changed source/default/observation"
        );
        assert!(crate::provider::resolve_provider_account_credentials(
            &source,
            "local",
            "claude",
            &profile.profile_id
        )
        .unwrap()
        .iter()
        .any(|(_, value)| value == canary));
        assert_native_files_clean(
            &source
                .private_runtime_state_root()
                .join("provider-accounts"),
            canary,
        );
        assert_native_files_clean(
            &target
                .private_runtime_state_root()
                .join("provider-accounts"),
            canary,
        );
        assert_native_files_clean(&root.join("logs"), canary);
        for config in [&source, &target] {
            assert_native_files_clean(
                &config.private_runtime_state_root().join("sessions"),
                canary,
            );
            for entry in std::fs::read_dir(config.private_runtime_state_root()).unwrap() {
                let path = entry.unwrap().path();
                if path.is_file()
                    && path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("history.db")
                {
                    assert!(
                        !std::fs::read(path)
                            .unwrap()
                            .windows(canary.len())
                            .any(|bytes| bytes == canary.as_bytes()),
                        "credential reached operational history"
                    );
                }
            }
        }

        let _ = stop_target.send(true);
        tokio::time::timeout(Duration::from_secs(5), target_task)
            .await
            .unwrap()
            .unwrap();
        let unavailable = good;

        assert_code(
            source_request(&source_router, unavailable).await,
            "provider_account_copy_unavailable",
            canary,
        );
        let _ = stop_source.send(true);
        tokio::time::timeout(Duration::from_secs(5), source_task)
            .await
            .unwrap()
            .unwrap();
        let _ = stop_cloud.send(true);
        cloud_task.await.unwrap();
        let _ = stop_relay.send(());
        relay_task.await.unwrap();
        crate::secret::lock_chariox_encrypted_vault(&source_vault).unwrap();
        crate::secret::lock_chariox_encrypted_vault(&target_vault).unwrap();
    });
}

#[test]
fn account_copy_encrypted_target_entry_point_requires_same_owner() {
    crate::test_support::isolated_env_test!();
    run_async_with_large_test_stack("account-copy-peer-entry", || async {
        let _relay = relay_client_test_guard().await;
        let _home = RelayTestHome::new();
        let root = PathBuf::from(std::env::var_os("CHARIOX_HOME").unwrap());
        let _restore = Environment(vec![("HOME", std::env::var_os("HOME"))]);
        std::env::set_var("HOME", root.join("native-home"));
        let source = config(&root, "source");
        let target = config(&root, "target");
        let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
            Arc::new(Mutex::new(DaemonApp::bootstrap(target.clone()).unwrap())),
            16,
        ));
        let canary = "synthetic-encrypted-account-copy-canary";
        let request = serde_json::json!({"kind":"install_provider_account_copy","binding":{
            "operation_id":"wrong-owner", "provider":"claude", "source_profile_id":"work-exact",
            "source_kernel_id":source.daemon_id,"target_kernel_id":target.daemon_id,
            "source_key_thumbprint":crate::runtime::terminal_pairings::public_key_thumbprint(&source.relay_public_key),
            "target_key_thumbprint":crate::runtime::terminal_pairings::public_key_thumbprint(&target.relay_public_key),"replace":false
        },"credential":canary});
        let encrypted = relay_crypto::encrypt_payload_for_peer(
            &source.relay_private_key,
            &target.relay_public_key,
            &serde_json::to_vec(&request).unwrap(),
        )
        .unwrap();
        let identity = chariox_relay::protocol::RelayCallerIdentity {
            realm_id: "copy-realm".into(),
            subject: source.daemon_id.clone(),
            subject_kind: RelaySubjectKind::Kernel,
            expires_at_ms: crate::session::unix_epoch_ms() + 60000,
            token_id: Some("fixture".into()),
            user_id: Some("another-owner".into()),
            public_key_thumbprint: Some(crate::runtime::terminal_pairings::public_key_thumbprint(
                &source.relay_public_key,
            )),
        };
        let (outgoing, _priority, _events) = RelayOutgoingSender::channel(8);
        let result = crate::transport::relay_client::peer_requests::handle_daemon_peer_request(
            &router,
            &Arc::new(RwLock::new(RelayClientState::default())),
            &outgoing,
            &source.daemon_id,
            Some(identity),
            encrypted,
        )
        .await;
        let public = if let Some(error) = result.error {
            serde_json::to_value(error).unwrap()
        } else {
            let decrypted = relay_crypto::decrypt_payload_for_private_key(
                &source.relay_private_key,
                &result.encrypted_response.unwrap(),
            )
            .unwrap();
            serde_json::from_slice::<serde_json::Value>(&decrypted.plaintext).unwrap()
        };
        assert!(!public.to_string().contains(canary));
        assert_eq!(
            public["code"], "provider_account_copy_unauthorized",
            "MP-11 account copy must reach its same-owner encrypted admission boundary"
        );
    });
}
