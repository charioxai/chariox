//! MP-08/MP-10/MP-11 A05: user-requested kernel-browser grants through the
//! real router and provider MCP admission. The browser is the fixture controller;
//! Live Chromium/provider acceptance is a separate recorded gate.
use super::*;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};

struct Room {
    router: CommandRouter,
    session: String,
    root: std::path::PathBuf,
    _workspace: crate::test_support::TestWorktree,
}

struct Setup {
    app: DaemonApp,
    session: String,
    agents: Vec<(crate::agent::AgentInstance, String)>,
    workspace: crate::test_support::TestWorktree,
}

/// Owner-created first agent, `spawned` children recording it as their
/// immutable creator, then unrelated `peers`. Every agent has a dev-stub run.
fn setup(name: &str, spawned: &[&str], peers: &[&str]) -> Setup {
    let workspace = crate::test_support::TestWorktree::new(name);
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, first) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    let mut agents = vec![first.clone()];
    for child in spawned {
        agents.push(
            crate::app::KernelSessionService::new(&mut app)
                .spawn_agent(
                    CreateAgentRequest::new(session.id(), "dev-stub")
                        .with_alias(*child)
                        .with_spawned_by_agent_id(first.id()),
                )
                .unwrap(),
        );
    }
    for peer in peers {
        agents.push(spawn_test_agent(&mut app, session.id(), peer, "dev-stub"));
    }
    let agents = agents
        .into_iter()
        .map(|agent| {
            let run = launch_test_provider(
                &mut app,
                session.id(),
                agent.id(),
                "dev-stub",
                "dev-stub",
                "native-tui-idle",
            );
            let token = run.runtime_mcp_auth_token().unwrap().to_string();
            (agent, token)
        })
        .collect();
    Setup {
        app,
        session: session.id().into(),
        agents,
        workspace,
    }
}

fn start(setup: Setup, owner: &str) -> Room {
    let root = std::env::temp_dir().join(format!(
        "chariox-capability-{:032x}",
        rand::random::<u128>()
    ));
    std::fs::create_dir(&root).unwrap();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(setup.app)), 4);
    router
        .runtime_state()
        .install_kernel_browser_fixture(owner, &root);
    Room {
        router,
        session: setup.session,
        root,
        _workspace: setup.workspace,
    }
}

/// Submits a prompt from an attachment of `level` and marks it running.
async fn running_prompt(
    room: &Room,
    agent: &str,
    level: ClientCapabilityLevel,
    client: &str,
) -> String {
    let mut app = room.router.app.lock().await;
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::new(&room.session, client, level))
        .unwrap();
    app.submit_prompt(
        &room.session,
        attachment.id(),
        Some(agent),
        "Open the docs page in my browser",
        Vec::new(),
    )
    .unwrap();
    let prompts = app.prompt_state_owner();
    let session = app.sessions().get_session(&room.session).unwrap();
    let active = prompts
        .active_prompt_for_agent_snapshot(&session, agent)
        .unwrap();
    let mut running = active.clone();
    running.set_status(crate::session::PromptStatus::Running);
    assert!(prompts.replace_active_prompt_if_matches(&session, agent, &active, running));
    active.id().to_string()
}

fn refusal(error: crate::DaemonError) -> Option<crate::error::UserDomainRefusalReason> {
    match error {
        crate::DaemonError::UserDomainRefused { reason } => Some(reason),
        _ => None,
    }
}

async fn grants(router: &CommandRouter) -> Vec<Value> {
    human(router, KernelBrowserCommand::ListGrants).await["grants"]
        .as_array()
        .unwrap()
        .clone()
}

fn snapshot(tab: &str) -> Value {
    json!({"command":{"op":"snapshot","tab_id":tab,"generation":1}})
}

#[test]
fn capability_owner_prompt_alone_cannot_mint_a_browser_grant() {
    run_test(|| {
        Box::pin(async {
            let setup = setup("capability-browser-explicit-request", &[], &[]);
            let (agent, token) = setup.agents[0].clone();
            let room = start(setup, agent.owner_user_id());
            running_prompt(
                &room,
                agent.id(),
                ClientCapabilityLevel::FullTerminal,
                "owner",
            )
            .await;
            let attempt = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                tool(
                    &room.router,
                    &token,
                    "chariox.load_kernel_browser",
                    json!({}),
                ),
            )
            .await;
            assert!(
                attempt.is_err(),
                "MP-11: a prompt alone is not a browser acquisition request"
            );
            assert!(grants(&room.router).await.is_empty());
            room.router
                .runtime_state()
                .shutdown_cleanup()
                .await
                .unwrap();
            std::fs::remove_dir_all(&room.root).unwrap();
        })
    });
}

#[test]
fn capability_owner_request_grants_only_opened_tabs_until_revoked() {
    run_test(|| {
        Box::pin(async {
            let setup = setup("capability-s01", &[], &["worker"]);
            let (agent, token) = setup.agents[1].clone();
            let room = start(setup, agent.owner_user_id());
            let router = &room.router;
            let prompt = running_prompt(
                &room,
                agent.id(),
                ClientCapabilityLevel::FullTerminal,
                "owner-terminal",
            )
            .await;
            assert!(router
                .runtime_tool_specs_for_auth_token(&token)
                .iter()
                .any(|spec| spec.name == "chariox.load_kernel_browser"));
            let loaded = approved_tool(&room, &token, "chariox.load_kernel_browser", json!({}))
                .await
                .expect("MP-08: the owner's request grants browser access without focus");
            assert_eq!(loaded.payload["prompt_id"], prompt);
            let holder = grants(router).await;
            let holder = holder
                .iter()
                .find(|grant| grant["agent_id"] == agent.id())
                .unwrap();
            assert_eq!(holder["prompt_id"], prompt);
            assert_eq!(holder["focused"], false);
            let expires = holder["expires_at_ms"].as_u64().unwrap();
            let lifetime = expires - holder["since_ms"].as_u64().unwrap();
            assert!((8 * 3600 * 1000 - 5_000..=8 * 3600 * 1000).contains(&lifetime));
            let opened = tool(
                router,
                &token,
                "chariox.kernel_browser",
                json!({"command":{"op":"open","url":"https://developer.mozilla.org/"}}),
            )
            .await
            .unwrap();
            assert_eq!(opened.payload["tab_id"], "host-tab-new");
            tool(
                router,
                &token,
                "chariox.kernel_browser",
                snapshot("host-tab-new"),
            )
            .await
            .expect("the opened tab joins the requested grant");
            assert_eq!(
                refusal(
                    tool(
                        router,
                        &token,
                        "chariox.kernel_browser",
                        snapshot("host-tab-fixture")
                    )
                    .await
                    .unwrap_err()
                ),
                Some(crate::error::UserDomainRefusalReason::NotFocusedAgent),
                "MP-11: an existing unrelated tab is not covered by the request"
            );
            assert_eq!(
                refusal(
                    tool(
                        router,
                        &token,
                        "chariox.kernel_browser_paste_secret",
                        json!({})
                    )
                    .await
                    .unwrap_err()
                ),
                Some(crate::error::UserDomainRefusalReason::SensitiveRequiresFocus)
            );
            human(
                router,
                KernelBrowserCommand::RevokeGrants {
                    agent_id: Some(agent.id().into()),
                },
            )
            .await;
            assert!(tool(
                router,
                &token,
                "chariox.kernel_browser",
                snapshot("host-tab-new")
            )
            .await
            .is_err());
            router.runtime_state().shutdown_cleanup().await.unwrap();
            std::fs::remove_dir_all(&room.root).unwrap();
        })
    });
}

/// MP-08 (#922 review 3): the owner's Deny is the typed `not_requested` refusal.
#[test]
fn capability_owner_deny_is_a_typed_not_requested_refusal() {
    run_test(|| {
        Box::pin(async {
            let setup = setup("capability-deny", &[], &[]);
            let (agent, token) = setup.agents[0].clone();
            let room = start(setup, agent.owner_user_id());
            running_prompt(
                &room,
                agent.id(),
                ClientCapabilityLevel::FullTerminal,
                "owner",
            )
            .await;
            let error = decided_tool(
                &room,
                &token,
                "chariox.load_kernel_browser",
                json!({}),
                "deny",
                None,
            )
            .await
            .unwrap_err();
            assert_eq!(
                refusal(error),
                Some(crate::error::UserDomainRefusalReason::NotRequested)
            );
            assert!(grants(&room.router).await.is_empty());
            room.router
                .runtime_state()
                .shutdown_cleanup()
                .await
                .unwrap();
            std::fs::remove_dir_all(&room.root).unwrap();
        })
    });
}

/// MP-11 (#922 review 1): another holder browsing while the owner decides
/// neither withdraws nor invalidates this agent's pending approval.
#[test]
fn capability_other_holder_activity_keeps_a_pending_browser_approval() {
    run_test(|| {
        Box::pin(async {
            let setup = setup("capability-busy-room", &[], &["browsing", "asking"]);
            let (browsing, browsing_token) = setup.agents[1].clone();
            let (asking, asking_token) = setup.agents[2].clone();
            let room = start(setup, browsing.owner_user_id());
            for agent in [&browsing, &asking] {
                running_prompt(
                    &room,
                    agent.id(),
                    ClientCapabilityLevel::FullTerminal,
                    &format!("owner-{}", agent.id()),
                )
                .await;
            }
            approved_tool(
                &room,
                &browsing_token,
                "chariox.load_kernel_browser",
                json!({}),
            )
            .await
            .unwrap();
            decided_tool(
                &room,
                &asking_token,
                "chariox.load_kernel_browser",
                json!({}),
                "allow",
                Some((
                    &browsing_token,
                    "chariox.kernel_browser",
                    json!({"command":{"op":"open","url":"https://developer.mozilla.org/"}}),
                )),
            )
            .await
            .expect("MP-11: an unrelated holder's grant change keeps this approval");
            assert_eq!(grants(&room.router).await.len(), 2);
            room.router
                .runtime_state()
                .shutdown_cleanup()
                .await
                .unwrap();
            std::fs::remove_dir_all(&room.root).unwrap();
        })
    });
}

#[test]
fn capability_unrequested_turns_cannot_acquire_browser_access() {
    run_test(|| {
        Box::pin(async {
            let setup = setup("capability-s02", &[], &["idle", "messaged"]);
            let agents = setup.agents.clone();
            let room = start(setup, agents[0].0.owner_user_id());
            let router = &room.router;
            // No running prompt at all.
            let (_, idle) = &agents[1];
            let error = tool(router, idle, "chariox.load_kernel_browser", json!({}))
                .await
                .unwrap_err();
            assert!(refusal(error).is_some());
            // A turn caused by automation (agent message, workflow, schedule, App).
            let (messaged, token) = &agents[2];
            running_prompt(
                &room,
                messaged.id(),
                ClientCapabilityLevel::AutomationOnly,
                &format!("agent:{}:messages", agents[0].0.id()),
            )
            .await;
            let error = tool(router, token, "chariox.load_kernel_browser", json!({}))
                .await
                .unwrap_err();
            assert!(refusal(error).is_some(), "MP-08: unrequested acquisition");
            assert!(tool(
                router,
                token,
                "chariox.kernel_browser",
                json!({"command":{"op":"open","url":"https://www.wikipedia.org/"}})
            )
            .await
            .is_err());
            // Agents cannot change user focus to mint access for themselves.
            let request = LocalDaemonRequest::FocusAgent(FocusAgentRequest {
                session_id: room.session.clone(),
                agent_id: messaged.id().into(),
            });
            let mut command =
                KernelCommand::from_local_request("agent-focus", None, None, &request);
            command.caller.metaagent_id = Some(messaged.id().into());
            assert!(router.dispatch(command, request).await.is_err());
            assert!(
                grants(router).await.is_empty(),
                "MP-11: grant set unchanged"
            );
            router.runtime_state().shutdown_cleanup().await.unwrap();
            std::fs::remove_dir_all(&room.root).unwrap();
        })
    });
}

#[test]
fn capability_child_subset_transfer_never_widens_and_follows_parent_revoke() {
    run_test(|| {
        Box::pin(async {
            let setup = setup("capability-s03", &["child"], &["peer"]);
            let (parent, parent_token) = setup.agents[0].clone();
            let (child, child_token) = setup.agents[1].clone();
            let (peer, _) = setup.agents[2].clone();
            let room = start(setup, parent.owner_user_id());
            let router = &room.router;
            running_prompt(
                &room,
                parent.id(),
                ClientCapabilityLevel::FullTerminal,
                "owner-terminal",
            )
            .await;
            approved_tool(
                &room,
                &parent_token,
                "chariox.load_kernel_browser",
                json!({}),
            )
            .await
            .unwrap();
            tool(
                router,
                &parent_token,
                "chariox.kernel_browser",
                json!({"command":{"op":"open","url":"https://github.com/"}}),
            )
            .await
            .unwrap();
            let tab = json!([{"kind":"browser_tab","tab_id":"host-tab-new"}]);
            for (target, resources) in [
                (peer.id(), tab.clone()),
                (
                    child.id(),
                    json!([{"kind":"browser_tab","tab_id":"host-tab-fixture"}]),
                ),
            ] {
                assert!(
                    tool(
                        router,
                        &parent_token,
                        "chariox.kernel_browser_share",
                        json!({"agent": target, "resources": resources}),
                    )
                    .await
                    .is_err(),
                    "MP-08: transfer is limited to a direct child and held resources"
                );
            }
            tool(
                router,
                &parent_token,
                "chariox.kernel_browser_share",
                json!({"agent": child.id(), "resources": tab}),
            )
            .await
            .expect("explicit subset transfer to a direct child");
            let listed = grants(router).await;
            let delegated = listed
                .iter()
                .find(|grant| grant["agent_id"] == child.id())
                .unwrap();
            assert_eq!(delegated["delegated_by_agent_id"], parent.id());
            let parent_grant = listed
                .iter()
                .find(|grant| grant["agent_id"] == parent.id())
                .unwrap();
            assert_eq!(delegated["expires_at_ms"], parent_grant["expires_at_ms"]);
            tool(
                router,
                &child_token,
                "chariox.kernel_browser",
                snapshot("host-tab-new"),
            )
            .await
            .expect("child uses the transferred tab");
            for args in [
                snapshot("host-tab-fixture"),
                json!({"command":{"op":"open","url":"https://www.google.com/search?q=chariox"}}),
            ] {
                assert!(
                    tool(router, &child_token, "chariox.kernel_browser", args)
                        .await
                        .is_err(),
                    "MP-11: no implicit widening"
                );
            }
            human(
                router,
                KernelBrowserCommand::RevokeGrants {
                    agent_id: Some(parent.id().into()),
                },
            )
            .await;
            assert!(
                grants(router).await.is_empty(),
                "child grant revoked with parent"
            );
            assert!(tool(
                router,
                &child_token,
                "chariox.kernel_browser",
                snapshot("host-tab-new")
            )
            .await
            .is_err());
            router.runtime_state().shutdown_cleanup().await.unwrap();
            std::fs::remove_dir_all(&room.root).unwrap();
        })
    });
}

#[test]
fn capability_leased_agents_cannot_acquire_or_receive_user_domain_access() {
    run_test(|| {
        Box::pin(async {
            let setup = setup("capability-s04", &["leased"], &[]);
            let (parent, parent_token) = setup.agents[0].clone();
            let (child, child_token) = setup.agents[1].clone();
            setup
                .app
                .agents()
                .bind_remote_execution(
                    child.id(),
                    crate::agent::RemoteAgentBinding {
                        worker_kernel_id: "execution-kernel".into(),
                        worker_machine_id: "worker-machine".into(),
                        execution_lease_id: "lease".into(),
                        leased_agent_id: "worker-agent".into(),
                        active_worker_provider_run_id: None,
                        relay_url: None,
                        relay_token: None,
                        relay_peer_protocol_version: Some(
                            crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                        ),
                    },
                )
                .unwrap();
            let room = start(setup, parent.owner_user_id());
            let router = &room.router;
            running_prompt(
                &room,
                child.id(),
                ClientCapabilityLevel::FullTerminal,
                "owner-terminal-leased",
            )
            .await;
            let error = tool(
                router,
                &child_token,
                "chariox.load_kernel_browser",
                json!({}),
            )
            .await
            .unwrap_err();
            assert_eq!(
                refusal(error),
                Some(crate::error::UserDomainRefusalReason::NotGranted),
                "MP-08: leased acquisition is a stable typed refusal"
            );
            running_prompt(
                &room,
                parent.id(),
                ClientCapabilityLevel::FullTerminal,
                "owner-terminal",
            )
            .await;
            approved_tool(
                &room,
                &parent_token,
                "chariox.load_kernel_browser",
                json!({}),
            )
            .await
            .unwrap();
            tool(
                router,
                &parent_token,
                "chariox.kernel_browser",
                json!({"command":{"op":"open","url":"https://en.wikipedia.org/wiki/Web_browser"}}),
            )
            .await
            .unwrap();
            assert!(
                tool(
                    router,
                    &parent_token,
                    "chariox.kernel_browser_share",
                    json!({"agent": child.id(), "resources": [{"kind":"browser_tab","tab_id":"host-tab-new"}]}),
                )
                .await
                .is_err(),
                "MP-08: leased agents keep only their Room Browser/Computer route"
            );
            assert_eq!(grants(router).await.len(), 1);
            router.runtime_state().shutdown_cleanup().await.unwrap();
            std::fs::remove_dir_all(&room.root).unwrap();
        })
    });
}

// MP-08/MP-11 SB-01: a session grant admits attachment, not human focus.
async fn external_attach(room: &Room) -> crate::attachment::RuntimeAttachment {
    let grant = room
        .router
        .runtime_state()
        .insert_access_grant_for_test(&room.session);
    let request = LocalDaemonRequest::AttachToSession(crate::local::AttachToSessionRequest {
        session_id: room.session.clone(),
        client_id: "external-requesting-terminal".into(),
        capability_level: ClientCapabilityLevel::FullTerminal,
    });
    let mut command = KernelCommand::from_local_request("MP-11-SB-01", None, None, &request);
    command.caller.connection_class = Some(crate::local::KernelConnectionClass::ExternalAgent);
    command.caller.caller_id = grant;
    let LocalDaemonResponse::SessionAttached { attachment } =
        room.router.dispatch(command, request).await.unwrap()
    else {
        panic!("MP-11: session attachment expected");
    };
    attachment
}

#[test]
fn security_sb01_external_attach_cannot_restore_revoked_focus() {
    run_test(|| {
        Box::pin(async {
            let setup = setup("security-sb01-revoked", &[], &[]);
            let (agent, token) = setup.agents[0].clone();
            let room = start(setup, agent.owner_user_id());
            focus(&room.router, &room.session, agent.id()).await;
            human(
                &room.router,
                KernelBrowserCommand::RevokeGrants {
                    agent_id: Some(agent.id().into()),
                },
            )
            .await;
            external_attach(&room).await;
            assert!(
                grants(&room.router).await.is_empty(),
                "MP-11 SB-01: automation cannot restore focus authority"
            );
            assert!(tool(
                &room.router,
                &token,
                "chariox.load_kernel_browser",
                json!({})
            )
            .await
            .is_err());
            assert!(tool(
                &room.router,
                &token,
                "chariox.kernel_browser",
                snapshot("host-tab-fixture")
            )
            .await
            .is_err());
            room.router
                .runtime_state()
                .shutdown_cleanup()
                .await
                .unwrap();
            std::fs::remove_dir_all(&room.root).unwrap();
        })
    });
}

#[test]
fn security_sb01_external_attach_is_not_owner_prompt_causation() {
    run_test(|| {
        Box::pin(async {
            let setup = setup("security-sb01-causation", &[], &[]);
            let (agent, _) = setup.agents[0].clone();
            let room = start(setup, agent.owner_user_id());
            let attachment = external_attach(&room).await;
            {
                let mut app = room.router.app.lock().await;
                app.submit_prompt(
                    &room.session,
                    attachment.id(),
                    Some(agent.id()),
                    "Use my browser",
                    Vec::new(),
                )
                .unwrap();
                let prompts = app.prompt_state_owner();
                let session = app.sessions().get_session(&room.session).unwrap();
                let active = prompts
                    .active_prompt_for_agent_snapshot(&session, agent.id())
                    .unwrap();
                let mut running = active.clone();
                running.set_status(crate::session::PromptStatus::Running);
                assert!(prompts.replace_active_prompt_if_matches(
                    &session,
                    agent.id(),
                    &active,
                    running
                ));
            }
            assert!(
                room.router
                    .runtime_state()
                    .owner_requested_prompt(&agent)
                    .is_none(),
                "MP-11 SB-01: caller-selected terminal capability is not human causation"
            );
            room.router
                .runtime_state()
                .shutdown_cleanup()
                .await
                .unwrap();
            std::fs::remove_dir_all(&room.root).unwrap();
        })
    });
}

#[test]
fn security_sb01_external_attach_cannot_promote_delegated_focus() {
    run_test(|| {
        Box::pin(async {
            let setup = setup("security-sb01-delegated", &["child"], &[]);
            let (parent, parent_token) = setup.agents[0].clone();
            let (child, child_token) = setup.agents[1].clone();
            let room = start(setup, parent.owner_user_id());
            running_prompt(
                &room,
                parent.id(),
                ClientCapabilityLevel::FullTerminal,
                "owner",
            )
            .await;
            approved_tool(
                &room,
                &parent_token,
                "chariox.load_kernel_browser",
                json!({}),
            )
            .await
            .unwrap();
            tool(
                &room.router,
                &parent_token,
                "chariox.kernel_browser",
                json!({"command":{"op":"open","url":"https://www.wikipedia.org/"}}),
            )
            .await
            .unwrap();
            tool(&room.router, &parent_token, "chariox.kernel_browser_share", json!({"agent":child.id(), "resources":[{"kind":"browser_tab","tab_id":"host-tab-new"}]})).await.unwrap();
            room.router
                .app
                .lock()
                .await
                .focus_agent(&room.session, child.id())
                .unwrap();
            let before = grants(&room.router).await;
            external_attach(&room).await;
            assert_eq!(
                grants(&room.router).await,
                before,
                "MP-11 SB-01: automation preserves delegation cause and expiry"
            );
            assert!(tool(
                &room.router,
                &child_token,
                "chariox.kernel_browser",
                snapshot("host-tab-fixture")
            )
            .await
            .is_err());
            human(
                &room.router,
                KernelBrowserCommand::RevokeGrants {
                    agent_id: Some(parent.id().into()),
                },
            )
            .await;
            assert!(
                grants(&room.router).await.is_empty(),
                "MP-11 SB-01: parent revoke still reaches child"
            );
            room.router
                .runtime_state()
                .shutdown_cleanup()
                .await
                .unwrap();
            std::fs::remove_dir_all(&room.root).unwrap();
        })
    });
}

// MP-11 SB-01: prompt routing must retain the authenticated caller role too.
#[test]
fn security_sb01_external_alias_prompt_cannot_mint_focus() {
    run_test(|| {
        Box::pin(async {
            let setup = setup("security-sb01-alias", &[], &["victim"]);
            let (agent, _) = setup.agents[1].clone();
            let room = start(setup, agent.owner_user_id());
            let attachment = external_attach(&room).await;
            human(
                &room.router,
                KernelBrowserCommand::RevokeGrants { agent_id: None },
            )
            .await;
            let grant = room
                .router
                .runtime_state()
                .insert_access_grant_for_test(&room.session);
            let request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: room.session.clone(),
                attachment_id: attachment.id().into(),
                target_agent_id: None,
                prompt: format!("@{} inspect the browser", agent.alias().unwrap()),
                attachments: Vec::new(),
            });
            let mut command =
                KernelCommand::from_local_request("MP-11-SB-01-alias", None, None, &request);
            command.caller.connection_class =
                Some(crate::local::KernelConnectionClass::ExternalAgent);
            command.caller.caller_id = grant;
            room.router.dispatch(command, request).await.unwrap();
            assert!(
                grants(&room.router).await.is_empty(),
                "MP-11 SB-01: automated alias routing cannot grant focus"
            );
            room.router
                .runtime_state()
                .shutdown_cleanup()
                .await
                .unwrap();
            std::fs::remove_dir_all(&room.root).unwrap();
        })
    });
}

#[test]
fn security_sb01_external_prompt_cannot_borrow_human_attachment() {
    run_test(|| {
        Box::pin(async {
            let setup = setup("security-sb01-borrow", &[], &[]);
            let (agent, _) = setup.agents[0].clone();
            let room = start(setup, agent.owner_user_id());
            let attachment =
                crate::app::KernelSessionService::new(&mut *room.router.app.lock().await)
                    .attach(AttachRequest::new(
                        &room.session,
                        "real-terminal",
                        ClientCapabilityLevel::FullTerminal,
                    ))
                    .unwrap();
            let grant = room
                .router
                .runtime_state()
                .insert_access_grant_for_test(&room.session);
            let mut request = LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: room.session.clone(),
                attachment_id: attachment.id().into(),
                target_agent_id: Some(agent.id().into()),
                prompt: "Use my App".into(),
                attachments: Vec::new(),
            });
            let mut command =
                KernelCommand::from_local_request("MP-11-SB-01-borrow", None, None, &request);
            command.caller.connection_class =
                Some(crate::local::KernelConnectionClass::ExternalAgent);
            command.caller.caller_id = grant;
            assert!(
                room.router
                    .authorize_external_request(&command, &mut request)
                    .is_err(),
                "MP-11 SB-01: automation cannot submit as a human attachment"
            );
            room.router
                .runtime_state()
                .shutdown_cleanup()
                .await
                .unwrap();
            std::fs::remove_dir_all(&room.root).unwrap();
        })
    });
}

#[test]
fn security_sb02_prompt_grant_cannot_stop_unrelated_browser() {
    run_test(|| {
        Box::pin(async {
            let setup = setup("security-sb02-stop", &[], &["worker"]);
            let (agent, token) = setup.agents[1].clone();
            let room = start(setup, agent.owner_user_id());
            running_prompt(
                &room,
                agent.id(),
                ClientCapabilityLevel::FullTerminal,
                "owner",
            )
            .await;
            approved_tool(&room, &token, "chariox.load_kernel_browser", json!({}))
                .await
                .unwrap();
            let before = human(&room.router, KernelBrowserCommand::State).await;
            let error = tool(
                &room.router,
                &token,
                "chariox.kernel_browser",
                json!({"command":{"op":"stop"}}),
            )
            .await
            .expect_err("MP-11 R3: new-tabs approval cannot stop the whole browser");
            assert_eq!(
                refusal(error),
                Some(crate::error::UserDomainRefusalReason::NotFocusedAgent),
                "MP-11 R3: Stop keeps the typed not-focused refusal"
            );
            assert_eq!(
                crate::error::UserDomainRefusalReason::NotFocusedAgent.code(),
                "user_domain_not_focused_agent"
            );
            assert_eq!(
                human(&room.router, KernelBrowserCommand::State).await,
                before,
                "MP-11 SB-02: unrelated tab/browser remains live"
            );
            room.router
                .runtime_state()
                .shutdown_cleanup()
                .await
                .unwrap();
            std::fs::remove_dir_all(&room.root).unwrap();
        })
    });
}

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

async fn approved_tool(
    room: &Room,
    token: &str,
    name: &str,
    args: Value,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, crate::DaemonError> {
    decided_tool(room, token, name, args, "allow", None).await
}

/// Answers the owner's resource decision with `choice`, after `meanwhile`
/// (another agent's tool call) ran while the decision was pending.
async fn decided_tool(
    room: &Room,
    token: &str,
    name: &str,
    args: Value,
    choice: &str,
    meanwhile: Option<(&str, &str, Value)>,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, crate::DaemonError> {
    let (result, ()) = tokio::join!(tool(&room.router, token, name, args), async {
        let interaction = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let session = room
                    .router
                    .runtime_state()
                    .session_snapshot(&room.session)
                    .await
                    .unwrap();
                if let Some(interaction) = session
                    .active_interactions()
                    .iter()
                    .find(|interaction| interaction.title() == Some("Chariox resource access"))
                {
                    break interaction.id().to_string();
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("resource-specific owner interaction");
        if let Some((token, name, args)) = meanwhile {
            tool(&room.router, token, name, args).await.unwrap();
        }
        let owner = room
            .router
            .runtime_state()
            .session_snapshot(&room.session)
            .await
            .unwrap()
            .owner_user_id()
            .to_string();
        room.router
            .runtime_state()
            .resolve_terminal_runtime_interaction(
                &room.session,
                &interaction,
                choice,
                None,
                Some(&owner),
            )
            .await
            .unwrap();
    });
    result
}

// MP-08/MP-10/MP-11 R1: an admitted external edit must not retain the
// human queue item's acquisition provenance through promotion.
#[test]
fn capability_review_r1_external_queue_edit_cannot_acquire_browser() {
    run_test(|| {
        Box::pin(async {
            let setup = setup("capability-r1-edit", &[], &[]);
            let (agent, token) = setup.agents[0].clone();
            let room = start(setup, agent.owner_user_id());
            running_prompt(
                &room,
                agent.id(),
                ClientCapabilityLevel::FullTerminal,
                "human",
            )
            .await;
            let (human, automation, queued) = {
                let mut app = room.router.app.lock().await;
                let human = app
                    .attachments()
                    .list_client_attachments("human")
                    .into_iter()
                    .next()
                    .unwrap();
                let automation = crate::app::KernelSessionService::new(&mut app)
                    .attach(AttachRequest::new(
                        &room.session,
                        "external",
                        ClientCapabilityLevel::AutomationOnly,
                    ))
                    .unwrap();
                app.submit_prompt(
                    &room.session,
                    human.id(),
                    Some(agent.id()),
                    "Human queued request",
                    Vec::new(),
                )
                .unwrap();
                let session = app.sessions().get_session(&room.session).unwrap();
                let (_, queued) = app.prompt_state_owner().state_parts(&session, agent.id());
                (human, automation, queued.back().unwrap().clone())
            };
            assert!(queued.owner_request());
            let grant = room
                .router
                .runtime_state()
                .insert_access_grant_for_test(&room.session);
            // MP-11 R1: external callers cannot borrow the human attachment for edits either.
            let make_request = |attachment: &str| {
                LocalDaemonRequest::UpdateQueuedPrompt(crate::local::UpdateQueuedPromptRequest {
                    session_id: room.session.clone(),
                    attachment_id: attachment.into(),
                    target_agent_id: agent.id().into(),
                    prompt_id: queued.id().into(),
                    prompt: "Automation requests my browser".into(),
                })
            };
            let mut request = make_request(human.id());
            let mut command =
                KernelCommand::from_local_request("MP-11-R1-borrow", None, None, &request);
            command.caller.connection_class =
                Some(crate::local::KernelConnectionClass::ExternalAgent);
            command.caller.caller_id = grant.clone();
            let borrowed = room
                .router
                .authorize_external_request(&command, &mut request);

            let request = make_request(automation.id());
            let mut command =
                KernelCommand::from_local_request("MP-11-R1-edit", None, None, &request);
            command.caller.connection_class =
                Some(crate::local::KernelConnectionClass::ExternalAgent);
            command.caller.caller_id = grant;
            room.router.dispatch(command, request).await.unwrap();
            {
                let app = room.router.app.lock().await;
                let session = app.sessions().get_session(&room.session).unwrap();
                let prompts = app.prompt_state_owner();
                prompts
                    .complete_active_prompt_only(&session, agent.id())
                    .unwrap();
                let promoted = prompts
                    .activate_next_queued_prompt(&session, agent.id(), Some(queued.id()))
                    .unwrap()
                    .unwrap();
                assert_eq!(promoted.prompt(), "Automation requests my browser");
                assert!(
                    !promoted.owner_request(),
                    "MP-11 R1: editing automation owns acquisition provenance"
                );
            }
            let attempt = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                tool(
                    &room.router,
                    &token,
                    "chariox.load_kernel_browser",
                    json!({}),
                ),
            )
            .await
            .unwrap();
            assert_eq!(
                refusal(attempt.unwrap_err()),
                Some(crate::error::UserDomainRefusalReason::NotRequested)
            );
            assert!(grants(&room.router).await.is_empty());
            assert!(
                borrowed.is_err(),
                "MP-11 R1: external edits cannot borrow a human attachment"
            );
            room.router
                .runtime_state()
                .shutdown_cleanup()
                .await
                .unwrap();
            std::fs::remove_dir_all(&room.root).unwrap();
        })
    });
}

// MP-08/MP-10/MP-11 R2: same prompt, different run or delivery witness.
#[test]
fn capability_review_r2_browser_decision_withdraws_on_same_prompt_replacement() {
    run_test(|| {
        Box::pin(async {
            for delivery_only in [false, true] {
                let setup = setup("capability-r2-replacement", &[], &[]);
                let (agent, token) = setup.agents[0].clone();
                let room = start(setup, agent.owner_user_id());
                let prompt_id = running_prompt(
                    &room,
                    agent.id(),
                    ClientCapabilityLevel::FullTerminal,
                    "owner",
                )
                .await;
                let (attempt, ()) = tokio::join!(
                    tokio::time::timeout(
                        std::time::Duration::from_secs(2),
                        tool(
                            &room.router,
                            &token,
                            "chariox.load_kernel_browser",
                            json!({})
                        )
                    ),
                    async {
                        tokio::time::timeout(std::time::Duration::from_secs(1), async {
                            loop {
                                let snapshot = room
                                    .router
                                    .runtime_state()
                                    .session_snapshot(&room.session)
                                    .await
                                    .unwrap();
                                if snapshot
                                    .active_interactions()
                                    .iter()
                                    .any(|i| i.title() == Some("Chariox resource access"))
                                {
                                    break;
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                            }
                        })
                        .await
                        .unwrap();
                        let mut app = room.router.app.lock().await;
                        if delivery_only {
                            let session = app.sessions().get_session(&room.session).unwrap();
                            let prompts = app.prompt_state_owner();
                            let active = prompts
                                .active_prompt_for_agent_snapshot(&session, agent.id())
                                .unwrap();
                            let mut changed = active.clone();
                            changed.set_durable_delivery(
                                crate::session::DurablePromptDeliveryPhase::Delivered,
                                Some("replacement-run".into()),
                                None,
                            );
                            assert!(prompts.replace_active_prompt_if_matches(
                                &session,
                                agent.id(),
                                &active,
                                changed
                            ));
                        } else {
                            let run = app
                                .providers()
                                .get_run_for_agent(&room.session, agent.id())
                                .unwrap();
                            app.providers_mut()
                                .mark_run_ended_provider_only(&room.session, run.id())
                                .unwrap();
                            launch_test_provider(
                                &mut app,
                                &room.session,
                                agent.id(),
                                "dev-stub",
                                "dev-stub",
                                "native-tui-idle",
                            );
                        }
                    }
                );
                assert!(attempt
                    .expect("MP-11 R2: stale browser acquisition must finish without owner reply")
                    .is_err());
                let snapshot = room
                    .router
                    .runtime_state()
                    .session_snapshot(&room.session)
                    .await
                    .unwrap();
                assert!(
                    snapshot
                        .active_interactions()
                        .iter()
                        .all(|i| i.title() != Some("Chariox resource access")),
                    "MP-11 R2: stale popup withdrawn"
                );
                assert_eq!(
                    room.router
                        .runtime_state()
                        .owner_requested_prompt(&agent)
                        .as_deref(),
                    Some(prompt_id.as_str())
                );
                assert!(grants(&room.router).await.is_empty());
                room.router
                    .runtime_state()
                    .shutdown_cleanup()
                    .await
                    .unwrap();
                std::fs::remove_dir_all(&room.root).unwrap();
            }
        })
    });
}
