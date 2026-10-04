//! MD-3/MD-4: focused MCP admission and opt-in real host-browser drill.
use super::*;
use crate::local::{KernelBrowserCommand, KernelBrowserInput, KernelBrowserRequest};
use serde_json::{json, Value};

fn run_test(test: fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(64 * 1024 * 1024)
                .enable_all()
                .build()
                .unwrap()
                .block_on(test());
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn focus(router: &CommandRouter, session: &str, agent: &str) {
    let request = LocalDaemonRequest::FocusAgent(FocusAgentRequest {
        session_id: session.into(),
        agent_id: agent.into(),
    });
    router
        .dispatch(
            KernelCommand::from_local_request("MD-3-focus", None, None, &request),
            request,
        )
        .await
        .unwrap();
}

#[test]
fn kernel_browser_mcp_focus_load_and_revocation() {
    run_test(|| Box::pin(focus_check()));
}
async fn focus_check() {
    let workspace = crate::test_support::TestWorktree::new("md3-browser-focus");
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, first) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let second = spawn_test_agent(&mut app, session.id(), "second", "dev-stub");
    let first_run = launch_test_provider(
        &mut app,
        session.id(),
        first.id(),
        "dev-stub",
        "dev-stub",
        "default",
    );
    let second_run = launch_test_provider(
        &mut app,
        session.id(),
        second.id(),
        "dev-stub",
        "dev-stub",
        "default",
    );
    let first_token = first_run.runtime_mcp_auth_token().unwrap();
    let second_token = second_run.runtime_mcp_auth_token().unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    focus(&router, session.id(), first.id()).await;
    let names = |token: &str| {
        router
            .runtime_tool_specs_for_auth_token(token)
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>()
    };
    assert!(names(first_token).contains(&"chariox.load_kernel_browser".into()));
    assert!(!names(first_token).contains(&"chariox.kernel_browser".into()));
    router
        .dispatch_authenticated_runtime_tool_call(
            first_token,
            "chariox.load_kernel_browser",
            json!({}),
        )
        .await
        .unwrap();
    assert!(names(first_token).contains(&"chariox.kernel_browser".into()));
    assert!(names(first_token).contains(&"chariox.kernel_browser_paste_secret".into()));
    focus(&router, session.id(), second.id()).await;
    assert!(!names(first_token).contains(&"chariox.load_kernel_browser".into()));
    assert!(router
        .dispatch_authenticated_runtime_tool_call(
            first_token,
            "chariox.kernel_browser",
            json!({"command":{"op":"stop"}})
        )
        .await
        .is_err());
    assert!(router
        .dispatch_authenticated_runtime_tool_call(
            first_token,
            "chariox.kernel_browser_paste_secret",
            json!({})
        )
        .await
        .is_err());
    assert!(router
        .dispatch_authenticated_runtime_tool_call(
            second_token,
            "chariox.kernel_browser",
            json!({"command":{"op":"stop"}})
        )
        .await
        .is_err());
    router
        .dispatch_authenticated_runtime_tool_call(
            second_token,
            "chariox.load_kernel_browser",
            json!({}),
        )
        .await
        .unwrap();
    router
        .dispatch_authenticated_runtime_tool_call(
            second_token,
            "chariox.kernel_browser",
            json!({"command":{"op":"stop"}}),
        )
        .await
        .unwrap();
    let request = LocalDaemonRequest::KernelBrowser(KernelBrowserRequest {
        command: KernelBrowserCommand::Stop,
    });
    let command = KernelCommand::from_local_request_with_source(
        "MD-2-foreign",
        KernelCommandSource::RelayPeer,
        None,
        None,
        &request,
    );
    assert!(router.dispatch(command, request).await.is_err());
    router.runtime_state.shutdown_cleanup().await.unwrap();
}

#[test]
#[ignore = "MD-4: requires a sandbox-capable Linux or macOS host, explicit disposable CHARIOX_HOME and native Chromium"]
fn kernel_browser_linux_integration_drill() {
    match std::env::var("CHARIOX_MD4_PHASE").as_deref() {
        Ok("before") => run_test(|| Box::pin(live_check())),
        Ok("after") => run_test(|| Box::pin(restart_check())),
        _ => {
            // Real process exit releases every kernel task and durable owner.
            // In-process router drop is insufficient while actors retain clones.
            for phase in ["before", "after"] {
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--ignored", "--exact", "runtime::router::tests::kernel_browser::kernel_browser_linux_integration_drill", "--nocapture"])
                    .env("CHARIOX_MD4_PHASE", phase).status().unwrap();
                assert!(status.success(), "MD-4 kernel subprocess {phase} failed");
            }
            let root =
                std::path::PathBuf::from(std::env::var_os("CHARIOX_MD4_DRILL_ROOT").unwrap());
            std::fs::write(root.join("MD4-PASS.txt"), "MD-4: kernel routing, focused MCP input, PNG, bounded stream, stale refs, browser crash, real kernel process restart and last-tab close passed; MD-5 Vault injection, retired-value scrubbing and protected pixels passed; dev-stub provider, no model/Cloud/Web/TUI acceptance; native platform identified by replay receipt.\n").unwrap();
        }
    }
}

fn drill_config(home: &std::path::Path) -> DaemonConfig {
    // MD-4/MD-5: normal product identity persistence, never a handcrafted key.
    // The replay's clean environment binds this loader to disposable CHARIOX_HOME.
    let mut config = DaemonConfig::load_from_env();
    config.provider_process_orphan_ttl_ms = u64::MAX;
    config.user_config.state.path = Some(home.join("state.db").display().to_string());
    config.user_config_path = home.join("config.toml");
    config.user_config.credential_vault.path = home.join("vault/vault.json").display().to_string();
    config
}

async fn restart_check() {
    let root = std::path::PathBuf::from(std::env::var_os("CHARIOX_MD4_DRILL_ROOT").unwrap());
    let home = std::path::PathBuf::from(std::env::var_os("CHARIOX_HOME").unwrap());
    let restored: Value =
        serde_json::from_slice(&std::fs::read(root.join("MD4-RESTART.json")).unwrap()).unwrap();
    let config = drill_config(&home);
    let router = CommandRouter::with_interactive_capacity(
        Arc::new(Mutex::new(DaemonApp::bootstrap(config).unwrap())),
        4,
    );
    use futures_util::FutureExt;
    let outcome = std::panic::AssertUnwindSafe(async {
        let state = human(&router, KernelBrowserCommand::State).await;
        let id = restored["tab_id"].as_str().unwrap();
        assert!(state["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tab| tab["tab_id"] == id && tab["url"] == restored["url"]));
        let generation = state["generation"].as_u64().unwrap();
        assert!(generation > restored["generation"].as_u64().unwrap());
        human(
            &router,
            KernelBrowserCommand::Screenshot {
                tab_id: id.into(),
                generation,
            },
        )
        .await;
        human(
            &router,
            KernelBrowserCommand::Close {
                tab_id: id.into(),
                generation,
            },
        )
        .await;
        assert!(human(&router, KernelBrowserCommand::State).await["tabs"]
            .as_array()
            .unwrap()
            .is_empty());
    })
    .catch_unwind()
    .await;
    router.runtime_state.shutdown_cleanup().await.unwrap();
    if let Err(error) = outcome {
        std::panic::resume_unwind(error);
    }
}

async fn human(router: &CommandRouter, command: KernelBrowserCommand) -> Value {
    let request = LocalDaemonRequest::KernelBrowser(KernelBrowserRequest { command });
    let response = router
        .dispatch(
            KernelCommand::from_local_request("MD-4-browser", None, None, &request),
            request,
        )
        .await
        .expect("MD-4 host browser request");
    let LocalDaemonResponse::KernelBrowser { result } = response else {
        panic!("MD-4 response variant");
    };
    result
}
async fn live_check() {
    use base64::Engine;
    use std::io::{Read, Write};
    let root = std::path::PathBuf::from(
        std::env::var_os("CHARIOX_MD4_DRILL_ROOT").expect("MD-4 external disposable root"),
    );
    assert!(root.is_absolute());
    let home = std::path::PathBuf::from(
        std::env::var_os("CHARIOX_HOME").expect("MD-4 explicit CHARIOX_HOME"),
    );
    assert!(home.starts_with(&root));
    assert!(
        unsafe { libc::geteuid() } != 0,
        "MD-4 normal Unix user required"
    );
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let fixture = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    fixture.set_nonblocking(true).unwrap();
    let address = fixture.local_addr().unwrap();
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopped = done.clone();
    let fixture_thread = std::thread::spawn(move || {
        while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
            match fixture.accept() {
                Ok((mut socket, _)) => {
                    socket
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut request = [0; 4096];
                    let _ = socket.read(&mut request);
                    let body = "<!doctype html><title>MD-4 fixture</title><input style='position:absolute;left:20px;top:20px;width:300px;height:40px' oninput='document.querySelector(\"p\").textContent=this.value'><p style='margin-top:100px'>MD-4 ready</p><input id='password' type='password' style='position:absolute;left:20px;top:180px;width:300px;height:40px' oninput='document.querySelector(\"p\").textContent=this.value'>";
                    let _ = write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    });
    let config = drill_config(&home);
    std::fs::create_dir_all(home.join("vault")).unwrap();
    crate::secret::create_chariox_encrypted_vault_for_test(
        std::path::Path::new(&config.user_config.credential_vault.path),
        "synthetic-md5-passphrase",
    )
    .unwrap();
    crate::secret::unlock_chariox_encrypted_vault(
        std::path::Path::new(&config.user_config.credential_vault.path),
        "synthetic-md5-passphrase",
        crate::secret::VaultUnlockLease::KernelShutdown,
    )
    .unwrap();
    crate::credential::CharioxCredentialRegistry::user()
        .unwrap()
        .upsert(crate::config::UserCredentialConfig {
            id: "md5-login".into(),
            description: None,
            source: crate::config::UserCredentialSourceConfig::Vault {
                key: "md5-login".into(),
            },
            allowed_hosts: vec!["127.0.0.1".into()],
            allowed_uses: vec![crate::config::UserCredentialUse::Browser],
            injection: crate::config::UserCredentialInjectionConfig::Browser,
            metadata: None,
        })
        .unwrap();
    crate::credential::CharioxCredentialRegistry::user()
        .unwrap()
        .upsert(crate::config::UserCredentialConfig {
            id: "md5-wrong-host".into(),
            description: None,
            source: crate::config::UserCredentialSourceConfig::Vault {
                key: "md5-login".into(),
            },
            allowed_hosts: vec!["example.com".into()],
            allowed_uses: vec![crate::config::UserCredentialUse::Browser],
            injection: crate::config::UserCredentialInjectionConfig::Browser,
            metadata: None,
        })
        .unwrap();
    let secret_service = crate::secret::RuntimeSecretService::with_vault_config(
        vec![],
        &config.user_config.credential_vault,
    )
    .unwrap();
    secret_service
        .set_vault_secret("md5-login", "synthetic-md5-protected-value")
        .unwrap();
    let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new(
            workspace.to_string_lossy(),
            workspace.to_string_lossy(),
        ))
        .unwrap();
    let run = launch_test_provider(
        &mut app,
        session.id(),
        agent.id(),
        "dev-stub",
        "dev-stub",
        "default",
    );
    let token = run.runtime_mcp_auth_token().unwrap().to_string();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let result = std::panic::AssertUnwindSafe(async {
        let opened = human(
            &router,
            KernelBrowserCommand::Open {
                url: "about:blank".into(),
            },
        )
        .await;
        let id = opened["tab_id"].as_str().unwrap().to_string();
        let generation = opened["generation"].as_u64().unwrap();
        human(
            &router,
            KernelBrowserCommand::Navigate {
                tab_id: id.clone(),
                generation,
                url: format!("http://{address}/fixture"),
            },
        )
        .await;
        focus(&router, session.id(), agent.id()).await;
        router
            .dispatch_authenticated_runtime_tool_call(
                &token,
                "chariox.load_kernel_browser",
                json!({}),
            )
            .await
            .unwrap();
        for input in [
            KernelBrowserInput::Click { x: 80, y: 40 },
            KernelBrowserInput::Text {
                text: "MD-4 MCP typed".into(),
            },
        ] {
            router.dispatch_authenticated_runtime_tool_call(&token, "chariox.kernel_browser", json!({"command":KernelBrowserCommand::Input { tab_id:id.clone(),generation,input }})).await.unwrap();
        }
        let snapshot = human(
            &router,
            KernelBrowserCommand::Snapshot {
                tab_id: id.clone(),
                generation,
            },
        )
        .await;
        assert!(snapshot.to_string().contains("MD-4 MCP typed"));
        let frame = human(
            &router,
            KernelBrowserCommand::Screenshot {
                tab_id: id.clone(),
                generation,
            },
        )
        .await;
        let png = base64::engine::general_purpose::STANDARD
            .decode(frame["data_base64"].as_str().unwrap())
            .unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        std::fs::write(root.join("screenshot.png"), png).unwrap();
        let stream = human(
            &router,
            KernelBrowserCommand::Subscribe {
                tab_id: id.clone(),
                generation,
            },
        )
        .await;
        let stream_id = stream["subscription_id"].as_str().unwrap().to_string();
        let mut saw_frame = false;
        for _ in 0..50 {
            let value = human(
                &router,
                KernelBrowserCommand::Poll {
                    subscription_id: stream_id.clone(),
                    generation,
                },
            )
            .await;
            if !value["frame"].is_null() {
                saw_frame = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(saw_frame);
        // MD-5: normal Vault/MCP path, including hostile plaintext echo on input.
        let observed = human(
            &router,
            KernelBrowserCommand::Snapshot {
                tab_id: id.clone(),
                generation,
            },
        )
        .await;
        let node = observed["snapshot"]["dom_nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["attributes"]["id"] == "password")
            .unwrap();
        let secret_args = json!({"credential_id":"md5-login", "tab_id":id, "generation":generation, "document_id":observed["snapshot"]["document_id"], "node_ref":node["node_ref"]});
        // MD-5: reject wrong-host handles and stale/non-password references before input.
        for (field, value) in [
            ("credential_id", json!("md5-wrong-host")),
            ("document_id", json!("stale-document")),
            (
                "node_ref",
                observed["snapshot"]["dom_nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|node| {
                        node["node_name"] == "INPUT" && node["attributes"]["id"] != "password"
                    })
                    .unwrap()["node_ref"]
                    .clone(),
            ),
        ] {
            let mut invalid = secret_args.clone();
            invalid[field] = value;
            assert!(
                router
                    .dispatch_authenticated_runtime_tool_call(
                        &token,
                        "chariox.kernel_browser_paste_secret",
                        invalid
                    )
                    .await
                    .is_err(),
                "MD-5: invalid secret target must be refused"
            );
        }
        let paste = router
            .dispatch_authenticated_runtime_tool_call(
                &token,
                "chariox.kernel_browser_paste_secret",
                secret_args,
            )
            .await
            .unwrap();
        assert_eq!(paste.payload, json!({"inserted":true}));
        let protected = human(
            &router,
            KernelBrowserCommand::Snapshot {
                tab_id: id.clone(),
                generation,
            },
        )
        .await;
        assert!(
            !protected
                .to_string()
                .contains("synthetic-md5-protected-value"),
            "MD-5: secret echo must be scrubbed"
        );
        assert!(protected.to_string().contains("[redacted]"));
        let masked = human(
            &router,
            KernelBrowserCommand::Screenshot {
                tab_id: id.clone(),
                generation,
            },
        )
        .await;
        let png = base64::engine::general_purpose::STANDARD
            .decode(masked["data_base64"].as_str().unwrap())
            .unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        std::fs::write(root.join("MD5-masked.png"), png).unwrap();
        // Delete via the existing product lifecycle. Old page echoes remain scrubbed.
        router
            .runtime_state
            .revoke_vault_observation_values("md5-login")
            .await
            .unwrap();
        secret_service.delete_vault_secret("md5-login").unwrap();
        let retired = human(
            &router,
            KernelBrowserCommand::Snapshot {
                tab_id: id.clone(),
                generation,
            },
        )
        .await;
        assert!(
            !retired
                .to_string()
                .contains("synthetic-md5-protected-value"),
            "MD-5: retired echoes remain scrubbed"
        );
        assert!(retired.to_string().contains("[redacted]"));
        let protected_stream = human(
            &router,
            KernelBrowserCommand::Subscribe {
                tab_id: id.clone(),
                generation,
            },
        )
        .await;
        let protected_id = protected_stream["subscription_id"]
            .as_str()
            .unwrap()
            .to_string();
        let mut saw_mask = false;
        for _ in 0..50 {
            let polled = human(
                &router,
                KernelBrowserCommand::Poll {
                    subscription_id: protected_id.clone(),
                    generation,
                },
            )
            .await;
            if !polled["frame"].is_null() {
                assert_eq!(polled["frame"]["mime_type"], "image/png");
                saw_mask = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            saw_mask,
            "MD-5: protected frame source must produce opaque frames"
        );
        human(
            &router,
            KernelBrowserCommand::Unsubscribe {
                subscription_id: protected_id,
                generation,
            },
        )
        .await;
        // Fault the exact browser selected by this lane-owned profile's singleton PID.
        let profile = router
            .runtime_state
            .kernel_browser_profile_root(DEFAULT_LOCAL_USER_ID);
        let lock = std::fs::read_link(profile.join("profile/SingletonLock")).unwrap();
        let pid = lock
            .to_string_lossy()
            .rsplit('-')
            .next()
            .unwrap()
            .parse::<i32>()
            .unwrap();
        assert_eq!(unsafe { libc::kill(pid, libc::SIGKILL) }, 0);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let recovered = human(&router, KernelBrowserCommand::State).await;
        assert!(recovered["generation"].as_u64().unwrap() > generation);
        assert!(recovered["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tab| tab["tab_id"] == id));
        let stale = LocalDaemonRequest::KernelBrowser(KernelBrowserRequest {
            command: KernelBrowserCommand::Screenshot {
                tab_id: id.clone(),
                generation,
            },
        });
        assert!(router
            .dispatch(
                KernelCommand::from_local_request("MD-4-stale", None, None, &stale),
                stale
            )
            .await
            .is_err());
        let stale_stream = LocalDaemonRequest::KernelBrowser(KernelBrowserRequest {
            command: KernelBrowserCommand::Poll {
                subscription_id: stream_id,
                generation,
            },
        });
        assert!(router
            .dispatch(
                KernelCommand::from_local_request("MD-4-stale-stream", None, None, &stale_stream),
                stale_stream
            )
            .await
            .is_err());
        std::fs::write(root.join("MD4-RESTART.json"), serde_json::to_vec(&json!({"tab_id": id, "generation": recovered["generation"], "url":format!("http://{address}/fixture")})).unwrap()).unwrap();
    });
    use futures_util::FutureExt;
    let outcome = result.catch_unwind().await;
    router.runtime_state.shutdown_cleanup().await.unwrap();
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    fixture_thread.join().unwrap();
    if let Err(error) = outcome {
        std::panic::resume_unwind(error);
    }
}
