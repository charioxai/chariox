//! Linux kernel-access live drill fixture: real listeners and dev-stub PTY, no accounts.
use super::*;

#[test]
#[ignore = "live subprocess entry point"]
fn ka_validation_live_server() {
    let Ok(root) = std::env::var("CHARIOX_KA_LIVE_ROOT") else {
        return;
    };
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all()
        .thread_stack_size(32 * 1024 * 1024).build().unwrap().block_on(async {
        let root = PathBuf::from(root);
        let tcp = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let addr = tcp.local_addr().unwrap();
        let mcp = StdTcpListener::bind("127.0.0.1:0").unwrap();
        let mut config = daemon_config_for_runtime_mcp_listener(&mcp);
        config.local_socket_path = root.join("run/kernel.sock");
        config.user_config.state.path = Some(root.join("control/state.db").display().to_string());
        config.user_config_path = root.join("config/config.toml");
        config.user_config.credential_vault.path = root.join("vault.json").display().to_string();
        config.user_config.credential_vault.backend = crate::config::CredentialVaultBackend::CharioxEncrypted;
        config.user_config.credential_vault.unlock_policy = crate::config::CredentialVaultUnlockPolicy::KernelInit;
        let passkey = std::fs::read_to_string(root.join("test-passkey")).unwrap();
        crate::secret::create_chariox_encrypted_vault_for_test(&root.join("vault.json"), &passkey).unwrap();
        crate::secret::unlock_chariox_encrypted_vault(root.join("vault.json"), &passkey, crate::secret::VaultUnlockLease::KernelShutdown).unwrap();
        let mut app = crate::test_support::bootstrap_authenticated_app(config).unwrap();
        let worktree = root.join("workspace");
        std::fs::create_dir_all(&worktree).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(worktree.display().to_string(), worktree.display().to_string()).with_alias("ka-live")).unwrap();
        app.agents_mut().set_agent_runtime_profile_with_account_profile(agent.id(), "dev-stub", Some("native-tui-idle".into()), Some("default".into()), Some("default".into()), Default::default()).unwrap();
        let run = app.launch_provider(crate::provider::LaunchProviderRequest::new(session.id(), "dev-stub", "dev-stub", "default", "native-tui-idle").with_agent_id(agent.id())).unwrap();
        let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(Arc::new(Mutex::new(app)), 32));
        let runtime = router.runtime_state();
        let (auth, _token_guard) = KernelLocalAuth::for_local_kernel(None, Ok(addr)).unwrap();
        let metadata = serde_json::json!({"endpoint": format!("ws://{addr}/kernel"), "session":session.id(), "agent":agent.id(), "run":run.id(), "mcp":format!("http://127.0.0.1:{}/mcp", mcp.local_addr().unwrap().port()), "runtime_token":run.runtime_mcp_auth_token()});
        std::fs::write(root.join("ready.json"), metadata.to_string()).unwrap();
        let (stop, stopped) = oneshot::channel();
        let controls = root.clone();
        tokio::spawn(async move {
            let mut waiting = Vec::new();
            loop {
                sleep(Duration::from_millis(20)).await;
                let Ok(action) = std::fs::read_to_string(controls.join("command")) else {continue;};
                std::fs::remove_file(controls.join("command")).unwrap();
                match action.as_str() {
                    "stop" => { let _ = stop.send(()); break; }
                    "critical" => waiting.push(runtime.raise_critical_approval_for_test(session.id(), &format!("live-critical-{}", waiting.len())).await),
                    "reuse" => runtime.control_access_for_test("reuse", &controls.join("vault.json")).await,
                    _ => panic!("unknown candidate control"),
                }
                std::fs::write(controls.join("ack"), action).unwrap();
            }
        });
        run_kernel_websocket_server_with_bound_listeners(router, adopt_std_listener(tcp,"tcp").unwrap(), adopt_std_listener(mcp,"mcp").unwrap(), auth, async {let _ = stopped.await;}).await.unwrap();
    });
}
