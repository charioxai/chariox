//! MP-08/MP-10/MP-11: real authority checks; native browser proof is opt-in.
use super::*;

#[test]
fn mdaccess_kernel_scope_lifecycle_visibility_and_revocation() {
    run_test(|| Box::pin(check(false)));
}

#[test]
fn mdaccess_cross_kernel_tool_names_both_kernels_and_badges_window() {
    run_test(|| {
        Box::pin(async {
            let workspace = crate::test_support::TestWorktree::new("mdaccess-cross-kernel");
            let config = DaemonConfig::for_tests();
            let window_kernel = config.daemon_id.clone();
            let mut app = DaemonApp::bootstrap(config).unwrap();
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(workspace.session_request())
                .unwrap();
            let run = launch_test_provider(
                &mut app,
                session.id(),
                agent.id(),
                "dev-stub",
                "dev-stub",
                "default",
            );
            app.agents()
                .bind_remote_execution(
                    agent.id(),
                    crate::agent::RemoteAgentBinding {
                        worker_kernel_id: "execution-kernel".into(),
                        worker_machine_id: "worker-machine".into(),
                        execution_lease_id: "lease".into(),
                        leased_agent_id: "worker-agent".into(),
                        active_worker_provider_run_id: Some(run.id().into()),
                        relay_url: None,
                        relay_token: None,
                        relay_peer_protocol_version: Some(
                            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                        ),
                    },
                )
                .unwrap();
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
            focus(&router, session.id(), agent.id()).await;
            let root = std::env::temp_dir().join(format!(
                "chariox-mdaccess-cross-{:032x}",
                rand::random::<u128>()
            ));
            std::fs::create_dir(&root).unwrap();
            router
                .runtime_state()
                .install_kernel_browser_fixture(agent.owner_user_id(), &root);
            let state = human(&router, KernelBrowserCommand::State).await;
            assert_eq!(state["kernel_id"], window_kernel);
            // MP-08: a leased agent cannot retain owning-kernel browser focus.
            assert!(state["focused_agent_kernel_id"].is_null());
            assert_eq!(state["reachable_by_focused_agent"], false);
            let error = tool(
                &router,
                run.runtime_mcp_auth_token().unwrap(),
                "chariox.load_kernel_browser",
                json!({}),
            )
            .await
            .unwrap_err();
            assert!(matches!(
                error,
                crate::DaemonError::UserDomainRefused {
                    reason: crate::error::UserDomainRefusalReason::NotGranted
                }
            ));
            router.runtime_state().shutdown_cleanup().await.unwrap();
            std::fs::remove_dir_all(root).unwrap();
        })
    });
}

#[test]
#[cfg(any(target_os = "macos", all(target_os = "linux", target_env = "gnu")))]
fn mdaccess_cloud_owner_window_alias_keeps_collaborators_separate() {
    run_test(|| {
        Box::pin(async {
            let workspace = crate::test_support::TestWorktree::new("mdaccess-owner-alias");
            let mut config = DaemonConfig::for_tests();
            config.cloud_relay = Some(crate::config::PersistedCloudRelayProfile {
                user_id: "cloud-owner".into(),
                ..Default::default()
            });
            let mut app = DaemonApp::bootstrap(config).unwrap();
            let (session, agent) = crate::app::KernelSessionService::new(&mut app)
                .create_session(workspace.session_request())
                .unwrap();
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
            focus(&router, session.id(), agent.id()).await;
            let request = LocalDaemonRequest::KernelBrowser(KernelBrowserRequest {
                command: KernelBrowserCommand::ListGrants,
            });
            let mut owner = terminal_command("mdaccess-cloud-owner", &request);
            owner.caller.user_id = Some("cloud-owner".into());
            let LocalDaemonResponse::KernelBrowser { result: grants } =
                terminal(&router, owner, request.clone()).await.unwrap()
            else {
                panic!("grant snapshot")
            };
            assert_eq!(grants["grants"][0]["agent_id"], agent.id());
            let mut collaborator = terminal_command("mdaccess-collaborator", &request);
            collaborator.caller.user_id = Some("collaborator".into());
            let LocalDaemonResponse::KernelBrowser { result: grants } =
                terminal(&router, collaborator, request).await.unwrap()
            else {
                panic!("grant snapshot")
            };
            assert!(grants["grants"].as_array().unwrap().is_empty());
            // A native App view uses the same Cloud/local alias when projecting
            // reachability, without aliasing another collaborator's registry.
            router
                .runtime_state()
                .app_control()
                .user_views()
                .open(
                    "cloud-owner",
                    crate::runtime::app_views::AppViewBinding {
                        owner: "cloud-owner".into(),
                        installation: "fixture".into(),
                        generation: 1,
                        panel: Default::default(),
                        logical_tab: None,
                    },
                    "fixture",
                )
                .unwrap();
            let request =
                LocalDaemonRequest::ListUserAppViews(crate::local::ListUserAppViewsRequest {});
            let mut owner = terminal_command("mdaccess-cloud-window", &request);
            owner.caller.user_id = Some("cloud-owner".into());
            let LocalDaemonResponse::UserAppViewsListed { views } =
                terminal(&router, owner, request).await.unwrap()
            else {
                panic!("view projection")
            };
            assert_eq!(views.len(), 1);
            assert!(views[0].access.as_ref().unwrap().reachable_by_focused_agent);
            router.runtime_state().shutdown_cleanup().await.unwrap();
        })
    });
}

#[test]
#[ignore = "MP-10: sandbox-capable native Linux browser and disposable CHARIOX_MDACCESS_DRILL_ROOT required"]
fn mdaccess_native_two_agent_held_turn_drill() {
    run_test(|| Box::pin(check(true)));
}

async fn check(native: bool) {
    let workspace = crate::test_support::TestWorktree::new("mdaccess-authority");
    let root = if native {
        let root =
            std::path::PathBuf::from(std::env::var_os("CHARIOX_MDACCESS_DRILL_ROOT").unwrap());
        assert!(root.is_absolute());
        assert_ne!(
            unsafe { libc::geteuid() },
            0,
            "MP-10: normal Unix user required"
        );
        root
    } else {
        std::env::temp_dir().join(format!("chariox-mdaccess-{:032x}", rand::random::<u128>()))
    };
    std::fs::create_dir_all(&root).unwrap();
    let config = if native {
        let home = std::path::PathBuf::from(std::env::var_os("CHARIOX_HOME").unwrap());
        assert!(home.starts_with(&root));
        let config = drill_config(&home);
        std::fs::create_dir_all(home.join("vault")).unwrap();
        crate::secret::create_chariox_encrypted_vault_for_test(
            std::path::Path::new(&config.user_config.credential_vault.path),
            "mdaccess-fixture",
        )
        .unwrap();
        crate::secret::unlock_chariox_encrypted_vault(
            std::path::Path::new(&config.user_config.credential_vault.path),
            "mdaccess-fixture",
            crate::secret::VaultUnlockLease::KernelShutdown,
        )
        .unwrap();
        config
    } else {
        DaemonConfig::for_tests()
    };
    let fixture = if native {
        Some(payment_fixture().await)
    } else {
        None
    };
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, first) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let second = spawn_test_agent(&mut app, session.id(), "child", "dev-stub");
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
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let runtime = router.runtime_state();
    if !native {
        runtime.install_kernel_browser_fixture(first.owner_user_id(), &root);
    }
    focus(&router, session.id(), first.id()).await;
    let token = first_run.runtime_mcp_auth_token().unwrap();
    tool(&router, token, "chariox.load_kernel_browser", json!({}))
        .await
        .unwrap();
    let mut state = if native {
        tool(
            &router,
            token,
            "chariox.kernel_browser",
            json!({"command":{"op":"open","url":fixture.as_ref().unwrap().0}}),
        )
        .await
        .unwrap()
        .payload
    } else {
        human(&router, KernelBrowserCommand::State).await
    };
    let tab = state["tabs"][0]["tab_id"].as_str().unwrap().to_string();
    let generation = state["generation"].as_u64().unwrap();
    let snapshot_args = json!({"command":{"op":"snapshot","tab_id":tab,"generation":generation}});
    tool(
        &router,
        token,
        "chariox.kernel_browser",
        snapshot_args.clone(),
    )
    .await
    .unwrap();
    let stream = tool(
        &router,
        token,
        "chariox.kernel_browser",
        json!({"command":{"op":"subscribe","tab_id":tab,"generation":generation}}),
    )
    .await
    .unwrap();
    let poll = json!({"command":{"op":"poll","subscription_id":stream.payload["subscription_id"],"generation":generation}});
    if native {
        state = human(&router, KernelBrowserCommand::State).await;
    }
    // Hold the kernel's existing parent-turn state while another admitted agent
    // runs. This verifies the harness-wait authority seam, not model behavior.
    runtime.start_active_turn_with_trace_id(
        session.id(),
        first.id(),
        "held-parent",
        first_run.id(),
        "waiting-child",
    );
    runtime.start_active_turn_with_trace_id(
        session.id(),
        second.id(),
        "child-work",
        second_run.id(),
        "child-work",
    );
    focus(&router, session.id(), second.id()).await;
    let grants = human(&router, KernelBrowserCommand::ListGrants).await;
    assert_eq!(grants["grants"].as_array().unwrap().len(), 2);
    let holder = grants["grants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|holder| holder["agent_id"] == first.id())
        .unwrap();
    assert_eq!(holder["idle_since_ms"], Value::Null);
    assert_eq!(holder["session_id"], session.id());
    assert_eq!(holder["resources"][0]["tab_id"], tab);
    tool(
        &router,
        token,
        "chariox.kernel_browser",
        snapshot_args.clone(),
    )
    .await
    .unwrap();
    tool(&router, token, "chariox.kernel_browser", poll.clone())
        .await
        .unwrap();
    tool(&router, token, "chariox.kernel_browser", json!({"document_id":state["tabs"][0]["document_id"],"command":{"op":"input","tab_id":tab,"generation":generation,"input":{"kind":"scroll","x":500,"y":500,"delta_x":0,"delta_y":100}}})).await.unwrap();
    if !native {
        std::fs::remove_file(root.join("input")).unwrap();
    }
    let sensitive_click = json!({"document_id":state["tabs"][0]["document_id"],"command":{"op":"input","tab_id":tab,"generation":generation,"input":{"kind":"click","x":30,"y":30}}});
    tool(
        &router,
        token,
        "chariox.kernel_browser",
        sensitive_click.clone(),
    )
    .await
    .unwrap();
    assert!(matches!(
        tool(
            &router,
            token,
            "chariox.kernel_browser_paste_secret",
            json!({})
        )
        .await
        .unwrap_err(),
        crate::error::DaemonError::UserDomainRefused {
            reason: crate::error::UserDomainRefusalReason::SensitiveRequiresFocus
        }
    ));
    if !native {
        assert!(
            root.join("input").exists(),
            "MP-11: retained input did not dispatch"
        );
    }
    let new_tab = tool(
        &router,
        token,
        "chariox.kernel_browser",
        json!({"command":{"op":"open","url":"about:blank"}}),
    )
    .await
    .unwrap();
    assert!(
        new_tab.payload["tab_id"].is_string(),
        "MP-11: explicit retained open must grant its new tab"
    );
    assert!(tool(
        &router,
        token,
        "chariox.kernel_browser",
        json!({"command":{"op":"snapshot","tab_id":"unclaimed","generation":generation}})
    )
    .await
    .is_err());
    let changes = human(
        &router,
        KernelBrowserCommand::SubscribeGrants {
            after: grants["cursor"].as_u64().unwrap(),
            wait_ms: 0,
        },
    )
    .await;
    assert!(changes["cursor"].as_u64().unwrap() > grants["cursor"].as_u64().unwrap());
    assert_eq!(changes["notice"]["agent_id"], first.id());
    // A scheduled wake retains access after the parent turn has settled.
    let request = LocalDaemonRequest::CreateAgentPromptSchedule(
        crate::local::CreateAgentPromptScheduleRequest {
            session_id: session.id().into(),
            agent_id: first.id().into(),
            kind: crate::session::AgentPromptScheduleKind::Once,
            interval_seconds: 600,
            prompt: None,
        },
    );
    terminal(
        &router,
        terminal_command("mdaccess-wake", &request),
        request,
    )
    .await
    .unwrap();
    runtime.clear_prompt_activity_for_managed_activity_test(first_run.id());
    let waiting = human(&router, KernelBrowserCommand::ListGrants).await;
    assert_eq!(
        waiting["grants"]
            .as_array()
            .unwrap()
            .iter()
            .find(|grant| grant["agent_id"] == first.id())
            .unwrap()["idle_since_ms"],
        Value::Null
    );
    tool(
        &router,
        token,
        "chariox.kernel_browser",
        snapshot_args.clone(),
    )
    .await
    .unwrap();
    let revoked = human(
        &router,
        KernelBrowserCommand::RevokeGrants {
            agent_id: Some(first.id().into()),
        },
    )
    .await;
    assert_eq!(revoked["grants"].as_array().unwrap().len(), 1);
    assert!(
        tool(&router, token, "chariox.kernel_browser", snapshot_args)
            .await
            .is_err()
    );
    assert!(tool(&router, token, "chariox.kernel_browser", poll)
        .await
        .is_err());
    if !native {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while !root.join("subscriptions-revoked").exists() && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            root.join("subscriptions-revoked").exists(),
            "MP-11: idle subscriber cleanup must execute"
        );
    }
    let all = human(
        &router,
        KernelBrowserCommand::RevokeGrants { agent_id: None },
    )
    .await;
    assert!(all["grants"].as_array().unwrap().is_empty());
    focus(&router, session.id(), second.id()).await;
    let destroy = LocalDaemonRequest::DestroyAgent(crate::local::DestroyAgentRequest {
        session_id: session.id().into(),
        agent_id: second.id().into(),
    });
    terminal(
        &router,
        terminal_command("mdaccess-destroy", &destroy),
        destroy,
    )
    .await
    .unwrap();
    assert!(
        human(&router, KernelBrowserCommand::ListGrants).await["grants"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    focus(&router, session.id(), first.id()).await;
    let request = LocalDaemonRequest::EndSession(crate::local::EndSessionRequest {
        session_id: session.id().into(),
    });
    terminal(&router, terminal_command("mdaccess-end", &request), request)
        .await
        .unwrap();
    assert!(
        human(&router, KernelBrowserCommand::ListGrants).await["grants"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    runtime.shutdown_cleanup().await.unwrap();
    if let Some((_, task)) = fixture {
        task.abort();
    }
    if native {
        std::fs::write(root.join("MP-08-native-access.json"), serde_json::to_vec_pretty(&json!({"result":"PASS","providers":"dev-stub admitted runs","browser":"native sandboxed Chromium","proof":"two-agent held turn, pending scheduled wake, scoped resource, visibility, notice, immediate revoke","limits":"no official-provider subagent, client, hosted transport or fresh-machine acceptance"})).unwrap()).unwrap();
    } else {
        std::fs::remove_dir_all(root).unwrap();
    }
}

// MP-10/MP-11: synthetic critical-action page; no provider or account data.
async fn payment_fixture() -> (String, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await;
            let body = "<!doctype html><html><body><button style='position:absolute;left:10px;top:10px;width:180px;height:40px'>Approve payment</button><p>MP-08 / MP-10 / MP-11 synthetic page</p></body></html>";
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    (url, task)
}

// MP-10: keep the matrix harness's futures boxed at the call boundary so the
// real router still runs on ordinary test thread stacks.
fn tool<'a>(
    router: &'a CommandRouter,
    token: &'a str,
    name: &'a str,
    args: Value,
) -> futures_util::future::BoxFuture<
    'a,
    Result<crate::transport::runtime_tools::RuntimeToolResult, crate::DaemonError>,
> {
    Box::pin(router.dispatch_authenticated_runtime_tool_call(token, name, args))
}
fn terminal<'a>(
    router: &'a CommandRouter,
    command: KernelCommand,
    request: LocalDaemonRequest,
) -> futures_util::future::BoxFuture<'a, Result<LocalDaemonResponse, crate::DaemonError>> {
    Box::pin(router.dispatch(command, request))
}
