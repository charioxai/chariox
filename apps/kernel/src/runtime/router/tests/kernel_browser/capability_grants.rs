//! Every local agent uses the owner's kernel browser through the real router and
//! provider MCP admission, with no grant, card, focus or delegation. The browser
//! is the fixture controller; live Chromium/provider acceptance is a separate
//! recorded gate.
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

/// Owner-created first agent, a chain of `spawned` descendants (each one the
/// child of the agent before it), then unrelated `peers`. Every agent has a
/// dev-stub run.
fn setup(name: &str, spawned: &[&str], peers: &[&str]) -> Setup {
    let workspace = crate::test_support::TestWorktree::new(name);
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, first) = crate::app::KernelSessionService::new(&mut app)
        .create_session(workspace.session_request())
        .unwrap();
    // A02: spawning children requires the creator's live turn.
    let launch = |app: &mut DaemonApp, agent: &crate::agent::AgentInstance| {
        let run = launch_test_provider(
            app,
            session.id(),
            agent.id(),
            "dev-stub",
            "dev-stub",
            "native-tui-idle",
        );
        run.runtime_mcp_auth_token().unwrap().to_string()
    };
    let mut tokens = vec![launch(&mut app, &first)];
    let mut agents = vec![first.clone()];
    for child in spawned {
        let parent = agents.last().unwrap().id().to_owned();
        let child = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias(*child)
                    .with_parent_agent_id(&parent),
            )
            .unwrap();
        tokens.push(launch(&mut app, &child));
        agents.push(child);
    }
    for peer in peers {
        let peer = spawn_test_agent(&mut app, session.id(), peer, "dev-stub");
        tokens.push(launch(&mut app, &peer));
        agents.push(peer);
    }
    // MP-11: retire the launch helper's synthetic owner turn before each test
    // submits the human or automation prompt whose causation it exercises.
    for agent in &agents {
        app.prompt_owner_complete_active_prompt_only(session.id(), agent.id())
            .unwrap();
    }
    let agents = agents.into_iter().zip(tokens).collect();
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

fn snapshot(tab: &str) -> Value {
    json!({"command":{"op":"snapshot","tab_id":tab,"generation":1}})
}

/// Row 322 (Miguel 2026-11-21): no grant, card, focus or delegation. An agent at depth 3 and an
/// idle peer use the owner's existing tab and open their own with no load, no
/// transfer from a parent and no owner prompt, and keep doing so across calls.
#[test]
fn every_agent_uses_the_owners_browser_with_no_grant_at_any_depth() {
    run_test(|| {
        Box::pin(async {
            let setup = setup(
                "browser-no-grant",
                &["child", "grandchild", "great-grandchild"],
                &["idle"],
            );
            let agents = setup.agents.clone();
            let room = start(setup, agents[0].0.owner_user_id());
            let router = &room.router;
            for (agent, token) in &agents {
                tool(
                    router,
                    token,
                    "chariox.kernel_browser",
                    snapshot("host-tab-fixture"),
                )
                .await
                .unwrap_or_else(|error| panic!("{} reads the owner's tab: {error}", agent.id()));
                tool(
                    router,
                    token,
                    "chariox.kernel_browser",
                    json!({"command":{"op":"open","url":"https://www.wikipedia.org/"}}),
                )
                .await
                .unwrap_or_else(|error| panic!("{} opens a tab: {error}", agent.id()));
                let loaded = tool(router, token, "chariox.load_kernel_browser", json!({}))
                    .await
                    .unwrap_or_else(|error| panic!("{} lists the browser: {error}", agent.id()));
                assert_eq!(loaded.payload["loaded"], "chariox.kernel_browser");
                assert!(
                    loaded.payload.get("granted").is_none(),
                    "loading grants nothing"
                );
                assert!(
                    tool(
                        router,
                        token,
                        "chariox.load_kernel_browser",
                        json!({"lifetime_hours": 4}),
                    )
                    .await
                    .is_err(),
                    "the loader takes no lifetime"
                );
                tool(
                    router,
                    token,
                    "chariox.kernel_browser",
                    snapshot("host-tab-fixture"),
                )
                .await
                .expect("still no refusal after loading");
            }
            let session = router
                .runtime_state()
                .session_snapshot(&room.session)
                .await
                .unwrap();
            assert!(
                session.active_interactions().is_empty(),
                "no card: {:?}",
                session
                    .active_interactions()
                    .iter()
                    .map(|interaction| interaction.title())
                    .collect::<Vec<_>>()
            );
            // The share tool is gone: there is nothing to delegate.
            assert!(tool(
                router,
                &agents[0].1,
                "chariox.kernel_browser_share",
                json!({"agent": agents[1].0.id(), "resources": []}),
            )
            .await
            .is_err());
            router.runtime_state().shutdown_cleanup().await.unwrap();
            std::fs::remove_dir_all(&room.root).unwrap();
        })
    });
}

#[test]
fn leased_agents_keep_only_their_room_browser_route() {
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
                        worker_worktree_id: None,
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
            tool(
                router,
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
            router.runtime_state().shutdown_cleanup().await.unwrap();
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
