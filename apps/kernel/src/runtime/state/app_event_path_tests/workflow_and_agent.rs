//! MP-08 / MP-10 / MP-11: one delivery reaching a workflow endpoint and an agent.
use super::*;

#[test]
fn event_path_one_delivery_runs_a_workflow_and_an_agent_once() {
    on_runtime(exercise);
}

/// The Room's own agent with a provider run and an active prompt, so that an
/// admitted notification queues behind it, as for a working agent.
fn busy_agent(app: &mut DaemonApp, session: &str) -> String {
    use crate::{
        app::KernelSessionService,
        attachment::{AttachRequest, ClientCapabilityLevel},
        provider::LaunchProviderRequest,
        session::{PromptQueueItem, PromptStatus, PromptSubmissionOutcome},
    };
    let agent = app.agents().get_session_agents(session)[0].id().to_owned();
    let attachment = KernelSessionService::new(app)
        .attach(AttachRequest::new(
            session,
            "attachment-github-events",
            ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let run = app
        .launch_provider(
            LaunchProviderRequest::new(session, "dev-stub", "claude-code", "default", "sonnet")
                .with_agent_id(&agent),
        )
        .unwrap();
    app.update_provider_run_projection(run);
    let prompt = PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        &agent,
        "active prompt",
        PromptStatus::Queued,
    );
    assert!(matches!(
        app.prompt_owner_submit_prepared_prompt(session, prompt, false)
            .unwrap(),
        PromptSubmissionOutcome::Started { .. }
    ));
    agent
}
fn agent_fired(state: &KernelRuntimeState, session: &str) -> u64 {
    state
        .owned
        .session_store
        .get_session(session)
        .unwrap()
        .agent_triggers()
        .iter()
        .map(|trigger| trigger.fired())
        .sum()
}
async fn route_agent(state: &KernelRuntimeState) {
    let catalog = state
        .owned
        .durable_state_store
        .active_app_event_catalog("local", "installed")
        .unwrap();
    state
        .route_app_agent_installation("local", catalog)
        .await
        .unwrap();
}

/// MP-08 / MP-10 / MP-11: the GitHub events App's shape, with the fixture
/// generator standing in for `dev.chariox.github` (the connection scope is the
/// repository there). One delivery on one route reaches the handler once; the
/// handler emits to a workflow endpoint automation and to an agent automation
/// by name. A route for another scope gets nothing, and a restart plus replays
/// run neither target a second time.
async fn exercise() {
    use crate::{
        local::{AgentTriggerSourceRequest, CreateAgentTriggerRequest},
        session::AgentTriggerNotificationSource,
    };
    const REPOSITORY: &str = "charioxai/chariox";
    const OTHER_REPOSITORY: &str = "charioxai/other";
    let root = std::env::temp_dir().join(format!("chariox-evval-{:032x}", rand::random::<u128>()));
    std::fs::create_dir(&root).unwrap();
    let scratch = Scratch(root.clone());
    let services = Services::new(&root).await;
    let config = test_config(&root, &services);
    let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
    let control = app.app_control_service();
    let cleanup = WorkerCleanup(control.lifecycle().clone());
    let observations = control.lifecycle().fixture_inbox_workers_with_mode(
        chariox_app_runtime::worker_process::test_fixture::Mode::LifecycleInboxAutomations,
    );
    let store = app.durable_state_store();
    install_fixture_app(&store);
    let sessions = vec![workflow(&mut app, &root.join("workspace/first"), "first")];
    let agent = busy_agent(&mut app, &sessions[0].0);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let state = router.runtime_state();
    configure(&state, &sessions[0].0, &sessions[0].1, "first");
    record_fixture_connection(&state, &config);
    let LocalDaemonResponse::AgentTriggerCreated { trigger, .. } = state
        .create_agent_trigger(
            CreateAgentTriggerRequest {
                session_id: sessions[0].0.clone(),
                agent_id: agent,
                source: AgentTriggerSourceRequest::Notification {
                    source: AgentTriggerNotificationSource::App {
                        installation_id: "installed".into(),
                        event_name: "changed".into(),
                    },
                },
                busy_policy: None,
                prompt: Some("Review the pull request".into()),
            },
            "local",
        )
        .await
        .unwrap()
    else {
        panic!("agent trigger")
    };
    // An agent target's automation name is its subscription id, which the
    // App's automation setting must list.
    let agent_automation =
        super::agent_trigger_notification::notification_subscription_id(&trigger)
            .unwrap()
            .to_owned();
    for (id, scope) in [
        ("pull-request-updated", REPOSITORY),
        ("other-repository", OTHER_REPOSITORY),
    ] {
        assert!(matches!(
            route_in(&router, id, scope, json!({})).await.unwrap(),
            LocalDaemonResponse::AppInboxRoutes { .. }
        ));
    }
    // Bindings are named after the kernel's identity key, which is what the connector signs with.
    let kernel_id =
        crate::transport::gateway_identity::GatewayIdentity::from_config(&config).kernel_id();
    let resumes = state
        .event_delivery_resumes(&kernel_id, &config.event_delivery_environment_id)
        .unwrap();
    let claims = &resumes[0].routes;
    assert_eq!(claims.len(), 2);
    // Both bindings are addressed; only the one whose scope was published is
    // delivered.
    let occurred_at_ms = crate::session::unix_epoch_ms();
    let deliveries: Vec<_> = claims
        .iter()
        .map(|claim| {
            envelope_to(
                &claim.binding_id,
                "pull-request-updated",
                occurred_at_ms,
                ["first", &agent_automation],
                json!({"kind":"wanted"}),
            )
        })
        .collect();
    // The explicit bounded passes below are the only handler drivers here.
    let held_wake = state
        .app_control()
        .wake_pump()
        .try_begin(crate::session::unix_epoch_ms())
        .unwrap();
    let connection = connector(&state, &config, &services).await;
    assert_eq!(
        services
            .generate_in(REPOSITORY, deliveries.clone(), &json!({}), true)
            .await,
        1,
        "only the route of the published repository is delivered"
    );
    assert_eq!(counts(&state), (1, 0, 0));
    drain(&state, 1, &sessions).await;
    route_agent(&state).await;
    assert_eq!(runs(&state, &sessions).len(), 1, "workflow target ran once");
    assert_eq!(
        agent_fired(&state, &sessions[0].0),
        1,
        "agent target ran once"
    );

    connection.shutdown().await;
    drop(held_wake);
    control.lifecycle().shutdown_blocking().unwrap();
    drop(state);
    drop(router);
    drop(cleanup);
    drop(control);
    drop(store);
    // Reopen the whole kernel, then replay the same delivery and a new network
    // attempt of it: both targets stay at one run.
    let app = DaemonApp::bootstrap(config.clone()).unwrap();
    let control = app.app_control_service();
    let cleanup = WorkerCleanup(control.lifecycle().clone());
    let restarted = control.lifecycle().fixture_inbox_workers_with_mode(
        chariox_app_runtime::worker_process::test_fixture::Mode::LifecycleInboxAutomations,
    );
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
    let state = router.runtime_state();
    let held_wake = state
        .app_control()
        .wake_pump()
        .try_begin(crate::session::unix_epoch_ms())
        .unwrap();
    let connection = connector(&state, &config, &services).await;
    let original_runs = runs(&state, &sessions);
    assert_eq!(original_runs.len(), 1);
    // The route of the published repository is the one the generator knows.
    let watched = services
        .subscriptions
        .matching(GENERATOR, "dummy.test", REPOSITORY)
        .unwrap()[0]
        .binding_id
        .clone();
    let delivery = deliveries
        .iter()
        .find(|d| d.binding_id == watched)
        .unwrap()
        .clone();
    assert_eq!(services.deliver(vec![delivery.clone()], false).await, 1);
    let mut retry = delivery;
    retry.delivery_id = "delivery-new-attempt".into();
    assert_eq!(services.deliver(vec![retry], false).await, 1);
    assert_eq!(counts(&state), (0, 1, 0));
    state.app_inbox_pass(crate::session::unix_epoch_ms()).await;
    let owned = state.clone();
    tokio::task::spawn_blocking(move || owned.fixture_app_event_pass())
        .await
        .unwrap();
    route_agent(&state).await;
    assert_eq!(runs(&state, &sessions), original_runs);
    assert_eq!(agent_fired(&state, &sessions[0].0), 1);

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
