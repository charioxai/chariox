//! MP-08 / MP-09 / MP-10 / MP-11: actual router admission and producer seams.
//! Supplementary regressions; these are not official-provider live acceptance.
use super::*;
use crate::local::*;

fn command(grant: &str, request: &LocalDaemonRequest) -> KernelCommand {
    let mut command = KernelCommand::from_local_request(
        format!("outside-test-{:016x}", rand::random::<u64>()),
        None,
        None,
        request,
    );
    command.caller.caller_id = grant.into();
    command.caller.connection_class = Some(KernelConnectionClass::ExternalAgent);
    command.caller.user_id = Some(DEFAULT_LOCAL_USER_ID.into());
    command
}
async fn dispatch(
    router: &CommandRouter,
    grant: &str,
    request: LocalDaemonRequest,
) -> LocalDaemonResponse {
    router
        .dispatch(command(grant, &request), request)
        .await
        .unwrap()
}
fn request(value: serde_json::Value) -> LocalDaemonRequest {
    serde_json::from_value(value).unwrap()
}
fn run(name: &str, test: impl std::future::Future<Output = ()> + Send + 'static) {
    std::thread::Builder::new()
        .name(name.into())
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(64 * 1024 * 1024)
                .enable_all()
                .build()
                .unwrap()
                .block_on(test)
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn outside_router_collects_states_and_ends_a_running_wait_on_revoke() {
    run("outside-router", async {
        let worktree = crate::test_support::TestWorktree::new("outside-router");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let session = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .unwrap();
        let agent = spawn_test_agent(&mut app, session.id(), "lane", "dev-stub");
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(app.clone(), 4);
        let grant = router
            .runtime_state()
            .insert_access_grant_for_test(session.id());
        dispatch(&router,&grant,request(serde_json::json!({"SubscribeEvents":{"targets":[{"kernel":"local","session_id":session.id(),"agent_id":agent.id()}]}}))).await;
        app.lock()
            .await
            .agents_mut()
            .set_agent_state(agent.id(), crate::agent::AgentState::Working)
            .unwrap();
        app.lock()
            .await
            .agents_mut()
            .set_agent_state(agent.id(), crate::agent::AgentState::Idle)
            .unwrap();
        let LocalDaemonResponse::EventsListed {
            events,
            next_cursor,
            ..
        } = dispatch(
            &router,
            &grant,
            request(serde_json::json!({"ListEvents":{}})),
        )
        .await
        else {
            panic!("expected events")
        };
        assert_eq!(events.len(), 2);
        assert!(
            matches!(&events[0].value,OutsideEventValue::Agent { run_state:Some(state),.. } if *state==EventRunState::Working)
        );
        assert!(
            matches!(&events[1].value,OutsideEventValue::Agent { run_state:Some(state),.. } if *state==EventRunState::Idle)
        );
        let wait =
            request(serde_json::json!({"WaitEvents":{"since":next_cursor,"timeout_ms":30000}}));
        let waiting = tokio::spawn({
            let router = router.clone();
            let grant = grant.clone();
            async move { dispatch(&router, &grant, wait).await }
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        router
            .runtime_state()
            .revoke_kernel_access(None, Some(&grant), "revoked")
            .unwrap();
        let result = timeout(Duration::from_secs(2), waiting)
            .await
            .unwrap()
            .unwrap();
        let LocalDaemonResponse::EventsListed { events, .. } = result else {
            panic!("expected final event")
        };
        assert!(
            matches!(&events.last().unwrap().value,OutsideEventValue::SubscriptionEnded { reason } if reason=="revoked")
        );
    });
}
#[test]
fn outside_router_denies_session_agents_and_foreign_sessions() {
    run("outside-confinement", async {
        let worktree = crate::test_support::TestWorktree::new("outside-confinement");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let session = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .unwrap();
        let mut foreign = worktree.session_request();
        foreign.owner_user_id = "stranger".into();
        let foreign = app.sessions_mut().create_session(foreign).unwrap();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        let grant = router
            .runtime_state()
            .insert_access_grant_for_test(session.id());
        let subscribe = request(
            serde_json::json!({"SubscribeEvents":{"targets":[{"kernel":"local","session_id":foreign.id()}]}}),
        );
        assert!(router
            .dispatch(command(&grant, &subscribe), subscribe)
            .await
            .is_err());
        for value in [
            serde_json::json!({"SubscribeEvents":{"targets":[{"kernel":"local","session_id":session.id()}]}}),
            serde_json::json!({"ListEvents":{}}),
            serde_json::json!({"WaitEvents":{"timeout_ms":1}}),
            serde_json::json!({"AckEvents":{"cursor":0}}),
            serde_json::json!({"UnsubscribeEvents":{"all":true}}),
            serde_json::json!({"GetEventSubscription":{}}),
        ] {
            let request = request(value);
            let mut caller = command(&grant, &request);
            caller.caller.connection_class = Some(KernelConnectionClass::KernelAgent);
            assert!(router.dispatch(caller, request).await.is_err());
        }
    });
}
#[test]
fn outside_router_automatically_follows_created_work_and_honors_opt_out() {
    run("outside-created", async {
        let worktree = crate::test_support::TestWorktree::new("outside-created");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let session = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .unwrap();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        let grant = router
            .runtime_state()
            .insert_access_grant_for_test(session.id());
        let LocalDaemonResponse::SessionCreated {
            session: created, ..
        } = dispatch(
            &router,
            &grant,
            LocalDaemonRequest::CreateSession(worktree.session_request()),
        )
        .await
        else {
            panic!("expected session")
        };
        let LocalDaemonResponse::EventSubscription { subscription } = dispatch(
            &router,
            &grant,
            request(serde_json::json!({"GetEventSubscription":{}})),
        )
        .await
        else {
            panic!("expected subscription")
        };
        assert_eq!(subscription.targets.len(), 1);
        assert_eq!(subscription.targets[0].session_id, created.id());
        let mut no_follow = worktree.session_request();
        no_follow.no_subscribe = true;
        dispatch(
            &router,
            &grant,
            LocalDaemonRequest::CreateSession(no_follow),
        )
        .await;
        let LocalDaemonResponse::EventSubscription { subscription } = dispatch(
            &router,
            &grant,
            request(serde_json::json!({"GetEventSubscription":{}})),
        )
        .await
        else {
            panic!("expected subscription")
        };
        assert_eq!(subscription.targets.len(), 1);
    });
}
#[test]
fn outside_router_observes_a_real_terminal_job_without_a_terminal_watch() {
    run("outside-terminal", async {
        let worktree = crate::test_support::TestWorktree::new("outside-terminal");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let session = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .unwrap();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        let grant = router
            .runtime_state()
            .insert_access_grant_for_test(session.id());
        dispatch(&router,&grant,request(serde_json::json!({"SubscribeEvents":{"targets":[{"kernel":"local","session_id":session.id()}]}}))).await;
        dispatch(
            &router,
            &grant,
            LocalDaemonRequest::RoomTerminal(RoomTerminalRequest {
                session_id: session.id().into(),
                acting_agent: None,
                host_agent: None,
                operation: RoomTerminalOperation::Open {
                    cols: 80,
                    rows: 24,
                    title: None,
                    command: Some(vec!["/bin/sh".into(), "-c".into(), "exit 7".into()]),
                },
            }),
        )
        .await;
        let LocalDaemonResponse::EventsListed { events, .. } = dispatch(
            &router,
            &grant,
            request(serde_json::json!({"WaitEvents":{"timeout_ms":2000}})),
        )
        .await
        else {
            panic!("expected events")
        };
        assert!(events.iter().any(|event| matches!(
            event.value,
            OutsideEventValue::Terminal { exit_code: Some(7) }
        )));
    });
}

#[test]
fn outside_router_critical_approvals_are_only_section_eight_pointers() {
    run("outside-pointer", async {
        let worktree = crate::test_support::TestWorktree::new("outside-pointer");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let session = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .unwrap();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        let grant = router
            .runtime_state()
            .insert_access_grant_for_test(session.id());
        dispatch(&router,&grant,request(serde_json::json!({"SubscribeEvents":{"targets":[{"kernel":"local","session_id":session.id()}]}}))).await;
        let canary = "outside-event-approval-body-canary";
        let interaction = RuntimeInteraction::for_kernel_operation(
            "approval",
            "validation:outside-pointer",
            canary,
            canary,
            vec![
                RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
                RuntimeInteractionChoice::new("approve", "Approve", "approve", None)
                    .requiring_passkey(),
            ],
        );
        let _responder = router
            .runtime_state()
            .create_kernel_operation_interaction(session.id(), session.owner_user_id(), interaction)
            .await
            .unwrap();
        let response = dispatch(
            &router,
            &grant,
            request(serde_json::json!({"ListEvents":{}})),
        )
        .await;
        let wire = serde_json::to_string(&response).unwrap();
        assert!(!wire.contains(canary));
        assert!(wire.contains("one_line"));
        assert!(wire.contains("expires_at"));
        let part = router
            .runtime_state()
            .outside_store()
            .outside_list(&grant, None, 100)
            .unwrap()
            .unwrap();
        assert!(!serde_json::to_string(&part).unwrap().contains(canary));
    });
}
