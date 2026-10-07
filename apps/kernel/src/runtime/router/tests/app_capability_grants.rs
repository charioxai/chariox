//! MP-08/MP-10/MP-11 A05: App bindings follow the user's request and direct
//! lineage on the room surface. Trusted-installation checks are unchanged.
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
            request = request.with_spawned_by_agent_id(creator);
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

#[tokio::test]
async fn capability_owner_prompt_alone_cannot_mint_an_app_binding() {
    let room = room();
    let (agent, token) = room.agents[2].clone();
    running_prompt(&room, &agent, ClientCapabilityLevel::FullTerminal, "owner").await;
    let attempt = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        request_app(&room, &token, "installed"),
    )
    .await;
    assert!(
        attempt.is_err(),
        "MP-11: acquisition needs explicit resource approval, even in YOLO mode"
    );
    assert!(
        !bound(&room, &agent).await,
        "MP-11: human prompt alone grants no installation"
    );
}

#[tokio::test]
async fn capability_app_self_binding_requires_the_users_request() {
    let room = room();
    let (agent, token) = room.agents[2].clone();
    // A turn caused by another agent's message, even in YOLO mode.
    running_prompt(
        &room,
        &agent,
        ClientCapabilityLevel::AutomationOnly,
        &format!("agent:{}:messages", room.agents[0].0),
    )
    .await;
    assert!(
        !request_app(&room, &token, "installed").await,
        "MP-08: unrequested App acquisition"
    );
    assert!(!bound(&room, &agent).await);

    let room = self::room();
    let (agent, token) = room.agents[2].clone();
    running_prompt(&room, &agent, ClientCapabilityLevel::FullTerminal, "owner").await;
    assert!(
        !request_app(&room, &token, "not-installed").await,
        "MP-11: untrusted or absent installations are never bound"
    );
    assert!(approved_app(&room, &token, "installed").await);
    assert!(bound(&room, &agent).await);
}

#[tokio::test]
async fn capability_app_child_transfer_is_a_held_subset_and_revocation_cascades() {
    let room = room();
    let [(parent, parent_token), (child, _), (peer, _)] = room.agents.clone();
    running_prompt(&room, &parent, ClientCapabilityLevel::FullTerminal, "owner").await;
    assert!(
        !room_command(
            &room,
            &parent_token,
            format!("extension grant app {child} installed")
        )
        .await,
        "MP-08: a parent cannot hand over an App it does not hold"
    );
    assert!(approved_app(&room, &parent_token, "installed").await);
    assert!(
        !room_command(
            &room,
            &parent_token,
            format!("extension grant app {peer} installed")
        )
        .await,
        "MP-08: transfer is limited to a direct child"
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
        !bound(&room, &child).await,
        "MP-11: a revoked parent binding is revoked from its descendants"
    );
}

/// MP-08 (#922 review 2): a room transfer of a running App refreshes the
/// target's provider catalog like every other grant path.
#[tokio::test]
async fn capability_room_app_transfer_refreshes_the_target_catalog() {
    let room = room();
    let [(parent, parent_token), (child, _), _] = room.agents.clone();
    running_prompt(&room, &parent, ClientCapabilityLevel::FullTerminal, "owner").await;
    assert!(approved_app(&room, &parent_token, "installed").await);
    // The App is dormant: its tools are listable, so a new binding is due a refresh.
    let control = room.router.runtime_state.app_control();
    crate::runtime::app_lifecycle::tests::stage(&room.app.lock().await.durable_state_store());
    control.seed_dormant("alice", "installed");
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
async fn capability_leased_agents_cannot_publish_or_retain_app_tools() {
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
            },
        )
        .unwrap();
    running_prompt(&room, &agent, ClientCapabilityLevel::FullTerminal, "owner").await;
    assert!(
        !request_app(&room, &token, "installed").await,
        "MP-08: a prior binding cannot authorize leased App acquisition"
    );
    assert!(
        room.router
            .runtime_tool_specs_for_auth_token(&token)
            .iter()
            .all(|tool| !tool.name.starts_with("app_")),
        "MP-11: leased runs publish no App tools"
    );
}

#[tokio::test]
async fn capability_deleting_an_app_grant_source_revokes_its_child() {
    let room = room();
    let [(parent, parent_token), (child, _), (peer, _)] = room.agents.clone();
    for agent in [&parent, &peer] {
        room.router
            .runtime_state
            .grant_agent_extension(agent, ExtensionGrant::app("installed"), "alice")
            .await
            .unwrap();
    }
    assert!(
        room_command(
            &room,
            &parent_token,
            format!("extension grant app {child} installed")
        )
        .await
    );
    assert!(bound(&room, &child).await);
    room.router
        .runtime_state
        .destroy_agent(&parent, "alice")
        .await
        .unwrap();
    assert!(
        !bound(&room, &child).await,
        "MP-11: source deletion revokes delegated authority"
    );
    assert!(
        bound(&room, &peer).await,
        "MP-11: independent owner grants survive"
    );
}

#[tokio::test]
async fn capability_pending_owner_decision_wakes_when_its_turn_ends() {
    let room = room();
    let (agent, token) = room.agents[0].clone();
    running_prompt(&room, &agent, ClientCapabilityLevel::FullTerminal, "owner").await;
    let (attempt, ()) = tokio::join!(
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            request_app(&room, &token, "installed")
        ),
        async {
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let session = room
                        .router
                        .runtime_state
                        .session_snapshot(&room.session)
                        .await
                        .unwrap();
                    if session
                        .active_interactions()
                        .iter()
                        .any(|item| item.title() == Some("Chariox resource access"))
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            let app = room.app.lock().await;
            let prompts = app.prompt_state_owner();
            let session = app.sessions().get_session(&room.session).unwrap();
            let active = prompts
                .active_prompt_for_agent_snapshot(&session, &agent)
                .unwrap();
            let mut cancelled = active.clone();
            cancelled.set_status(crate::session::PromptStatus::Cancelled);
            assert!(prompts.replace_active_prompt_if_matches(&session, &agent, &active, cancelled));
        }
    );
    assert!(!attempt.expect("MP-11: turn loss must wake acquisition without an owner reply"));
    assert!(!bound(&room, &agent).await);
    assert!(
        room.router
            .runtime_state
            .session_snapshot(&room.session)
            .await
            .unwrap()
            .active_interactions()
            .iter()
            .all(|item| item.title() != Some("Chariox resource access")),
        "MP-11: the stale owner popup is withdrawn"
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

async fn approved_app(room: &Room, token: &str, name: &str) -> bool {
    let (result, ()) = tokio::join!(
        request_app(room, token, name),
        answer_app_decision(room, "allow")
    );
    result
}

/// MP-08/MP-11 (#922 round 3): App and Browser Deny share the typed refusal.
#[tokio::test]
async fn capability_review_round3_app_deny_is_typed_not_requested() {
    let room = room();
    let (agent, token) = room.agents[0].clone();
    running_prompt(&room, &agent, ClientCapabilityLevel::FullTerminal, "owner").await;
    let (result, ()) = tokio::join!(
        room.router
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &token,
                crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL,
                serde_json::json!({"kind":"app","name":"installed"}),
            ),
        answer_app_decision(&room, "deny")
    );
    assert!(
        matches!(
            result,
            Err(DaemonError::UserDomainRefused {
                reason: crate::error::UserDomainRefusalReason::NotRequested
            })
        ),
        "MP-11: owner Deny must return user_domain_not_requested: {result:?}"
    );
    assert!(!bound(&room, &agent).await);
    assert!(room
        .router
        .runtime_state
        .session_snapshot(&room.session)
        .await
        .unwrap()
        .active_interactions()
        .iter()
        .all(|item| item.title() != Some("Chariox resource access")));
    room.router.runtime_state.shutdown_cleanup().await.unwrap();
}

async fn answer_app_decision(room: &Room, choice: &str) {
    let interaction = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let session = room
                .router
                .runtime_state
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
    assert!(
        room.router
            .runtime_state
            .answer_terminal_runtime_interaction(
                &room.session,
                &interaction,
                "allow",
                None,
                Some("alice"),
                None,
                None,
                Some(crate::local::KernelConnectionClass::KernelAgent),
            )
            .await
            .is_err(),
        "MP-11: an agent cannot answer its resource approval"
    );
    assert!(
        room.router
            .runtime_state
            .resolve_runtime_interaction(&room.session, &interaction, "allow", None,)
            .await
            .is_err(),
        "MP-11: ownerless agent replies cannot mint authority"
    );
    room.router
        .runtime_state
        .resolve_terminal_runtime_interaction(
            &room.session,
            &interaction,
            choice,
            None,
            Some("alice"),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn capability_agents_cannot_supply_app_authority_or_safety_overrides() {
    let room = room();
    let (agent, token) = room.agents[2].clone();
    running_prompt(&room, &agent, ClientCapabilityLevel::FullTerminal, "owner").await;
    for args in [
        serde_json::json!({"kind":"app","name":"installed","allow":"write"}),
        serde_json::json!({"kind":"app","name":"installed","app_grant":{"grant_id":"external","expires_at_ms":u64::MAX}}),
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
