//! MP-08/MP-10/MP-11: local vertical path on the ordinary kernel.
//! The fixed native ABI App exercises real IPC/broker calls, not embedded Node
//! or production sandbox admission. No provider is dispatched by this harness.
use super::*;
use crate::{
    durable_state::{
        app_inbox::{AppInboxOperation, AppInboxOutcome},
        app_state::fixture_inbox_installation,
    },
    local::{
        AppInboxConnection, CreateAppInboxRouteRequest, LocalDaemonRequest, LocalDaemonResponse,
    },
    runtime::{
        app_operation_budget::AppOperationBudget, command::KernelCommand, router::CommandRouter,
    },
    transport::event_delivery_client::{run_event_delivery_connector, EventDeliveryClientConfig},
};
use chariox_app_package::{verify, VerificationPolicy};
use chariox_app_runtime::{
    app_outbox::{occurrence_id, Invocation, Occurrence},
    release_store::{ReleaseStore, StageBudget},
};
use chariox_event_protocol::EventDeliveryEnvelope;
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};
use tokio::sync::{watch, Mutex};
mod services;
use services::{Services, GENERATOR};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        fn writable(path: &std::path::Path) {
            use std::os::unix::fs::PermissionsExt;
            if std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
                for child in std::fs::read_dir(path).unwrap() {
                    writable(&child.unwrap().path());
                }
            }
        }
        writable(&self.0);
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn budget() -> AppOperationBudget {
    AppOperationBudget::from_supervisor(|| false)
}
async fn route(
    router: &CommandRouter,
    id: &str,
    filter: Value,
) -> Result<LocalDaemonResponse, crate::error::DaemonError> {
    let request = LocalDaemonRequest::CreateAppInboxRoute(CreateAppInboxRouteRequest {
        installation_id: "installed".into(),
        route_id: id.into(),
        event_name: "received".into(),
        source_event_type: "dummy.test".into(),
        source_event_version: 1,
        connection: Some(AppInboxConnection {
            generator_id: GENERATOR.into(),
            connection_id: "fixture-connection".into(),
            connection_scope: "default".into(),
            filter,
        }),
    });
    router
        .dispatch(
            KernelCommand::from_local_request(
                format!("route-{id}-{}", rand::random::<u64>()),
                None,
                None,
                &request,
            ),
            request,
        )
        .await
}
struct WorkerCleanup(crate::runtime::app_lifecycle::AppLifecycleService);
impl Drop for WorkerCleanup {
    fn drop(&mut self) {
        let _ = self.0.shutdown_blocking();
    }
}
struct Connector {
    stop: watch::Sender<bool>,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Connector {
    async fn shutdown(mut self) {
        self.stop.send(true).unwrap();
        self.task.take().unwrap().await.unwrap();
    }
}
impl Drop for Connector {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
fn connector(state: &KernelRuntimeState, config: &crate::config::DaemonConfig) -> Connector {
    let (stop, stopped) = watch::channel(false);
    let task = tokio::spawn(run_event_delivery_connector(
        state.clone(),
        EventDeliveryClientConfig {
            url: config.event_delivery_url.clone(),
            token: None,
            kernel_id: config.daemon_id.clone(),
            environment_id: config.event_delivery_environment_id.clone(),
            generator_management_targets: config.event_generator_management_targets.clone(),
            config_projection: state.owned.config_projection.clone(),
        },
        stopped,
    ));
    Connector {
        stop,
        task: Some(task),
    }
}
fn configure(state: &KernelRuntimeState, session: &str, publication: &str, automation: &str) {
    let catalog = state
        .owned
        .durable_state_store
        .active_app_event_catalog("local", "installed")
        .unwrap();
    state
        .owned
        .configure_app_automation(
            "local",
            catalog,
            super::app_automation_owned_state::ConfigureAppAutomation {
                automation_id: automation.into(),
                expected_revision: 0,
                event_name: "changed".into(),
                session_id: session.into(),
                publication_ref: publication.into(),
                queue_ref: None,
                scheduled: false,
                delivery_mode: crate::local::NotificationDeliveryMode::Queue,
            },
            budget(),
        )
        .unwrap();
}
fn workflow(app: &mut DaemonApp, root: &std::path::Path, label: &str) -> (String, String) {
    let path = root.to_str().unwrap();
    let (session, _) = crate::app::KernelSessionService::new(app)
        .create_session(crate::session::CreateSessionRequest::new(path, path))
        .unwrap();
    let agent = crate::app::KernelSessionService::new(app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub").with_alias(label),
        )
        .unwrap();
    let workflow = app
        .sessions_mut()
        .create_workflow(session.id(), Some(label.into()))
        .unwrap();
    let node = app
        .sessions_mut()
        .add_workflow_node(session.id(), workflow.id(), agent.id())
        .unwrap();
    let endpoint = app
        .sessions_mut()
        .create_workflow_endpoint(session.id(), workflow.id(), node.id(), Some("entry".into()))
        .unwrap();
    let publication = app
        .sessions_mut()
        .create_workflow_publication_idempotent(
            session.id(),
            workflow.id(),
            endpoint.id(),
            None,
            None,
            Some("default".into()),
            Some(label.into()),
            Some("event_based".into()),
            None,
            vec![],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            vec![agent],
            "local".into(),
        )
        .unwrap();
    app.sessions_mut()
        .ensure_primary_workflow_runtime_instance(session.id())
        .unwrap();
    app.durable_state_store()
        .persist_workflow_runtime_transition(
            &app.sessions().get_session(session.id()).unwrap(),
            "evval_fixture",
        )
        .unwrap();
    (session.id().into(), publication.id().into())
}
fn counts(state: &KernelRuntimeState) -> (u64, u64, u64) {
    let AppInboxOutcome::Routes(routes) = state
        .owned
        .durable_state_store
        .app_inbox(AppInboxOperation::Routes {
            owner: "local".into(),
            installation: "installed".into(),
        })
        .unwrap()
    else {
        panic!("routes")
    };
    routes.iter().fold((0, 0, 0), |v, r| {
        (v.0 + r.1.pending, v.1 + r.1.delivered, v.2 + r.1.expired)
    })
}
fn runs(state: &KernelRuntimeState, sessions: &[(String, String)]) -> Vec<String> {
    sessions
        .iter()
        .flat_map(|(id, _)| {
            state
                .owned
                .session_store
                .get_session(id)
                .unwrap()
                .workflow_runs()
                .iter()
                .map(|r| r.id().to_owned())
                .collect::<Vec<_>>()
        })
        .collect()
}
fn envelope(binding: &str, suffix: &str, now: u64) -> EventDeliveryEnvelope {
    let emission = |automation: &str| {
        serde_json::to_value(Occurrence {
            automation_id: automation.into(),
            occurrence_id: occurrence_id(suffix, now).unwrap(),
            event_version: 1,
            occurred_at_ms: now,
            schedule_revision: None,
            payload: json!({"text":"fixture"}),
            invocation: Invocation {
                prompt: format!("review {suffix}"),
                artifacts: vec![],
            },
        })
        .unwrap()
    };
    EventDeliveryEnvelope {
        delivery_id: format!("delivery-{binding}-{suffix}"),
        binding_id: binding.into(),
        event_type: "dummy.test".into(),
        event_type_version: 1,
        occurrence_id: format!("upstream-{suffix}"),
        occurred_at: chrono::DateTime::from_timestamp_millis(now as i64)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        prompt: "canonical fixture prompt".into(),
        artifacts: vec![],
        metadata: json!({"event":{"kind":"wanted"},"emissionFirst":emission("first"),"emissionSecond":emission("second")}),
        reply_context: Some(json!({"thread":"safe-fixture"})),
        expires_at_ms: now + 60_000,
    }
}
async fn drain(state: &KernelRuntimeState, expected: u64, sessions: &[(String, String)]) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while counts(state).0 > 0 {
        state.app_inbox_pass(crate::session::unix_epoch_ms()).await;
        assert!(
            tokio::time::Instant::now() < deadline,
            "App inbox must drain"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(counts(state), (0, expected, 0));
    for _ in 0..expected {
        let owned = state.clone();
        tokio::task::spawn_blocking(move || owned.fixture_app_event_pass())
            .await
            .unwrap();
    }
    assert_eq!(runs(state, sessions).len(), expected as usize);
}

#[test]
fn event_path_delivery_filter_restart_ack_loss_and_ttl() {
    exercise_on_runtime(false);
}

#[test]
fn event_path_fanout_preserves_binding_and_automation_independence() {
    exercise_on_runtime(true);
}

fn exercise_on_runtime(fanout: bool) {
    // MP-10: whole-kernel drills use the production runtime's stack budget.
    let stack = crate::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE;
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(stack)
                .enable_all()
                .build()
                .unwrap()
                .block_on(Box::pin(exercise(fanout)));
        })
        .unwrap()
        .join()
        .unwrap();
}

async fn exercise(fanout: bool) {
    let root = std::env::temp_dir().join(format!("chariox-evval-{:032x}", rand::random::<u128>()));
    std::fs::create_dir(&root).unwrap();
    let scratch = Scratch(root.clone());
    let services = Services::new(&root).await;
    let mut config = crate::config::DaemonConfig::for_tests();
    std::fs::create_dir(root.join("state")).unwrap();
    std::fs::create_dir_all(root.join("workspace/first")).unwrap();
    std::fs::create_dir_all(root.join("workspace/second")).unwrap();
    config.user_config.state.path = Some(root.join("state/state.db").display().to_string());
    config.user_config_path = root.join("state/config.toml");
    config = config.with_session_history_root(root.join("session-history"));
    config.user_config.history.operational.path =
        Some(root.join("operational.db").display().to_string());
    config.user_config.artifacts.operational.root =
        Some(root.join("artifacts").display().to_string());
    config.user_config.artifacts.operational.index_path =
        Some(root.join("artifacts.db").display().to_string());
    config.event_delivery_url = Some(services.aeds_url.clone());
    config.event_delivery_environment_id = "evval-fixture".into();
    config.event_generator_management_targets.insert(
        GENERATOR.into(),
        crate::config::EventGeneratorManagementTarget {
            url: services.generator_url.clone(),
            token: "fixture-only".into(),
            expires_at_ms: None,
            owner_ids: None,
            owner_scoped: None,
        },
    );
    let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
    let control = app.app_control_service();
    let cleanup = WorkerCleanup(control.lifecycle().clone());
    let observations = control.lifecycle().fixture_inbox_automation_workers();
    let store = app.durable_state_store();
    let (bytes, publisher) = fixture_inbox_installation(&store, "local");
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
    let sessions = vec![
        workflow(&mut app, &root.join("workspace/first"), "first"),
        workflow(&mut app, &root.join("workspace/second"), "second"),
    ];
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let state = router.runtime_state();
    configure(&state, &sessions[0].0, &sessions[0].1, "first");
    configure(&state, &sessions[1].0, &sessions[1].1, "second");
    assert!(matches!(
        route(&router, "first", json!({"event.kind":"wanted"}))
            .await
            .unwrap(),
        LocalDaemonResponse::AppInboxRoutes { .. }
    ));
    assert!(
        matches!(
            route(&router, "first", json!({"event.kind":"wanted"}))
                .await
                .unwrap(),
            LocalDaemonResponse::AppRequestFailed {
                code: crate::local::AppRequestErrorCode::Conflict
            }
        ),
        "one route identity remains exclusive"
    );
    if fanout {
        assert!(matches!(
            route(&router, "second", json!({"event.kind":"wanted"}))
                .await
                .unwrap(),
            LocalDaemonResponse::AppInboxRoutes { .. }
        ));
    }
    let resumes = state
        .event_delivery_resumes(&config.daemon_id, &config.event_delivery_environment_id)
        .unwrap();
    let claims = &resumes[0].routes;
    assert_eq!(claims.len(), if fanout { 2 } else { 1 });
    // Both bindings and both automations see the same source occurrence and
    // outgoing occurrence identity; deduplication must remain independent.
    let occurred_at_ms = crate::session::unix_epoch_ms();
    let deliveries: Vec<_> = claims
        .iter()
        .map(|r| envelope(&r.binding_id, "first", occurred_at_ms))
        .collect();
    // Hold the production wake reservation while checking acceptance/ACK. The
    // explicit bounded inbox pass below is the only handler driver in this drill.
    let held_wake = state
        .app_control()
        .wake_pump()
        .try_begin(crate::session::unix_epoch_ms())
        .unwrap();
    let connection = connector(&state, &config);
    assert_eq!(
        services
            .generate(
                deliveries.clone(),
                &json!({"event":{"kind":"other"}}),
                false
            )
            .await,
        0
    );
    assert_eq!(counts(&state), (0, 0, 0));
    assert!(runs(&state, &sessions).is_empty());
    assert_eq!(
        services
            .generate(
                deliveries.clone(),
                &json!({"event":{"kind":"wanted"}}),
                true
            )
            .await,
        claims.len()
    );
    // Accepted before worker delivery; both declared automations exist, only
    // the route's handler-selected automation should be emitted.
    assert_eq!(counts(&state), (claims.len() as u64, 0, 0));
    assert!(runs(&state, &sessions).is_empty());
    assert!(services
        .publications(&json!({"event":{"kind":"other"}}))
        .is_empty());
    assert_eq!(
        services
            .publications(&json!({"event":{"kind":"wanted"}}))
            .len(),
        1,
        "one source publication, independent AEDS deliveries"
    );
    drain(&state, claims.len() as u64, &sessions).await;
    let original_runs = runs(&state, &sessions);
    for (session, _) in &sessions {
        for run in state
            .owned
            .session_store
            .get_session(session)
            .unwrap()
            .workflow_runs()
        {
            assert_eq!(run.invocation_prompt(), Some("review first"));
        }
    }
    assert_eq!(runs(&state, &sessions[..1]).len(), 1);
    assert_eq!(runs(&state, &sessions[1..]).len(), usize::from(fanout));
    let frames: Vec<_> = observations
        .lock()
        .unwrap()
        .iter()
        .flat_map(|o| o.lifecycle_frames().unwrap())
        .filter(|f| f["method"] == "events.deliver")
        .collect();
    assert_eq!(frames.len(), claims.len());
    for frame in frames {
        let payload = &frame["params"]["payload"];
        assert_eq!(payload["source"]["generator_id"], GENERATOR);
        assert_eq!(payload["source"]["connection_id"], "fixture-connection");
        assert_eq!(payload["metadata"]["event"]["kind"], "wanted");
        assert_eq!(payload["reply_context"]["thread"], "safe-fixture");
        assert_eq!(payload["text"], "canonical fixture prompt");
    }

    connection.shutdown().await;
    drop(held_wake);
    control.lifecycle().shutdown_blocking().unwrap();
    drop(state);
    drop(router);
    drop(cleanup);
    drop(control);
    drop(store);
    // Reopen the whole kernel (SQLite writer + sessions), not an in-memory set.
    let app = DaemonApp::bootstrap(config.clone()).unwrap();
    let control = app.app_control_service();
    let cleanup = WorkerCleanup(control.lifecycle().clone());
    let restarted = control.lifecycle().fixture_inbox_automation_workers();
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let state = router.runtime_state();
    // Hold the production wake reservation while checking acceptance/ACK. The
    // explicit bounded inbox pass below is the only handler driver in this drill.
    let held_wake = state
        .app_control()
        .wake_pump()
        .try_begin(crate::session::unix_epoch_ms())
        .unwrap();
    let connection = connector(&state, &config);
    assert_eq!(
        services.deliver(deliveries.clone(), false).await,
        deliveries.len()
    );
    assert_eq!(counts(&state), (0, claims.len() as u64, 0));
    assert_eq!(runs(&state, &sessions), original_runs);
    // Different network delivery ID, same source occurrence, still deduplicates.
    let mut replay = deliveries[0].clone();
    replay.delivery_id = "delivery-new-attempt".into();
    assert_eq!(services.deliver(vec![replay], false).await, 1);
    let mut expired = envelope(
        &claims[0].binding_id,
        "expired",
        crate::session::unix_epoch_ms(),
    );
    expired.expires_at_ms = crate::session::unix_epoch_ms() - 1;
    assert_eq!(services.deliver(vec![expired], false).await, 0);
    assert_eq!(counts(&state), (0, claims.len() as u64, 0));
    assert_eq!(runs(&state, &sessions), original_runs);
    // Transport TTL and durable inbox retention are separate contracts. An
    // accepted but undelivered inbox occurrence expires at the inbox's own TTL.
    let pending = envelope(
        &claims[0].binding_id,
        "inbox-expiry",
        crate::session::unix_epoch_ms(),
    );
    assert_eq!(services.deliver(vec![pending.clone()], false).await, 1);
    assert_eq!(counts(&state), (1, claims.len() as u64, 0));
    let AppInboxOutcome::Due(due) = state
        .owned
        .durable_state_store
        .app_inbox(AppInboxOperation::Due {
            now_ms: crate::session::unix_epoch_ms()
                + chariox_app_runtime::app_inbox::PENDING_LIFETIME_MS
                + 1,
            limit: 8,
        })
        .unwrap()
    else {
        panic!("expiry sweep")
    };
    assert!(due.is_empty());
    assert_eq!(counts(&state), (0, claims.len() as u64, 1));
    assert_eq!(
        services.deliver(vec![pending], false).await,
        1,
        "expired receipt deduplicates replay"
    );
    assert_eq!(counts(&state), (0, claims.len() as u64, 1));
    assert_eq!(runs(&state, &sessions), original_runs);
    connection.shutdown().await;
    drop(held_wake);
    control.lifecycle().shutdown_blocking().unwrap();
    drop(state);
    drop(router);
    drop(cleanup);
    drop(control);
    services.stop().await;
    assert!(observations
        .lock()
        .unwrap()
        .iter()
        .chain(restarted.lock().unwrap().iter())
        .all(|o| o.was_reaped() && o.lease_was_dropped()));
    drop(observations);
    drop(restarted);
    drop(scratch);
    assert!(!root.exists(), "all owned state removed");
}
