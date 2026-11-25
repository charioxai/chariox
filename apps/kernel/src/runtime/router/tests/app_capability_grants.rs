//! MP-08/MP-10/MP-11 A05: every agent binds the owner's trusted Apps with no
//! card; transfers follow direct lineage on the room surface.
use super::*;
use crate::attachment::{AttachRequest, ClientCapabilityLevel};
use crate::extension::{ExtensionGrant, ExtensionKind};

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Room {
    _scratch: Scratch,
    app: Arc<Mutex<DaemonApp>>,
    router: CommandRouter,
    session: String,
    /// (parent, child spawned by parent, unrelated peer) with run tokens.
    agents: [(String, String); 3],
}

fn room() -> Room {
    let scratch = Scratch(std::env::temp_dir().join(format!(
        "chariox-app-capability-{:016x}",
        rand::random::<u64>()
    )));
    std::fs::create_dir(&scratch.0).unwrap();
    let mut config = DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    crate::durable_state::app_state::fixture_catalog(&app.durable_state_store());
    let session = app
        .sessions_mut()
        .create_session(
            CreateSessionRequest::new(scratch.0.to_string_lossy(), scratch.0.to_string_lossy())
                .with_owner_user_id("alice"),
        )
        .unwrap();
    let spawn = |app: &mut DaemonApp, creator: Option<&str>| {
        let mut request = CreateAgentRequest::new(session.id(), "dev-stub")
            .with_owner_user_id("alice")
            .with_permission_level_override(crate::provider::AgentPermissionLevel::Yolo);
        if let Some(creator) = creator {
            request = request.with_parent_agent_id(creator);
        }
        let agent = crate::app::KernelSessionService::new(app)
            .spawn_agent(request)
            .unwrap();
        let run = launch_test_provider(
            app,
            session.id(),
            agent.id(),
            "dev-stub",
            "dev-stub",
            "native-tui-idle",
        );
        (
            agent.id().to_string(),
            run.runtime_mcp_auth_token().unwrap().to_string(),
        )
    };
    let parent = spawn(&mut app, None);
    let child = spawn(&mut app, Some(&parent.0));
    let peer = spawn(&mut app, None);
    // MP-11: acquisition provenance belongs to the test's explicit prompt,
    // not the synthetic owner turn admitted by launch_test_provider.
    for (agent, _) in [&parent, &child, &peer] {
        app.prompt_owner_complete_active_prompt_only(session.id(), agent)
            .unwrap();
    }
    let app = Arc::new(Mutex::new(app));
    Room {
        _scratch: scratch,
        router: CommandRouter::with_interactive_capacity(app.clone(), 4),
        app,
        session: session.id().into(),
        agents: [parent, child, peer],
    }
}

async fn running_prompt(room: &Room, agent: &str, level: ClientCapabilityLevel, client: &str) {
    let mut app = room.app.lock().await;
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(AttachRequest::for_user(
            &room.session,
            client,
            level,
            "alice",
        ))
        .unwrap();
    app.submit_prompt(
        &room.session,
        attachment.id(),
        Some(agent),
        "Use the installed App",
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
}

async fn bound(room: &Room, agent: &str) -> bool {
    room.app
        .lock()
        .await
        .agents()
        .get_agent(agent)
        .unwrap()
        .has_extension_grant(ExtensionKind::App, "installed")
}

async fn request_app(room: &Room, token: &str, name: &str) -> bool {
    room.router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            token,
            crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL,
            serde_json::json!({"kind":"app","name":name}),
        )
        .await
        .is_ok_and(|result| result.ok)
}

async fn room_command(room: &Room, token: &str, command: String) -> bool {
    room.router
        .dispatch_authenticated_runtime_tool_call(
            token,
            "chariox.room.run_command",
            serde_json::json!({ "command": command }),
        )
        .await
        .is_ok_and(|result| result.ok)
}

/// Every agent binds the owner's trusted App, in any turn, with no card.
#[tokio::test]
async fn an_agent_binds_a_trusted_app_in_any_turn_without_a_card() {
    let room = room();
    let (agent, token) = room.agents[2].clone();
    // A turn caused by another agent's message.
    running_prompt(
        &room,
        &agent,
        ClientCapabilityLevel::AutomationOnly,
        &format!("agent:{}:messages", room.agents[0].0),
    )
    .await;
    assert!(
        !request_app(&room, &token, "not-installed").await,
        "MP-11: untrusted or absent installations are never bound"
    );
    assert!(request_app(&room, &token, "installed").await);
    assert!(bound(&room, &agent).await);
    let session = room
        .router
        .runtime_state
        .session_snapshot(&room.session)
        .await
        .unwrap();
    assert!(session.active_interactions().is_empty());
}

/// An agent binds the owner's trusted App to a descendant with no step of its own: the parent
/// needs no binding to hand over, a peer outside its subtree is refused, and revoking one
/// binding revokes only that one.
#[tokio::test]
async fn an_agent_binds_the_owners_app_to_a_descendant_and_revokes_each_binding_alone() {
    let room = room();
    let [(parent, parent_token), (child, _), (peer, _)] = room.agents.clone();
    running_prompt(&room, &parent, ClientCapabilityLevel::FullTerminal, "owner").await;
    assert!(
        !room_command(
            &room,
            &parent_token,
            format!("extension grant app {peer} installed")
        )
        .await,
        "MP-08: binding is limited to the caller's own subtree"
    );
    assert!(!bound(&room, &peer).await);
    assert!(
        room_command(
            &room,
            &parent_token,
            format!("extension grant app {child} installed")
        )
        .await
    );
    assert!(bound(&room, &child).await);
    assert!(request_app(&room, &parent_token, "installed").await);
    assert!(
        room_command(
            &room,
            &parent_token,
            format!("extension revoke app {parent} installed")
        )
        .await
    );
    assert!(!bound(&room, &parent).await);
    assert!(
        bound(&room, &child).await,
        "a binding is revoked alone: there is no delegation chain"
    );
}

/// MP-08 (#922 review 2): a room transfer of a running App refreshes the
/// target's provider catalog like every other grant path.
#[tokio::test]
async fn capability_room_app_transfer_refreshes_the_target_catalog() {
    let room = room();
    let [(parent, parent_token), (child, _), _] = room.agents.clone();
    running_prompt(&room, &parent, ClientCapabilityLevel::FullTerminal, "owner").await;
    assert!(request_app(&room, &parent_token, "installed").await);
    // The App is dormant: its tools are listable, so a new binding is due a refresh.
    let control = room.router.runtime_state.app_control();
    crate::runtime::app_lifecycle::tests::stage(&room.app.lock().await.durable_state_store());
    if let Some(lease) = control.active_app_lease("alice", "installed") {
        let idle = control.clone();
        tokio::task::spawn_blocking(move || {
            idle.lifecycle()
                .idle_stop_blocking("alice", lease.catalog().clone(), || true)
        })
        .await
        .unwrap()
        .unwrap();
    } else {
        // The start gate can still be closed by the approval's own lifecycle
        // step on a loaded builder; seed again until it admits the catalog.
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !control.is_app_dormant("alice", "installed") {
                control.seed_dormant("alice", "installed");
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the idle App keeps a dormant catalog");
    }
    assert!(control.active_app_lease("alice", "installed").is_none());
    assert!(control.is_app_dormant("alice", "installed"));
    running_prompt(&room, &child, ClientCapabilityLevel::FullTerminal, "owner").await;
    assert!(
        room_command(
            &room,
            &parent_token,
            format!("extension grant app {child} installed")
        )
        .await
    );
    assert!(bound(&room, &child).await);
    assert!(
        room.router
            .runtime_state
            .pending_provider_reload_for_test(&child),
        "the busy child's catalog refresh is queued"
    );
}

#[tokio::test]
async fn capability_app_peer_cannot_revoke_or_grant_foreign_bindings() {
    let room = room();
    let [(parent, _), (child, _), (peer, peer_token)] = room.agents.clone();
    room.router
        .runtime_state
        .grant_agent_extension(&child, ExtensionGrant::app("installed"), "alice")
        .await
        .unwrap();
    running_prompt(&room, &peer, ClientCapabilityLevel::FullTerminal, "owner").await;
    assert!(
        !room_command(
            &room,
            &peer_token,
            format!("extension revoke app {child} installed")
        )
        .await
    );
    assert!(
        !room_command(
            &room,
            &peer_token,
            format!("extension grant app {parent} installed")
        )
        .await
    );
    assert!(bound(&room, &child).await);
}

#[tokio::test]
async fn capability_stale_home_run_cannot_acquire_after_agent_is_leased() {
    let room = room();
    let (agent, token) = room.agents[0].clone();
    room.router
        .runtime_state
        .grant_agent_extension(&agent, ExtensionGrant::app("installed"), "alice")
        .await
        .unwrap();
    room.app
        .lock()
        .await
        .agents()
        .bind_remote_execution(
            &agent,
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker".into(),
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
    running_prompt(&room, &agent, ClientCapabilityLevel::FullTerminal, "owner").await;
    assert!(
        !request_app(&room, &token, "installed").await,
        "MP-11 D1: an old local provider token cannot authorize the leased actor"
    );
    room.app
        .lock()
        .await
        .agents()
        .set_remote_execution_active_worker_provider_run_id(&agent, Some("worker-run".into()))
        .unwrap();
    let context = crate::transport::relay_peer::RemoteWorkspaceLiveSyncContext {
        home_kernel_id: room.app.lock().await.config().daemon_id.clone(),
        home_session_id: room.session.clone(),
        home_agent_id: agent.clone(),
        home_prompt_id: None,
        leased_agent_id: "worker-agent".into(),
        worker_kernel_id: "worker".into(),
        worker_machine_id: "worker-machine".into(),
        worker_provider_run_id: "worker-run".into(),
        worker_worktree_path: room._scratch.0.to_string_lossy().into(),
        worker_workspace_identity: crate::io::WorkspaceIdentity::local(
            room._scratch.0.to_string_lossy(),
        ),
    };
    let home = room.router.runtime_state.with_relay_peer_authority(
        crate::runtime::relay_peer_authority::test_peer_authority("worker"),
    );
    let (result, _) = home
        .dispatch_forwarded_capability_runtime_tool_call(
            context.clone(),
            crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL.into(),
            serde_json::json!({"kind":"app","name":"installed"}),
        )
        .await
        .unwrap();
    assert!(
        result.ok,
        "MP-08 D1: an admitted leased epoch reuses its home App binding"
    );
    let mut stale = context;
    stale.worker_provider_run_id = "replaced-run".into();
    assert!(home
        .dispatch_forwarded_capability_runtime_tool_call(
            stale,
            crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL.into(),
            serde_json::json!({"kind":"app","name":"installed"})
        )
        .await
        .is_err());

    assert!(
        room.router
            .runtime_tool_specs_for_auth_token(&token)
            .iter()
            .all(|tool| !tool.name.starts_with("app_")),
        "MP-11: a tool-less catalog still publishes no App tools"
    );
}

#[tokio::test]
async fn capability_agent_origin_cannot_use_an_owner_only_app_binding_helper() {
    let room = room();
    let (agent, _) = room.agents[0].clone();
    let run = room
        .app
        .lock()
        .await
        .providers()
        .get_run_for_agent(&room.session, &agent)
        .unwrap();
    let state = room
        .router
        .runtime_state
        .with_room_provider_origin(Some(&agent), Some(run.id()));
    assert!(
        state
            .grant_agent_extension(&agent, ExtensionGrant::app("installed"), "alice")
            .await
            .is_err(),
        "MP-11: indirect agent acquisition cannot impersonate an owner grant"
    );
    assert!(!bound(&room, &agent).await);
}

#[tokio::test]
async fn capability_agents_cannot_supply_safety_or_environment_overrides() {
    let room = room();
    let (agent, token) = room.agents[2].clone();
    running_prompt(&room, &agent, ClientCapabilityLevel::FullTerminal, "owner").await;
    for args in [
        serde_json::json!({"kind":"app","name":"installed","allow":"write"}),
        serde_json::json!({"kind":"app","name":"installed","environment":"elsewhere"}),
    ] {
        assert!(
            room.router
                .runtime_state
                .dispatch_authenticated_runtime_tool_call(
                    &token,
                    crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL,
                    args,
                )
                .await
                .is_err(),
            "MP-11: caller-supplied authority must be refused"
        );
    }
    assert!(!bound(&room, &agent).await);
}
