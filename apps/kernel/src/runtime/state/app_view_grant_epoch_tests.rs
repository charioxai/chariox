//! MP-08/MP-10/MP-11 #992: a Room App view call keeps the calling agent's identity and binding.
use super::*;
use chariox_app_package::{verify, VerificationPolicy};
use chariox_app_runtime::{
    release_store::{ReleaseStore, StageBudget},
    worker_process::test_fixture::Mode,
};
use std::sync::{Arc, Mutex};

struct ViewFixture {
    state: KernelRuntimeState,
    agent: crate::agent::AgentInstance,
    binding: AppViewBinding,
    observations: Arc<Mutex<Vec<chariox_app_runtime::worker_process::test_fixture::Observation>>>,
    _cleanup: super::cold_start_tests::Cleanup,
}
fn prepare(mode: Mode) -> ViewFixture {
    prepare_with_turn(mode, false)
}
fn prepare_with_turn(mode: Mode, with_turn: bool) -> ViewFixture {
    let (state, session, cleanup) =
        super::cold_start_tests::fixture_for_owner_with_turn("alice", with_turn);
    let store = state.owned.durable_state_store.clone();
    let catalog = crate::durable_state::app_state::fixture_tool_catalog(&store);
    let (bytes, publisher) = crate::durable_state::app_state::fixture_tool_package();
    let package = verify(
        &bytes,
        &VerificationPolicy::new(crate::local::LOCAL_DAEMON_PROTOCOL_VERSION, vec![publisher]),
    )
    .unwrap();
    ReleaseStore::open_or_create(store.path())
        .unwrap()
        .stage(
            &package,
            &bytes,
            StageBudget {
                max_stage_bytes: 1024 * 1024,
                reserved_bytes: 1024 * 1024,
                host_reserve_bytes: 1024 * 1024,
            },
        )
        .unwrap();
    let observations = state
        .app_control()
        .lifecycle()
        .fixture_inbox_workers_with_mode(mode);
    let agent = state
        .owned
        .agent_store
        .list_agents()
        .into_iter()
        .find(|a| a.session_id() == session)
        .unwrap();
    let agent = state
        .owned
        .agent_store
        .grant_extension(
            agent.id(),
            crate::extension::ExtensionGrant::app("installed"),
        )
        .unwrap();
    let binding = AppViewBinding {
        owner: "alice".into(),
        installation: "installed".into(),
        generation: catalog.generation(),
        panel: Default::default(),
        logical_tab: None,
    };
    let view = state
        .app_control()
        .user_views()
        .open("alice", binding.clone(), "a")
        .unwrap();
    state
        .app_control()
        .views()
        .register(&view.view_id, &view.view_id, binding.clone());
    ViewFixture {
        state,
        agent,
        binding,
        observations,
        _cleanup: cleanup,
    }
}
fn tool_method(state: &KernelRuntimeState) -> String {
    state
        .owned
        .durable_state_store
        .active_app_event_catalog("alice", "installed")
        .unwrap()
        .app_catalog()
        .tools()
        .next()
        .unwrap()
        .local_name
        .clone()
}
async fn wait_worker(state: &KernelRuntimeState) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while state
            .app_control()
            .active_app_lease("alice", "installed")
            .is_none()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
async fn cleanup_worker(state: &KernelRuntimeState) {
    let lifecycle = state.app_control().lifecycle().clone();
    tokio::task::spawn_blocking(move || lifecycle.shutdown_blocking())
        .await
        .unwrap()
        .unwrap();
}

// MP-08/MP-10/MP-11: exercise the Room drain, not the user-domain caller seam.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn room_app_view_backend_preserves_actor_and_revocation() {
    let ViewFixture {
        state,
        agent,
        binding,
        observations,
        _cleanup,
        ..
    } = prepare(Mode::ToolEcho);
    let session = agent.session_id().to_owned();
    state
        .app_control()
        .lifecycle()
        .start_active_blocking("alice", "installed", tokio::runtime::Handle::current())
        .unwrap();
    wait_worker(&state).await;
    let target = "room-actor-view";
    state
        .app_control()
        .views()
        .register(&session, target, binding);
    let method = tool_method(&state);
    let call = |actor| {
        serde_json::from_value::<BrowserAppViewCall>(serde_json::json!({
        "installation_id": "installed", "target_id": target, "document_id": "document-1",
        "call_id": "call-1", "method": method, "params": {"text": "actor receipt"}, "actor": actor,
    })).unwrap()
    };
    let actor = serde_json::json!({"kind":"agent", "id":agent.id()});
    let tracked =
        state
            .app_control()
            .views()
            .track_call(&session, target, Some("document-1".into()));
    state
        .clone()
        .answer_app_view_call(session.clone(), call(actor.clone()), tracked)
        .await;
    let requests: Vec<_> = observations
        .lock()
        .unwrap()
        .iter()
        .flat_map(|o| o.tool_requests().unwrap())
        .collect();
    cleanup_worker(&state).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["context"]["actor"], actor);
    assert_eq!(requests[0]["context"]["agent_id"], agent.id());
    assert_eq!(requests[0]["context"]["room_id"], session);

    state
        .revoke_agent_extension(
            agent.id(),
            crate::extension::ExtensionKind::App,
            "installed",
            "alice",
        )
        .await
        .unwrap();
    for rejected in [
        actor,
        serde_json::json!({"kind":"agent", "id":"missing-agent"}),
        serde_json::json!({"kind":"human", "id":"alice"}),
    ] {
        let tracked =
            state
                .app_control()
                .views()
                .track_call(&session, target, Some("document-1".into()));
        state
            .clone()
            .answer_app_view_call(session.clone(), call(rejected), tracked)
            .await;
    }
    assert_eq!(
        observations
            .lock()
            .unwrap()
            .iter()
            .map(|o| o.tool_invocations())
            .sum::<u64>(),
        1,
        "revoked or invalid actors never reach the backend"
    );
    cleanup_worker(&state).await;
}

#[path = "app_view_actor_roundtrip_tests.rs"]
mod actor_roundtrip_tests;
