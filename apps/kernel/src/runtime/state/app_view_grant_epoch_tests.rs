//! MP-08/MP-10/MP-11 #992: retain AppView authority through cold start and reply.
use super::*;
use crate::local::UserDomainResource;
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
    view: String,
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
    grant(&state, &agent, &view.view_id);
    ViewFixture {
        state,
        agent,
        binding,
        view: view.view_id,
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
fn grant(state: &KernelRuntimeState, agent: &crate::agent::AgentInstance, view: &str) {
    let host = &state.owned.kernel_browser_host;
    host.set_focus("alice", Some(agent.id()));
    host.load("alice", agent.id()).unwrap();
    let admission = host.admit("alice", agent.id()).unwrap();
    host.claim_resource(
        Some(&admission),
        UserDomainResource::AppView {
            view_id: view.into(),
        },
        false,
    )
    .unwrap();
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp11_app_view_revoke_regrant_during_cold_lease_never_enqueues() {
    let ViewFixture {
        state,
        agent,
        binding,
        view,
        observations,
        _cleanup,
    } = prepare(Mode::ToolEcho);
    let (entered, received) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let released = Mutex::new(released);
    state
        .app_control()
        .lifecycle()
        .fixture_before_publication(Arc::new(move || {
            entered.send(()).unwrap();
            released
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
        }));
    state
        .app_control()
        .lifecycle()
        .start_active_blocking("alice", "installed", tokio::runtime::Handle::current())
        .unwrap();
    tokio::task::spawn_blocking(move || received.recv_timeout(Duration::from_secs(10)))
        .await
        .unwrap()
        .unwrap();
    let mut tracked = state.app_control().views().track_call(&view, &view, None);
    let method = tool_method(&state);
    let call = state.invoke_user_app_view_call(
        "alice",
        &view,
        &binding,
        &method,
        serde_json::json!({"text":"cold"}),
        &mut tracked,
        ViewCaller::Agent(&agent),
    );
    tokio::pin!(call);
    tokio::select! { biased;
        result = &mut call => panic!("call must await cold lease: {result:?}"),
        _ = tokio::time::sleep(Duration::from_millis(100)) => {}
    }
    state
        .owned
        .kernel_browser_host
        .revoke_grants("alice", Some(agent.id()));
    grant(&state, &agent, &view);
    release.send(()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(3), &mut call).await;
    cleanup_worker(&state).await;
    assert!(observations.lock().unwrap().iter().all(|o| o.was_reaped()));
    assert!(
        matches!(result, Ok(Err(ref error)) if error.code == "AGENT_APP_NOT_GRANTED"),
        "old admitted call must remain revoked: {result:?}"
    );
    assert_eq!(
        observations
            .lock()
            .unwrap()
            .iter()
            .map(|o| o.tool_invocations())
            .sum::<u64>(),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp11_app_view_revoke_regrant_cancels_a_pending_worker_reply() {
    let ViewFixture {
        state,
        agent,
        binding,
        view,
        observations,
        _cleanup,
    } = prepare(Mode::ToolStall);
    state
        .app_control()
        .lifecycle()
        .start_active_blocking("alice", "installed", tokio::runtime::Handle::current())
        .unwrap();
    wait_worker(&state).await;
    let dispatch = state.clone();
    let caller = agent.clone();
    let id = view.clone();
    let mut call = tokio::spawn(async move {
        let mut tracked = dispatch.app_control().views().track_call(&id, &id, None);
        let method = tool_method(&dispatch);
        dispatch
            .invoke_user_app_view_call(
                "alice",
                &id,
                &binding,
                &method,
                serde_json::json!({"text":"pending"}),
                &mut tracked,
                ViewCaller::Agent(&caller),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while observations
            .lock()
            .unwrap()
            .iter()
            .map(|o| o.tool_invocations())
            .sum::<u64>()
            == 0
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    state
        .owned
        .kernel_browser_host
        .revoke_grants("alice", Some(agent.id()));
    grant(&state, &agent, &view);
    let result = tokio::time::timeout(Duration::from_secs(2), &mut call).await;
    if result.is_err() {
        call.abort();
        let _ = call.await;
    }
    let cancelled = tokio::time::timeout(Duration::from_secs(2), async {
        while observations
            .lock()
            .unwrap()
            .iter()
            .map(|o| o.tool_cancellations())
            .sum::<u64>()
            == 0
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok();
    cleanup_worker(&state).await;
    assert!(observations.lock().unwrap().iter().all(|o| o.was_reaped()));
    assert!(
        matches!(result, Ok(Ok(Err(ref error))) if error.code == "AGENT_APP_NOT_GRANTED"),
        "pending reply must wake on grant revocation: {result:?}"
    );
    assert!(cancelled, "the worker receives Cancel");
}

/// MP-08/MP-11: SQLite queue checks retain the document and the exact grant
/// without taking its mutex recursively while an enqueue commit holds it.
#[tokio::test]
async fn mp11_app_view_budget_keeps_document_and_grant_cancellation() {
    for close_document in [false, true] {
        let ViewFixture {
            state,
            agent,
            view,
            _cleanup,
            ..
        } = prepare(Mode::ToolEcho);
        let resource = UserDomainResource::AppView {
            view_id: view.clone(),
        };
        let grant_epoch = state
            .owned
            .kernel_browser_host
            .retain_resource_grant("alice", agent.id(), resource)
            .unwrap();
        let mut tracked = state.app_control().views().track_call(&view, &view, None);
        tracked.retain_browser_grant(grant_epoch.clone());
        let budget = tracked.operation_budget();
        grant_epoch.commit(|| budget.check()).unwrap().unwrap();
        if close_document {
            state.app_control().views().forget_session(&view);
        } else {
            state
                .owned
                .kernel_browser_host
                .revoke_grants("alice", Some(agent.id()));
            grant(&state, &agent, &view);
            assert!(
                grant_epoch.commit(|| ()).is_err(),
                "regrant cannot revive the old epoch"
            );
        }
        assert_eq!(
            budget.check(),
            Err(crate::runtime::app_operation_budget::AppOperationStopped::Cancelled)
        );
    }
}

// MP-08/MP-10/MP-11 review #1001 P1: installation revocation is independent of the retained browser grant.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp11_app_installation_revoke_regrant_during_cold_lease_never_enqueues() {
    let ViewFixture {
        state,
        agent,
        binding,
        view,
        observations,
        _cleanup,
    } = prepare(Mode::ToolEcho);
    let (entered, received) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let released = Mutex::new(released);
    state
        .app_control()
        .lifecycle()
        .fixture_before_publication(Arc::new(move || {
            entered.send(()).unwrap();
            released
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
        }));
    state
        .app_control()
        .lifecycle()
        .start_active_blocking("alice", "installed", tokio::runtime::Handle::current())
        .unwrap();
    tokio::task::spawn_blocking(move || received.recv_timeout(Duration::from_secs(10)))
        .await
        .unwrap()
        .unwrap();
    let mut tracked = state.app_control().views().track_call(&view, &view, None);
    let method = tool_method(&state);
    let call = state.invoke_user_app_view_call(
        "alice",
        &view,
        &binding,
        &method,
        serde_json::json!({"text":"cold"}),
        &mut tracked,
        ViewCaller::Agent(&agent),
    );
    tokio::pin!(call);
    tokio::select! { biased;
        result = &mut call => panic!("call must await cold lease: {result:?}"),
        _ = tokio::time::sleep(Duration::from_millis(100)) => {}
    }
    state
        .revoke_agent_extension(
            agent.id(),
            crate::extension::ExtensionKind::App,
            "installed",
            "alice",
        )
        .await
        .unwrap();
    state
        .grant_agent_extension(
            agent.id(),
            crate::extension::ExtensionGrant::app("installed"),
            "alice",
        )
        .await
        .unwrap();
    release.send(()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(3), &mut call).await;
    cleanup_worker(&state).await;
    assert!(observations.lock().unwrap().iter().all(|o| o.was_reaped()));
    assert!(
        matches!(result, Ok(Err(ref error)) if error.code == "AGENT_APP_NOT_GRANTED"),
        "old admitted call must remain revoked: {result:?}"
    );
    assert_eq!(
        observations
            .lock()
            .unwrap()
            .iter()
            .map(|o| o.tool_invocations())
            .sum::<u64>(),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mp11_app_installation_revoke_regrant_cancels_a_pending_worker_reply() {
    let ViewFixture {
        state,
        agent,
        binding,
        view,
        observations,
        _cleanup,
    } = prepare(Mode::ToolStall);
    state
        .app_control()
        .lifecycle()
        .start_active_blocking("alice", "installed", tokio::runtime::Handle::current())
        .unwrap();
    wait_worker(&state).await;
    let dispatch = state.clone();
    let caller = agent.clone();
    let id = view.clone();
    let mut call = tokio::spawn(async move {
        let mut tracked = dispatch.app_control().views().track_call(&id, &id, None);
        let method = tool_method(&dispatch);
        dispatch
            .invoke_user_app_view_call(
                "alice",
                &id,
                &binding,
                &method,
                serde_json::json!({"text":"pending"}),
                &mut tracked,
                ViewCaller::Agent(&caller),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while observations
            .lock()
            .unwrap()
            .iter()
            .map(|o| o.tool_invocations())
            .sum::<u64>()
            == 0
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    state
        .revoke_agent_extension(
            agent.id(),
            crate::extension::ExtensionKind::App,
            "installed",
            "alice",
        )
        .await
        .unwrap();
    state
        .grant_agent_extension(
            agent.id(),
            crate::extension::ExtensionGrant::app("installed"),
            "alice",
        )
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), &mut call).await;
    if result.is_err() {
        call.abort();
        let _ = call.await;
    }
    let cancelled = tokio::time::timeout(Duration::from_secs(2), async {
        while observations
            .lock()
            .unwrap()
            .iter()
            .map(|o| o.tool_cancellations())
            .sum::<u64>()
            == 0
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_ok();
    cleanup_worker(&state).await;
    assert!(observations.lock().unwrap().iter().all(|o| o.was_reaped()));
    assert!(
        matches!(result, Ok(Ok(Err(ref error))) if error.code == "AGENT_APP_NOT_GRANTED"),
        "pending reply must wake on grant revocation: {result:?}"
    );
    assert!(cancelled, "the worker receives Cancel");
}

#[path = "app_host_concurrency_tests.rs"]
mod host_concurrency_tests;

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
