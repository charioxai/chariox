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
