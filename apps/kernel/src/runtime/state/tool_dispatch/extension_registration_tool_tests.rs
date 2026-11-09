//! MP-08 / MP-09 / MP-10 / MP-11: whether a registration asks the person follows the level the
//! agent's parent chain allows, not the level stored on the agent.
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::provider::{AgentExecutionMode as Mode, AgentPermissionLevel as Level};
use crate::runtime::state::KernelRuntimeState;

/// A room where `child` keeps its own Yolo/Build setting under `parent`, which the person
/// lowered to Required/Plan after the spawn.
async fn lowered_parent() -> (
    KernelRuntimeState,
    crate::session::RuntimeSession,
    crate::agent::AgentInstance,
) {
    let mut app =
        crate::app::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, _) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "workspace-registration-limit",
            std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        ))
        .unwrap();
    let room = session.id().to_string();
    let parent = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(crate::agent::CreateAgentRequest::new(&room, "dev-stub").with_alias("parent"))
        .unwrap()
        .id()
        .to_string();
    let limit = |app: &crate::app::DaemonApp, agent: &str, mode, level| {
        app.agents()
            .update_agent_config(agent, Some(Some(mode)), Some(Some(level)), None, None)
            .unwrap();
    };
    limit(&app, &parent, Mode::Build, Level::Yolo);
    crate::test_support::admit_room_test_turn(&mut app, &room, &parent);
    let child = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(&room, "dev-stub")
                .with_alias("child")
                .with_parent_agent_id(&parent),
        )
        .unwrap()
        .id()
        .to_string();
    limit(&app, &child, Mode::Build, Level::Yolo);
    limit(&app, &parent, Mode::Plan, Level::Required);
    let session = app.sessions().get_session(&room).unwrap();
    let child = app.agents().get_agent(&child).unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime =
        crate::runtime::state::provider_launch_owned_state::owned_runtime_state(&app).await;
    (runtime, session, child)
}

#[tokio::test]
async fn registration_by_a_child_under_a_lowered_parent_asks_the_person() {
    let (runtime, session, child) = lowered_parent().await;

    let registration = tokio::time::timeout(
        Duration::from_millis(500),
        runtime.maybe_gate_extension_registration(&session, &child, "mcp", Some("child-mcp"), None),
    )
    .await;

    assert!(
        registration.is_err(),
        "a child limited to Required registered a global extension without asking: {registration:?}"
    );
}

#[tokio::test]
async fn the_room_agent_list_shows_a_child_at_its_parent_chain_limit() {
    let (runtime, session, child) = lowered_parent().await;

    let listed = runtime.handle_list_session_agents_runtime_tool(&session, &child);

    let entry = listed.payload["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["id"] == child.id())
        .unwrap()
        .clone();
    assert_eq!(
        (
            entry["permission_level"].clone(),
            entry["execution_mode"].clone()
        ),
        (
            serde_json::json!(Level::Required.as_str()),
            serde_json::json!(Mode::Plan.as_str())
        ),
        "the room agent list shows the child's own setting, not what it runs under"
    );
}
