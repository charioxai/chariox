//! MP-08 / MP-11 focused protocol-478 drill: actual terminal WebSocket frames,
//! ordinary kernel router and provider-native export with synthetic credentials.
use super::*;
use crate::config::PersistedCloudRelayProfile;
use crate::local::ManagedEnvironmentProviderAccountSelection;

#[test]
fn provider_account_portability_websocket_drill() {
    crate::test_support::isolated_env_test!();
    let _guard = crate::env_lock::lock();
    let root = std::env::temp_dir().join(format!("chx-kp-{:x}", rand::random::<u64>()));
    std::fs::create_dir(&root).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let _cleanup = Cleanup(root.clone());
    std::env::set_var("CHARIOX_HOME", &root);
    tokio::runtime::Builder::new_multi_thread().enable_all().thread_stack_size(16 * 1024 * 1024).build().unwrap().block_on(async {
        let tcp = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let addr = tcp.local_addr().unwrap();
        let mcp = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let mut config = daemon_config_for_runtime_mcp_listener(&mcp);
        // The isolated child already owns TMPDIR; avoid nesting the socket path
        // below the profile root, which exceeds Unix sun_path on long builders.
        config.local_socket_path = std::env::temp_dir().join("kp.sock");
        config.user_config.state.path = Some(root.join("state.db").to_string_lossy().into());
        config.user_config_path = root.join("config.toml");
        config.cloud_relay = Some(PersistedCloudRelayProfile {
            api_url: "http://127.0.0.1:1".into(), user_id: "owner".into(), realm_id: "realm-a".into(),
            kernel_id: Some(config.daemon_id.clone()), kernel_credential: Some("synthetic-kernel-only".into()),
            cloud_session_token: None, ..Default::default()
        });
        let app = DaemonApp::bootstrap(config).unwrap();
        let profiles = app.provider_account_profile_registry();
        let mut accounts = Vec::new();
        let mut paths = Vec::new();
        for provider in ["codex", "claude", "opencode"] {
            let profile = profiles.create_managed(DEFAULT_LOCAL_USER_ID, provider, "Synthetic").unwrap();
            let environment = profiles.resolve_environment(DEFAULT_LOCAL_USER_ID, provider, &profile.profile_id).unwrap();
            let (path, bytes): (PathBuf, &[u8]) = match provider {
                "codex" => (Path::new(&environment["CODEX_HOME"]).join("auth.json"), br#"{"token":"synthetic-codex-only"}"#),
                "claude" => (Path::new(&environment["CLAUDE_CONFIG_DIR"]).join(".credentials.json"), br#"{"claudeAiOauth":{"refreshToken":"synthetic-claude-only"}}"#),
                _ => (Path::new(&environment["XDG_DATA_HOME"]).join("opencode/auth.json"), br#"{"synthetic":{"type":"api","key":"synthetic-opencode-only"}}"#),
            };
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, bytes).unwrap();
            paths.push(path);
            accounts.push(ManagedEnvironmentProviderAccountSelection {provider: provider.into(), account_profile: profile.profile_id});
        }
        let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(Arc::new(Mutex::new(app)), 16));
        let (stop, stopped) = oneshot::channel();
        let server = tokio::spawn(run_kernel_websocket_server_with_bound_listeners(
            router, adopt_std_listener(tcp, "tcp").unwrap(), adopt_std_listener(mcp, "mcp").unwrap(),
            KernelLocalAuth::LocalToken(Arc::new(local_auth::LocalTokenAuth::new("synthetic-terminal-only".into()))),
            async { let _ = stopped.await; }
        ));
        let mut upgrade = format!("ws://{addr}/kernel").into_client_request().unwrap();
        upgrade.headers_mut().insert(AUTHORIZATION, HeaderValue::from_static("Bearer synthetic-terminal-only"));
        let (mut socket, _) = connect_async(upgrade).await.unwrap();
        let result = async {
            for (index, path) in paths.iter().enumerate() {
                for missing in [false, true] {
                    if missing {std::fs::remove_file(path).unwrap();}
                    let request_id = format!("portability-{index}-{missing}");
                    let request = serde_json::json!({"type":"request", "request_id":request_id, "request":{"PreflightProviderAccountPortability":{"providerAccounts":{"kind":"selected", "accounts":&accounts[index..]}}}});
                    socket.send(Message::Text(request.to_string().into())).await.unwrap();
                    let frame: Value = loop {
                        let message = timeout(Duration::from_secs(5), socket.next()).await.unwrap().unwrap().unwrap();
                        let Message::Text(text) = message else {continue};
                        let frame: Value = serde_json::from_str(&text).unwrap();
                        if frame["type"] == "response" && frame["request_id"] == request_id {break frame;}
                    };
                    if !missing {
                        assert!(frame["error"].is_null());
                        assert_eq!(frame["response"], serde_json::json!({"ProviderAccountPortabilityPreflightPassed":{}}));
                    } else {
                        assert!(frame["response"].is_null());
                        assert!(frame["error"]["message"].as_str().unwrap().contains("no transferable credentials"));
                        let text = frame.to_string();
                        for marker in ["synthetic-codex-only", "synthetic-claude-only", "synthetic-opencode-only"] {
                            assert!(!text.contains(marker));
                        }
                    }
                }
            }
            socket.close(None).await.unwrap();
        }.await;
        let _ = stop.send(());
        timeout(Duration::from_secs(5), server).await.unwrap().unwrap().unwrap();
        result
    });
}
