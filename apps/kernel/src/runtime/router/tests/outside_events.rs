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
        router.runtime_state().start_active_turn_with_trace_id(
            session.id(),
            agent.id(),
            "outside-prompt",
            "outside-run",
            "outside-trace",
        );
        router
            .runtime_state()
            .outside_store()
            .outside_settled()
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
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0].value,OutsideEventValue::Agent { run_state:Some(state),.. } if *state==EventRunState::Working)
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
/// Every outside event request needs the access grant itself: another caller class, or a
/// grant that is gone, is refused with the typed kernel-access error and reaches no ledger.
fn outside_event_requests() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({"SubscribeEvents":{"targets":[{"kernel":"local","session_id":"any"}]}}),
        serde_json::json!({"WaitEvents":{"timeout_ms":1}}),
        serde_json::json!({"AckEvents":{"cursor":0}}),
        serde_json::json!({"ListEvents":{}}),
        serde_json::json!({"GetEventSubscription":{}}),
        serde_json::json!({"UnsubscribeEvents":{"all":true}}),
    ]
}
fn assert_kernel_access_refusal(result: Result<LocalDaemonResponse, DaemonError>, needle: &str) {
    match result {
        Err(DaemonError::LocalTransport { operation, message }) => {
            assert_eq!(operation, "kernel access");
            assert!(message.contains(needle), "{message}");
        }
        other => panic!("expected a kernel access refusal, got {other:?}"),
    }
}

#[test]
fn outside_event_requests_are_refused_without_the_external_agent_class() {
    run("outside-class-guard", async {
        let worktree = crate::test_support::TestWorktree::new("outside-class-guard");
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let session = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .unwrap();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        let grant = router
            .runtime_state()
            .insert_access_grant_for_test(session.id());
        for class in [
            KernelConnectionClass::Terminal,
            KernelConnectionClass::Unauthenticated,
            KernelConnectionClass::KernelAgent,
            KernelConnectionClass::Host,
        ] {
            for value in outside_event_requests() {
                let request = request(value);
                let mut caller = command(&grant, &request);
                caller.caller.connection_class = Some(class);
                assert_kernel_access_refusal(
                    router.dispatch(caller, request).await,
                    "require a local external access grant",
                );
            }
        }
    });
}

#[test]
fn outside_event_requests_are_refused_once_the_grant_is_revoked() {
    run("outside-revoked-guard", async {
        let worktree = crate::test_support::TestWorktree::new("outside-revoked-guard");
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let session = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .unwrap();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        let grant = router
            .runtime_state()
            .insert_access_grant_for_test(session.id());
        let live = request(serde_json::json!({"GetEventSubscription":{}}));
        // A live grant gets past the guard and is answered by the event ledger itself.
        assert!(matches!(
            router.dispatch(command(&grant, &live), live).await,
            Err(DaemonError::MachineRequest { code, .. }) if code == "events_refused"
        ));
        assert_eq!(
            router
                .runtime_state()
                .revoke_kernel_access(None, Some(&grant), "test revoke")
                .unwrap(),
            1
        );
        for value in outside_event_requests() {
            let request = request(value);
            let result = router.dispatch(command(&grant, &request), request).await;
            // What the holder is told on the wire: ended, and not worth a retry.
            let wire = crate::transport::kernel_protocol::map_kernel_error(
                result.as_ref().expect_err("an ended grant is refused"),
            );
            assert_eq!(wire.code, "kernel_access_ended", "{wire:?}");
            assert!(!wire.retryable, "{wire:?}");
            assert_kernel_access_refusal(result, "grant revoked or expired");
        }
    });
}

#[test]
fn outside_router_denies_session_agents_and_foreign_sessions() {
    run("outside-confinement", async {
        let worktree = crate::test_support::TestWorktree::new("outside-confinement");
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
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
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
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
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
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
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
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

#[test]
fn outside_router_acked_cancellation_still_returns_the_ending_receipt() {
    run("outside-ended-ack", async {
        let worktree = crate::test_support::TestWorktree::new("outside-ended-ack");
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
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
            request(serde_json::json!({"UnsubscribeEvents":{"all":true}})),
        )
        .await;
        let LocalDaemonResponse::EventsListed { next_cursor, .. } = dispatch(
            &router,
            &grant,
            request(serde_json::json!({"ListEvents":{}})),
        )
        .await
        else {
            panic!("expected events")
        };
        dispatch(
            &router,
            &grant,
            request(serde_json::json!({"AckEvents":{"cursor":next_cursor}})),
        )
        .await;
        let LocalDaemonResponse::EventsListed { events, .. } = dispatch(
            &router,
            &grant,
            request(serde_json::json!({"WaitEvents":{"timeout_ms":0}})),
        )
        .await
        else {
            panic!("expected events")
        };
        assert!(
            matches!(events.last().map(|event|&event.value),Some(OutsideEventValue::SubscriptionEnded { reason }) if reason=="cancelled"),
            "an acknowledged cancellation must still end a wait"
        );
    });
}

// MP-08 / MP-09 / MP-10 / MP-11 R1080-1: following is separate from creation.
#[test]
fn outside_router_preserves_create_results_after_subscription_cancellation() {
    run("outside-create-after-cancel", async {
        let worktree = crate::test_support::TestWorktree::new("outside-create-after-cancel");
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let session = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .unwrap();
        let app = Arc::new(Mutex::new(app));
        let router = CommandRouter::with_interactive_capacity(app.clone(), 4);
        let grant = router
            .runtime_state()
            .insert_access_grant_for_test(session.id());
        dispatch(&router, &grant, request(serde_json::json!({"SubscribeEvents":{"targets":[{"kernel":"local","session_id":session.id()}]}}))).await;
        dispatch(
            &router,
            &grant,
            request(serde_json::json!({"UnsubscribeEvents":{"all":true}})),
        )
        .await;
        for input in [
            LocalDaemonRequest::CreateSession(worktree.session_request()),
            request(
                serde_json::json!({"SpawnAgent":{"session_id":session.id(),"provider":"dev-stub"}}),
            ),
            request(
                serde_json::json!({"SpawnAgents":{"session_id":session.id(),"agents":[{"provider":"dev-stub"}]}}),
            ),
        ] {
            let response = dispatch(&router, &grant, input).await;
            let value = serde_json::to_value(&response).unwrap();
            let body = value.as_object().unwrap().values().next().unwrap();
            assert_eq!(
                body["event_subscription_error"]["code"],
                "events_subscription_failed"
            );
            if let Some(created) = body.get("session") {
                assert!(app
                    .lock()
                    .await
                    .sessions()
                    .get_session(created["id"].as_str().unwrap())
                    .is_ok());
            }
            assert!(
                body.get("session").is_some()
                    || body.get("agent").is_some()
                    || body.get("agents").is_some()
            );
        }
        let mut opt_out = worktree.session_request();
        opt_out.no_subscribe = true;
        let value = serde_json::to_value(
            dispatch(&router, &grant, LocalDaemonRequest::CreateSession(opt_out)).await,
        )
        .unwrap();
        assert!(value["SessionCreated"]
            .get("event_subscription_error")
            .is_none());
    });
}

#[test]
fn outside_router_preserves_create_ids_when_event_target_capacity_is_full() {
    run("outside-create-at-capacity", async {
        let worktree = crate::test_support::TestWorktree::new("outside-create-at-capacity");
        let app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let session = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .unwrap();
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        let grant = router
            .runtime_state()
            .insert_access_grant_for_test(session.id());
        let kernel = router.config_projection.snapshot().daemon_id;
        router
            .runtime_state()
            .outside_store()
            .outside_mutate(
                crate::durable_state::agent_lifecycle::outside::Mutation::Subscribe {
                    id: grant.clone(),
                    home: true,
                    baseline: vec![],
                    open_decisions: vec![],
                    targets: (0..1024)
                        .map(|i| EventTarget {
                            kernel: kernel.clone(),
                            session_id: session.id().into(),
                            agent_id: Some(format!("capacity-agent-{i}")),
                        })
                        .collect(),
                },
            )
            .unwrap();
        let response = dispatch(
            &router,
            &grant,
            LocalDaemonRequest::CreateSession(worktree.session_request()),
        )
        .await;
        let value = serde_json::to_value(response).unwrap();
        assert!(value["SessionCreated"]["session"]["id"].is_string());
        assert_eq!(
            value["SessionCreated"]["event_subscription_error"]["code"],
            "events_subscription_failed"
        );
        assert_eq!(
            router
                .runtime_state()
                .outside_store()
                .outside_part(&grant)
                .unwrap()
                .unwrap()
                .targets
                .len(),
            1024
        );
    });
}

#[test]
fn outside_router_a_removed_agent_leaves_the_other_targets_flowing() {
    run("outside-removed-agent", async {
        let worktree = crate::test_support::TestWorktree::new("outside-removed-agent");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let session = app
            .sessions_mut()
            .create_session(worktree.session_request())
            .unwrap();
        let removed = spawn_test_agent(&mut app, session.id(), "gone", "dev-stub");
        let kept = spawn_test_agent(&mut app, session.id(), "kept", "dev-stub");
        let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
        let grant = router
            .runtime_state()
            .insert_access_grant_for_test(session.id());
        let target = |agent: &str| serde_json::json!({"kernel":"local","session_id":session.id(),"agent_id":agent});
        let subscribe = request(serde_json::json!({"SubscribeEvents":{
            "targets":[target(removed.id()), target(kept.id())]
        }}));
        dispatch(&router, &grant, subscribe).await;
        let destroy = LocalDaemonRequest::DestroyAgent(DestroyAgentRequest {
            session_id: session.id().into(),
            agent_id: removed.id().into(),
        });
        router
            .dispatch(
                KernelCommand::from_local_request("destroy-removed", None, None, &destroy),
                destroy,
            )
            .await
            .unwrap();

        // The holder is told, and the removed agent no longer counts as a target.
        let LocalDaemonResponse::EventSubscription { subscription } = dispatch(
            &router,
            &grant,
            request(serde_json::json!({"GetEventSubscription":{}})),
        )
        .await
        else {
            panic!("expected subscription")
        };
        assert_eq!(
            subscription
                .targets
                .iter()
                .map(|target| target.agent_id.as_deref())
                .collect::<Vec<_>>(),
            [Some(kept.id())]
        );
        router.runtime_state().start_active_turn_with_trace_id(
            session.id(),
            kept.id(),
            "kept-prompt",
            "kept-run",
            "kept-trace",
        );
        router
            .runtime_state()
            .outside_store()
            .outside_settled()
            .unwrap();
        let LocalDaemonResponse::EventsListed { events, .. } = dispatch(
            &router,
            &grant,
            request(serde_json::json!({"ListEvents":{}})),
        )
        .await
        else {
            panic!("expected events")
        };
        assert!(events.iter().any(|event| matches!(
            &event.value,
            OutsideEventValue::Session {
                state: EventSessionState::AgentRemoved
            }
        ) && event.agent_id.as_deref() == Some(removed.id())));
        assert!(events.iter().any(|event| matches!(
            &event.value,
            OutsideEventValue::Agent {
                run_state: Some(EventRunState::Working),
                ..
            }
        ) && event.agent_id.as_deref() == Some(kept.id())));
    });
}
