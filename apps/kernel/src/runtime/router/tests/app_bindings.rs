use super::*;
use crate::extension::{ExtensionGrant, ExtensionKind};

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-app-binding-router-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn router(
        &self,
        permission: crate::provider::AgentPermissionLevel,
    ) -> (Arc<Mutex<DaemonApp>>, CommandRouter, String, String, String) {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        crate::durable_state::app_state::fixture_catalog(&app.durable_state_store());
        let session = app
            .sessions_mut()
            .create_session(
                CreateSessionRequest::new(self.0.to_string_lossy(), self.0.to_string_lossy())
                    .with_owner_user_id("alice"),
            )
            .unwrap();
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_owner_user_id("alice")
                    .with_permission_level_override(permission),
            )
            .unwrap();
        let run = launch_test_provider(
            &mut app,
            session.id(),
            agent.id(),
            "dev-stub",
            "dev-stub",
            "binding-model",
        );
        let auth = run.runtime_mcp_auth_token().unwrap().to_string();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(app.clone(), 4);
        (app, router, session.id().into(), agent.id().into(), auth)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn app_discovery_consumes_its_cursor_without_exposing_another_owners_installations() {
    let fixture = Fixture::new();
    let (app, router, _session, _agent, auth) =
        fixture.router(crate::provider::AgentPermissionLevel::Yolo);
    let store = app.lock().await.durable_state_store();
    let release = store
        .get_app_installation("alice", "installed")
        .unwrap()
        .active
        .unwrap()
        .release;
    for (owner, id) in (0..101)
        .map(|index| ("alice", format!("app-{index:03}")))
        .chain(std::iter::once(("bob", "app-099-private".into())))
    {
        store
            .mutate_app_installation(
                owner,
                crate::durable_state::apps::AppRegistryMutation::CreateAndStage {
                    installation_id: id,
                    release: release.clone(),
                    now_ms: 1,
                },
            )
            .unwrap();
    }
    let call = |arguments| {
        router
            .runtime_state
            .dispatch_authenticated_runtime_tool_call(
                &auth,
                crate::transport::runtime_tools::LIST_EXTENSIONS_TOOL,
                arguments,
            )
    };
    let first = call(serde_json::json!({"kind":"app"})).await.unwrap();
    assert!(first.ok);
    let first_page = first.payload["extensions"]["apps"].as_array().unwrap();
    assert_eq!(first_page.len(), 100);
    assert_eq!(first_page[0]["name"], "app-000");
    let cursor = first.payload["extensions"]["apps_next_cursor"]
        .as_str()
        .unwrap();
    assert_eq!(cursor, "app-099");
    let second = call(serde_json::json!({"kind":"app","apps_cursor":cursor}))
        .await
        .unwrap();
    assert!(second.ok);
    let second_page = second.payload["extensions"]["apps"].as_array().unwrap();
    assert_eq!(
        second_page
            .iter()
            .map(|app| app["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["app-100", "installed"]
    );
    assert!(second.payload["extensions"]["apps_next_cursor"].is_null());
    for arguments in [
        serde_json::json!({"kind":"mcp","apps_cursor":cursor}),
        serde_json::json!({"kind":"app","apps_cursor":"x".repeat(129)}),
        serde_json::json!({"kind":"app","apps_cursor":""}),
        serde_json::json!({"kind":"app","apps_cursor":"bad\nvalue"}),
        serde_json::json!({"kind":"app","apps_cursor":cursor,"owner":"bob"}),
    ] {
        if let Ok(result) = call(arguments).await {
            assert!(!result.ok)
        }
    }
}

#[tokio::test]
async fn explicit_app_grant_checks_owner_and_saves_an_offline_binding() {
    let fixture = Fixture::new();
    let (_app, router, _session, agent, _auth) =
        fixture.router(crate::provider::AgentPermissionLevel::Required);
    assert!(router
        .runtime_state
        .grant_agent_extension(&agent, ExtensionGrant::app("installed"), "bob")
        .await
        .is_err());
    assert!(router
        .runtime_state
        .grant_agent_extension(&agent, ExtensionGrant::app("missing"), "alice")
        .await
        .is_err());
    let updated = router
        .runtime_state
        .grant_agent_extension(&agent, ExtensionGrant::app("installed"), "alice")
        .await
        .unwrap();
    assert!(updated.has_extension_grant(ExtensionKind::App, "installed"));
    assert!(!updated.has_extension_grant(ExtensionKind::Mcp, "installed"));
    let updated = router
        .runtime_state
        .revoke_agent_extension(&agent, ExtensionKind::App, "installed", "alice")
        .await
        .unwrap();
    assert!(!updated.has_extension_grant(ExtensionKind::App, "installed"));
}

#[tokio::test]
async fn cancelled_grant_retains_shared_admission_until_the_writer_finishes() {
    let fixture = Fixture::new();
    let (app, router, _session, agent, _auth) =
        fixture.router(crate::provider::AgentPermissionLevel::Yolo);
    let control = router.runtime_state.app_control().clone();
    let permits = (0..8)
        .map(|_| control.try_admit().unwrap())
        .collect::<Vec<_>>();
    assert!(router
        .runtime_state
        .grant_agent_extension(&agent, ExtensionGrant::app("installed"), "alice")
        .await
        .is_err());
    drop(permits);
    let path = app.lock().await.durable_state_store().path().to_path_buf();
    let lock = rusqlite::Connection::open(path).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let state = router.runtime_state.clone();
    let target = agent.clone();
    let pending = tokio::spawn(async move {
        state
            .grant_agent_extension(&target, ExtensionGrant::app("installed"), "alice")
            .await
    });
    let admitted = timeout(Duration::from_secs(2), async {
        loop {
            let held = (0..8)
                .filter_map(|_| control.try_admit().ok())
                .collect::<Vec<_>>();
            if held.len() == 7 {
                break;
            }
            drop(held);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    pending.abort();
    let _ = pending.await;
    let retained = (0..8)
        .filter_map(|_| control.try_admit().ok())
        .collect::<Vec<_>>();
    let retained_count = retained.len();
    drop(retained);
    // Always unlock before any assertion so a failed fixture cannot strand the writer.
    lock.execute_batch("ROLLBACK").unwrap();
    let drained = timeout(Duration::from_secs(2), async {
        loop {
            let held = (0..8)
                .filter_map(|_| control.try_admit().ok())
                .collect::<Vec<_>>();
            if held.len() == 8 {
                break;
            }
            drop(held);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(admitted.is_ok());
    assert_eq!(retained_count, 7);
    assert!(drained.is_ok());
    assert!(!app
        .lock()
        .await
        .agents()
        .get_agent(&agent)
        .unwrap()
        .has_extension_grant(ExtensionKind::App, "installed"));
}

#[tokio::test]
async fn yolo_app_self_grant_saves_binding_without_claiming_ready_tools() {
    let fixture = Fixture::new();
    let (app, router, session, agent, auth) =
        fixture.router(crate::provider::AgentPermissionLevel::Yolo);
    let result = router
        .runtime_state
        .dispatch_authenticated_runtime_tool_call(
            &auth,
            crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL,
            serde_json::json!({"kind":"app","name":"installed"}),
        )
        .await
        .unwrap();
    assert!(result.ok);
    assert_eq!(result.payload["effective"], "binding_saved");
    assert_eq!(result.payload["tools_available"], false);
    assert!(app
        .lock()
        .await
        .sessions()
        .get_session(&session)
        .unwrap()
        .active_interaction_for_agent(&agent)
        .is_none());
}

#[tokio::test]
async fn self_grant_of_an_already_bound_app_changes_nothing_and_asks_nothing() {
    for level in [
        crate::provider::AgentPermissionLevel::Yolo,
        crate::provider::AgentPermissionLevel::Required,
    ] {
        let fixture = Fixture::new();
        let (app, router, session, agent, auth) = fixture.router(level);
        router
            .runtime_state
            .grant_agent_extension(&agent, ExtensionGrant::app("installed"), "alice")
            .await
            .unwrap();
        // Under Ask, a new binding would wait for a decision; an existing one
        // answers at once.
        let result = timeout(
            Duration::from_secs(2),
            router
                .runtime_state
                .dispatch_authenticated_runtime_tool_call(
                    &auth,
                    crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL,
                    serde_json::json!({"kind":"app","name":"installed"}),
                ),
        )
        .await
        .expect("an already-bound App must not wait for an approval")
        .unwrap();
        assert!(result.ok);
        assert_eq!(result.payload["effective"], "already_bound");
        assert_eq!(result.payload["requires_provider_restart"], false);
        assert!(app
            .lock()
            .await
            .sessions()
            .get_session(&session)
            .unwrap()
            .active_interaction_for_agent(&agent)
            .is_none());
        assert!(granted(&app, &agent).await);
    }
}

#[tokio::test]
async fn ask_app_self_grant_waits_for_the_existing_permission_interaction() {
    let fixture = Fixture::new();
    let (app, router, session, agent, auth) =
        fixture.router(crate::provider::AgentPermissionLevel::Required);
    for choice in ["deny", "allow"] {
        let state = router.runtime_state.clone();
        let token = auth.clone();
        let pending = tokio::spawn(async move {
            state
                .dispatch_authenticated_runtime_tool_call(
                    &token,
                    crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL,
                    serde_json::json!({"kind":"app","name":"installed"}),
                )
                .await
        });
        let interaction_id = timeout(Duration::from_secs(2), async {
            loop {
                if let Some(id) = app
                    .lock()
                    .await
                    .sessions()
                    .get_session(&session)
                    .unwrap()
                    .active_interaction_for_agent(&agent)
                    .map(|i| i.id().to_string())
                {
                    break id;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            !pending.is_finished(),
            "Ask must not grant before the user decides"
        );
        let request =
            LocalDaemonRequest::RespondToInteraction(crate::local::RespondToInteractionRequest {
                session_id: session.clone(),
                interaction_id,
                choice_id: choice.into(),
                custom_reply: None,
                passkey: None,
                passkey_remember_minutes: None,
            });
        let mut command = KernelCommand::from_local_request(
            format!("app-binding-{choice}"),
            None,
            None,
            &request,
        );
        command.caller.user_id = Some("alice".into());
        router.dispatch(command, request).await.unwrap();
        let result = timeout(Duration::from_secs(2), pending)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.ok, choice == "allow");
        let current = app.lock().await.agents().get_agent(&agent).unwrap();
        assert_eq!(
            current.has_extension_grant(ExtensionKind::App, "installed"),
            choice == "allow"
        );
    }
}

async fn granted(app: &Arc<Mutex<DaemonApp>>, agent: &str) -> bool {
    granted_app(app, agent, "installed").await
}

async fn granted_app(app: &Arc<Mutex<DaemonApp>>, agent: &str, installation: &str) -> bool {
    app.lock()
        .await
        .agents()
        .get_agent(agent)
        .unwrap()
        .has_extension_grant(ExtensionKind::App, installation)
}

async fn spawn_alice_agent(app: &Arc<Mutex<DaemonApp>>, session: &str) -> String {
    let mut app = app.lock().await;
    crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(CreateAgentRequest::new(session, "dev-stub").with_owner_user_id("alice"))
        .unwrap()
        .id()
        .to_owned()
}

/// The session's Room, ready for its browser controller's Tabs.
async fn start_room(app: &Arc<Mutex<DaemonApp>>, session: &str) {
    let store = app.lock().await.session_state_store();
    let viewport = crate::session::CanonicalViewport::new(1280, 800, 1, 1280, 800).unwrap();
    store
        .create_room_environment(session, "room", viewport.clone())
        .unwrap();
    store.start_room_environment(session, viewport).unwrap();
    store
        .transition_room_environment(session, crate::session::EnvironmentLifecycle::Ready)
        .unwrap();
}

/// The Room's Tabs as its browser controller reports them: these controller
/// targets are open and `focused` is in front.
fn show_tabs(router: &CommandRouter, session: &str, targets: &[&str], focused: &str) {
    let tabs = targets
        .iter()
        .map(|target| crate::session::EnvironmentTabObservation {
            runtime_target_id: (*target).into(),
            document_id: format!("document-{target}"),
            url: format!("https://{target}.test/"),
            title: (*target).into(),
        })
        .collect();
    router
        .runtime_state
        .reconcile_room_environment_controller_tabs(session, tabs, Some(focused))
        .unwrap();
}

/// An open view of `owner`'s App installation on a controller target.
fn open_view(router: &CommandRouter, session: &str, target: &str, owner: &str, app: &str) {
    router.runtime_state.app_control().views().register(
        session,
        target,
        crate::runtime::app_views::AppViewBinding {
            logical_tab: None,
            owner: owner.into(),
            installation: app.into(),
            generation: 1,
            panel: Default::default(),
        },
    );
}

/// Only these App view Tabs are still open.
fn close_views_except(router: &CommandRouter, session: &str, open: &[&str]) {
    let open = open
        .iter()
        .map(|target| (*target).to_owned())
        .collect::<Vec<_>>();
    router
        .runtime_state
        .app_control()
        .views()
        .retain_open(session, &open, u64::MAX);
}

#[tokio::test]
async fn a_foreground_app_binds_the_focus_agent_follows_focus_and_explicit_revocation_unbinds() {
    let fixture = Fixture::new();
    // Ask mode: a person foregrounding an App is an explicit selection.
    let (app, router, session, first, _auth) =
        fixture.router(crate::provider::AgentPermissionLevel::Required);
    let second = spawn_alice_agent(&app, &session).await;
    let state = &router.runtime_state;
    start_room(&app, &session).await;
    state.focus_agent(&session, &first, "alice").await.unwrap();
    // Only the App's owner foregrounds it for their agents.
    open_view(&router, &session, "bob-view", "bob", "installed");
    show_tabs(&router, &session, &["bob-view"], "bob-view");
    assert_eq!(state.foreground_app(&session, "bob-view").await, None);
    assert!(!granted(&app, &first).await);
    open_view(&router, &session, "todo", "alice", "installed");
    show_tabs(&router, &session, &["bob-view", "todo"], "todo");
    assert_eq!(
        state.foreground_app(&session, "todo").await,
        Some(first.clone())
    );
    assert!(granted(&app, &first).await && !granted(&app, &second).await);
    // The next focus agent gets the App of the focused Tab too.
    state.focus_agent(&session, &second, "alice").await.unwrap();
    assert!(granted(&app, &second).await);
    state.unbind_uninstalled_app("alice", "installed").await;
    assert!(!granted(&app, &first).await && !granted(&app, &second).await);
    // Its Tab stays on screen, unbound: it is no foreground App.
    let third = spawn_alice_agent(&app, &session).await;
    state.focus_agent(&session, &third, "alice").await.unwrap();
    assert!(!granted(&app, &third).await);
}

#[tokio::test]
async fn the_foreground_app_is_the_app_of_the_rooms_focused_tab() {
    let fixture = Fixture::new();
    let (app, router, session, first, _auth) =
        fixture.router(crate::provider::AgentPermissionLevel::Required);
    crate::durable_state::app_state::fixture_installation(
        &app.lock().await.durable_state_store(),
        "alice",
        "docs",
    );
    let second = spawn_alice_agent(&app, &session).await;
    let third = spawn_alice_agent(&app, &session).await;
    let state = &router.runtime_state;
    start_room(&app, &session).await;
    // Todo open and focused: the focus agent gets Todo.
    open_view(&router, &session, "todo", "alice", "installed");
    show_tabs(&router, &session, &["page", "todo"], "todo");
    state.focus_agent(&session, &first, "alice").await.unwrap();
    assert!(granted(&app, &first).await);
    // Two Apps open: opening Documents binds it; its Tab is in front.
    open_view(&router, &session, "docs", "alice", "docs");
    show_tabs(&router, &session, &["page", "todo", "docs"], "docs");
    assert_eq!(
        state.foreground_app(&session, "docs").await,
        Some(first.clone())
    );
    state.focus_agent(&session, &second, "alice").await.unwrap();
    assert!(granted_app(&app, &second, "docs").await && !granted(&app, &second).await);
    // Switching Tabs: the next focus agent gets the App now in front.
    show_tabs(&router, &session, &["page", "todo", "docs"], "todo");
    state.focus_agent(&session, &third, "alice").await.unwrap();
    assert!(granted(&app, &third).await && !granted_app(&app, &third, "docs").await);
    // Documents, the App opened last, closes and the Room shows Todo again:
    // a focus change binds Todo, never Documents.
    close_views_except(&router, &session, &["todo"]);
    show_tabs(&router, &session, &["page", "todo"], "todo");
    let fourth = spawn_alice_agent(&app, &session).await;
    state.focus_agent(&session, &fourth, "alice").await.unwrap();
    assert!(granted(&app, &fourth).await && !granted_app(&app, &fourth, "docs").await);
    // A plain page in front: no foreground App.
    show_tabs(&router, &session, &["page", "todo"], "page");
    let fifth = spawn_alice_agent(&app, &session).await;
    state.focus_agent(&session, &fifth, "alice").await.unwrap();
    assert!(!granted(&app, &fifth).await);
    // The Room still shows an App Tab whose view is gone: nothing is bound.
    close_views_except(&router, &session, &[]);
    show_tabs(&router, &session, &["page", "todo"], "todo");
    let sixth = spawn_alice_agent(&app, &session).await;
    state.focus_agent(&session, &sixth, "alice").await.unwrap();
    assert!(!granted(&app, &sixth).await);
}

#[tokio::test]
async fn a_revoked_foreground_binding_returns_only_when_the_app_is_opened_again() {
    let fixture = Fixture::new();
    let (app, router, session, first, _auth) =
        fixture.router(crate::provider::AgentPermissionLevel::Required);
    let second = spawn_alice_agent(&app, &session).await;
    let state = &router.runtime_state;
    start_room(&app, &session).await;
    open_view(&router, &session, "todo", "alice", "installed");
    show_tabs(&router, &session, &["todo"], "todo");
    state.focus_agent(&session, &first, "alice").await.unwrap();
    assert_eq!(
        state.foreground_app(&session, "todo").await,
        Some(first.clone())
    );
    // Cycling the focus binds the next focus agent too.
    let cycled = state.cycle_agent_focus(&session, "alice").await.unwrap();
    assert_eq!(
        cycled.map(|agent| agent.id().to_owned()),
        Some(second.clone())
    );
    assert!(granted(&app, &second).await);
    // A focus change does not bind a revoked pair again.
    state
        .revoke_agent_extension(&second, ExtensionKind::App, "installed", "alice")
        .await
        .unwrap();
    state.focus_agent(&session, &first, "alice").await.unwrap();
    state.focus_agent(&session, &second, "alice").await.unwrap();
    assert!(!granted(&app, &second).await);
    // Nor after the session's last App view closed and the App was opened
    // again for another agent.
    close_views_except(&router, &session, &[]);
    assert!(!state.app_control().views().keep_pumping(&session));
    state.focus_agent(&session, &first, "alice").await.unwrap();
    open_view(&router, &session, "todo-again", "alice", "installed");
    show_tabs(&router, &session, &["todo-again"], "todo-again");
    assert_eq!(
        state.foreground_app(&session, "todo-again").await,
        Some(first.clone())
    );
    state.focus_agent(&session, &second, "alice").await.unwrap();
    assert!(!granted(&app, &second).await);
    // Opening the App with that agent in focus binds it again.
    assert_eq!(
        state.foreground_app(&session, "todo-again").await,
        Some(second.clone())
    );
    assert!(granted(&app, &second).await);
}

#[tokio::test]
async fn a_persons_spawn_gets_the_foreground_app_and_a_meta_agents_spawn_does_not() {
    let fixture = Fixture::new();
    let (app, router, session, first, auth) =
        fixture.router(crate::provider::AgentPermissionLevel::Yolo);
    let state = &router.runtime_state;
    start_room(&app, &session).await;
    open_view(&router, &session, "todo", "alice", "installed");
    show_tabs(&router, &session, &["todo"], "todo");
    state.focus_agent(&session, &first, "alice").await.unwrap();
    assert_eq!(
        state.foreground_app(&session, "todo").await,
        Some(first.clone())
    );
    let request = LocalDaemonRequest::SpawnAgent(crate::local::SpawnAgentRequest {
        account_profile: None,
        session_id: session.clone(),
        alias: Some("person-worker".into()),
        provider: Some("dev-stub".into()),
        model: None,
        effort: None,
        execution_mode: None,
        permission_level: None,
        worktree_id: None,
        kernel_ref: None,
        slice_ref: None,
        worktree_placement: None,
        metaagent: false,
    });
    let mut command = KernelCommand::from_local_request("person-spawn", None, None, &request);
    command.caller.user_id = Some("alice".into());
    let LocalDaemonResponse::AgentSpawned { agent: spawned } =
        router.dispatch(command, request).await.unwrap()
    else {
        panic!("unexpected spawn response");
    };
    assert!(granted(&app, spawned.id()).await);
    // A Meta agent's new agent keeps the person's focus and does not bind
    // the foreground App.
    app.lock()
        .await
        .agents_mut()
        .activate_agent_meta_mode(&first, None)
        .unwrap();
    let result = router
        .dispatch_authenticated_runtime_tool_call(
            &auth,
            crate::transport::runtime_tools::META_RUN_COMMAND_TOOL,
            serde_json::json!({ "command": "agent spawn meta-worker" }),
        )
        .await
        .unwrap();
    assert!(result.ok, "{result:?}");
    assert_eq!(
        state.focused_agent_id(&session).await.unwrap().as_deref(),
        Some(spawned.id())
    );
    let worker = app
        .lock()
        .await
        .agents()
        .get_session_agents(&session)
        .into_iter()
        .find(|agent| agent.alias() == Some("meta-worker"))
        .expect("the meta agent's worker should exist")
        .id()
        .to_string();
    assert_ne!(worker, spawned.id());
    assert_ne!(worker, first);
    assert!(!granted(&app, &worker).await);
}

#[tokio::test]
async fn a_saturated_listing_omits_a_cold_app_and_refreshes_once_a_slot_frees() {
    let fixture = Fixture::new();
    let (app, router, _session, agent, auth) =
        fixture.router(crate::provider::AgentPermissionLevel::Yolo);
    let state = &router.runtime_state;
    state
        .grant_agent_extension(&agent, ExtensionGrant::app("installed"), "alice")
        .await
        .unwrap();
    let control = state.app_control().clone();
    control.forget_app_dormant("alice", "installed");
    let permits = (0..8)
        .map(|_| control.try_admit().unwrap())
        .collect::<Vec<_>>();
    // Nothing runs or is dormant: the listing answers without the App, and
    // one refresh waits for a slot however often the agent lists.
    for _ in 0..3 {
        state
            .runtime_tool_specs_for_auth_token_async(auth.clone())
            .await
            .expect("a cold App's listing does not fail when admission is busy");
    }
    assert!(control.catalog_refresh_pending(&agent));
    // Listed without its App: it is due a refresh when the App starts.
    assert!(control.app_unlisted("alice", "installed", &agent));
    // The shared projection (a leased agent's manifest) answers the same way.
    let bound = app.lock().await.agents().get_agent(&agent).unwrap();
    assert!(control
        .app_extension_tools_for_agent(&bound, &std::collections::BTreeSet::new())
        .unwrap()
        .is_empty());
    drop(permits);
    timeout(Duration::from_secs(5), async {
        while control.catalog_refresh_pending(&agent) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the refresh runs once a slot frees");
}

#[tokio::test]
async fn a_binding_saved_while_its_app_is_stopped_waits_for_its_start_unless_revoked() {
    let fixture = Fixture::new();
    let (app, router, _session, agent, _auth) =
        fixture.router(crate::provider::AgentPermissionLevel::Yolo);
    let state = &router.runtime_state;
    app.lock()
        .await
        .durable_state_store()
        .stop_app_worker_intent(
            "alice",
            "installed",
            crate::runtime::app_operation_budget::AppOperationBudget::from_supervisor(|| false),
        )
        .unwrap();
    state
        .grant_agent_extension(&agent, ExtensionGrant::app("installed"), "alice")
        .await
        .unwrap();
    let control = state.app_control();
    assert!(control.app_unlisted("alice", "installed", &agent));
    state
        .revoke_agent_extension(&agent, ExtensionKind::App, "installed", "alice")
        .await
        .unwrap();
    assert!(!control.app_unlisted("alice", "installed", &agent));
}

#[tokio::test]
async fn a_fork_copies_app_bindings_through_the_checked_audited_grant() {
    let fixture = Fixture::new();
    let (app, router, session, first, _auth) =
        fixture.router(crate::provider::AgentPermissionLevel::Required);
    crate::durable_state::app_state::fixture_installation(
        &app.lock().await.durable_state_store(),
        "alice",
        "docs",
    );
    let state = &router.runtime_state;
    state
        .grant_agent_extension(&first, ExtensionGrant::app("installed"), "alice")
        .await
        .unwrap();
    // The source still holds a binding whose App is gone.
    app.lock()
        .await
        .agents_mut()
        .grant_extension(&first, ExtensionGrant::app("gone"))
        .unwrap();
    // Documents' Tab is in front: the fork, the new focus agent, gets it.
    start_room(&app, &session).await;
    open_view(&router, &session, "docs", "alice", "docs");
    show_tabs(&router, &session, &["docs"], "docs");
    let (_, fork, _, _) = state
        .fork_agent(
            crate::local::ForkAgentRequest {
                session_id: session.clone(),
                source_agent_ref: Some(first.clone()),
                alias: Some("fork".into()),
            },
            "alice".into(),
        )
        .await
        .unwrap();
    assert!(fork.has_extension_grant(ExtensionKind::App, "installed"));
    assert!(fork.has_extension_grant(ExtensionKind::App, "docs"));
    assert!(!fork.has_extension_grant(ExtensionKind::App, "gone"));
    // Each copied or foreground binding has its grant event and audit.
    let audit = state
        .list_home_extension_audit_events(fork.id(), "alice", 50)
        .unwrap();
    for app in ["installed", "docs"] {
        assert!(audit
            .iter()
            .any(|event| event.kind == "agent.extension_granted"
                && event.payload["capability_name"] == format!("app:{app}")));
        assert!(audit
            .iter()
            .any(|event| event.kind == "home_extension.grant.created"
                && event.payload["grant"]["name"] == app));
    }
    assert!(!audit
        .iter()
        .any(|event| event.payload["grant"]["name"] == "gone"));
}

#[tokio::test]
async fn uninstall_keeps_missing_binding_visible_without_tools_or_a_reinstall_regrant() {
    let fixture = Fixture::new();
    let (app, router, session, agent, _auth) =
        fixture.router(crate::provider::AgentPermissionLevel::Yolo);
    let state = &router.runtime_state;
    state
        .grant_agent_extension(&agent, ExtensionGrant::app("installed"), "alice")
        .await
        .unwrap();
    let store = app.lock().await.durable_state_store();
    let generation = store
        .get_app_installation("alice", "installed")
        .unwrap()
        .generation;
    let request = LocalDaemonRequest::UninstallApp(crate::local::UninstallAppRequest {
        installation_id: "installed".into(),
        expected_generation: generation.to_string(),
        delete_data: false,
    });
    let mut command =
        KernelCommand::from_local_request("missing-binding-uninstall", None, None, &request);
    command.caller.user_id = Some("alice".into());
    assert!(matches!(
        router.dispatch(command, request).await.unwrap(),
        LocalDaemonResponse::AppInstallation { .. }
    ));
    // The existing grant identifies the broken binding to every inspector;
    // the inactive installation gives it no executable tools or version.
    assert!(
        granted(&app, &agent).await,
        "uninstall silently erased the binding"
    );
    assert!(store
        .get_app_installation("alice", "installed")
        .unwrap()
        .active
        .is_none());
    let bound = app.lock().await.agents().get_agent(&agent).unwrap();
    assert!(state
        .app_control()
        .app_extension_tools_for_agent(&bound, &std::collections::BTreeSet::new())
        .unwrap()
        .is_empty());
    // Exercise the actual reinstall request and its approval boundary, rather
    // than invoking the helper directly: skipping the gate must fail this test.
    use base64::Engine;
    use sha2::Digest;
    let (bytes, publisher) =
        crate::durable_state::app_state::fixture_release_package("1.0.1", 0, false);
    let verified = chariox_app_package::verify(
        &bytes,
        &chariox_app_package::VerificationPolicy::new(
            crate::local::LOCAL_DAEMON_PROTOCOL_VERSION,
            vec![publisher],
        ),
    )
    .unwrap();
    let digest = format!("sha256:{:x}", sha2::Sha256::digest(&bytes));
    let dispatch = |request: LocalDaemonRequest, id: &str| {
        let mut command = KernelCommand::from_local_request(id.to_owned(), None, None, &request);
        command.caller.user_id = Some("alice".into());
        async { router.dispatch(command, request).await.unwrap() }
    };
    let upload = dispatch(
        LocalDaemonRequest::BeginAppPackageUpload(crate::local::BeginAppPackageUploadRequest {
            request_id: "missing-binding-upload".into(),
            expected_size: bytes.len() as u64,
            sha256: digest.clone(),
        }),
        "missing-binding-upload",
    )
    .await;
    let LocalDaemonResponse::AppPackageUploadStatus { upload } = upload else {
        panic!("upload did not begin: {upload:?}")
    };
    let result = dispatch(
        LocalDaemonRequest::PutAppPackageUploadChunk(
            crate::local::PutAppPackageUploadChunkRequest {
                handle: upload.handle.clone(),
                offset: 0,
                data_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
                chunk_sha256: digest,
            },
        ),
        "missing-binding-chunk",
    )
    .await;
    assert!(matches!(
        result,
        LocalDaemonResponse::AppPackageUploadStatus { .. }
    ));
    let current = store
        .get_app_installation("alice", "installed")
        .unwrap()
        .generation;
    let update = dispatch(
        LocalDaemonRequest::BeginAppUpdate(crate::local::BeginAppUpdateRequest {
            session_id: session.clone(),
            request_id: "missing-binding-reinstall".into(),
            installation_id: "installed".into(),
            expected_generation: current.to_string(),
            upload_handle: upload.handle,
            expected_package_digest: verified.package_digest().into(),
        }),
        "missing-binding-reinstall",
    )
    .await;
    let LocalDaemonResponse::AppInstallOperationStatus { operation } = update else {
        panic!("reinstall did not begin: {update:?}")
    };
    assert_eq!(
        operation.phase,
        crate::local::AppInstallOperationPhase::Preparing
    );
    assert!(
        !granted(&app, &agent).await,
        "reinstall retained the old grant before approval"
    );
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            state.app_control().installs().pump(state).await;
            let response = dispatch(
                LocalDaemonRequest::GetAppInstallOperation(
                    crate::local::AppInstallOperationRequest {
                        request_id: "missing-binding-reinstall".into(),
                    },
                ),
                "missing-binding-status",
            )
            .await;
            let LocalDaemonResponse::AppInstallOperationStatus { operation } = response else {
                panic!("reinstall status: {response:?}")
            };
            assert!(!granted(&app, &agent).await);
            assert!(store
                .get_app_installation("alice", "installed")
                .unwrap()
                .active
                .is_none());
            if operation.phase == crate::local::AppInstallOperationPhase::AwaitingApproval {
                break;
            }
            assert_eq!(
                operation.phase,
                crate::local::AppInstallOperationPhase::Preparing,
                "{operation:?}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("reinstall reaches approval with no old grant");
    let control = state.app_control().installs().clone();
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || control.shutdown_blocking(handle))
        .await
        .unwrap();
}
