//! MP-08/MP-10/MP-11: Room Computer switches through the real router.
use super::*;

#[test]
fn mp11_room_computer_revoke_all_is_pruned_with_removed_agents() {
    run_test(|| {
        Box::pin(async {
            let workspace = crate::test_support::TestWorktree::new("mp11-room-revoke-prune");
            let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
            let (session, first) = crate::app::KernelSessionService::new(&mut app)
                .create_session(workspace.session_request())
                .unwrap();
            let second = spawn_test_agent(&mut app, session.id(), "second", "dev-stub");
            let revoked = app.room_computer_revoked.clone();
            let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 4);
            let ids = || revoked.lock().unwrap().iter().cloned().collect::<Vec<_>>();
            human(
                &router,
                KernelBrowserCommand::RevokeGrants { agent_id: None },
            )
            .await;
            let mut expected = vec![first.id().to_string(), second.id().to_string()];
            expected.sort();
            assert_eq!(ids(), expected);
            // MP-08 / MP-11 (local 461): bulk restore undoes "revoke all".
            let restored = human(
                &router,
                KernelBrowserCommand::GrantRoomComputer { agent_id: None },
            )
            .await;
            assert!(ids().is_empty());
            assert!(restored["room_computer"]
                .as_array()
                .unwrap()
                .iter()
                .all(|agent| agent["allowed"] == true));
            human(
                &router,
                KernelBrowserCommand::RevokeGrants { agent_id: None },
            )
            .await;
            assert_eq!(ids(), expected);
            for request in [
                LocalDaemonRequest::DestroyAgent(crate::local::DestroyAgentRequest {
                    session_id: session.id().into(),
                    agent_id: second.id().into(),
                }),
                LocalDaemonRequest::EndSession(crate::local::EndSessionRequest {
                    session_id: session.id().into(),
                }),
            ] {
                Box::pin(router.dispatch(terminal_command("mp11-remove", &request), request))
                    .await
                    .unwrap();
                assert!(!ids().contains(&second.id().to_string()));
            }
            assert!(ids().is_empty());
            router.runtime_state().shutdown_cleanup().await.unwrap();
        })
    });
}

// MP-08 / MP-11: revocations and explicit restores survive replay and checkpoints.
#[test]
fn mp11_room_computer_access_survives_kernel_restart() {
    run_test(|| {
        Box::pin(async {
            for checkpoint in [0, 1, 2] {
                let workspace = crate::test_support::TestWorktree::new("mp11-room-restart");
                let mut config = DaemonConfig::for_tests();
                config.user_config.state.snapshot_interval_events = Some(1);
                let mut app = DaemonApp::bootstrap(config.clone()).unwrap();
                let (session, first) = crate::app::KernelSessionService::new(&mut app)
                    .create_session(workspace.session_request())
                    .unwrap();
                let second = spawn_test_agent(&mut app, session.id(), "second", "dev-stub");
                let app = Arc::new(Mutex::new(app));
                let router = CommandRouter::with_interactive_capacity(app.clone(), 4);
                human(
                    &router,
                    KernelBrowserCommand::RevokeGrants { agent_id: None },
                )
                .await;
                if checkpoint == 1 {
                    app.lock().await.save_durable_state_snapshot().unwrap();
                }
                if checkpoint == 2 {
                    assert!(
                        router
                            .runtime_state()
                            .durable_snapshot_scheduler()
                            .unwrap()
                            .tick_once()
                            .unwrap()
                            .wrote_snapshot
                    );
                }
                router.runtime_state().shutdown_cleanup().await.unwrap();
                drop(router);
                drop(app);

                let app = DaemonApp::bootstrap(config.clone()).unwrap();
                assert!(
                    app.room_computer_revoked
                        .lock()
                        .unwrap()
                        .contains(first.id()),
                    "MP-11 restart must preserve owner denial"
                );
                assert!(app
                    .room_computer_revoked
                    .lock()
                    .unwrap()
                    .contains(second.id()));
                let app = Arc::new(Mutex::new(app));
                let router = CommandRouter::with_interactive_capacity(app.clone(), 4);
                let snapshot = human(&router, KernelBrowserCommand::ListGrants).await;
                assert!(snapshot["room_computer"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|row| row["allowed"] == false));
                human(
                    &router,
                    KernelBrowserCommand::GrantRoomComputer {
                        agent_id: Some(first.id().into()),
                    },
                )
                .await;
                if checkpoint == 1 {
                    app.lock().await.save_durable_state_snapshot().unwrap();
                }
                if checkpoint == 2 {
                    assert!(
                        router
                            .runtime_state()
                            .durable_snapshot_scheduler()
                            .unwrap()
                            .tick_once()
                            .unwrap()
                            .wrote_snapshot
                    );
                }
                router.runtime_state().shutdown_cleanup().await.unwrap();
                drop(router);
                drop(app);

                let app = DaemonApp::bootstrap(config.clone()).unwrap();
                assert!(
                    !app.room_computer_revoked
                        .lock()
                        .unwrap()
                        .contains(first.id()),
                    "MP-11 explicit restore must persist"
                );
                assert!(app
                    .room_computer_revoked
                    .lock()
                    .unwrap()
                    .contains(second.id()));
                let app = Arc::new(Mutex::new(app));
                let router = CommandRouter::with_interactive_capacity(app.clone(), 4);
                human(
                    &router,
                    KernelBrowserCommand::GrantRoomComputer { agent_id: None },
                )
                .await;
                if checkpoint == 1 {
                    app.lock().await.save_durable_state_snapshot().unwrap();
                }
                if checkpoint == 2 {
                    assert!(
                        router
                            .runtime_state()
                            .durable_snapshot_scheduler()
                            .unwrap()
                            .tick_once()
                            .unwrap()
                            .wrote_snapshot
                    );
                }
                router.runtime_state().shutdown_cleanup().await.unwrap();
                drop(router);
                drop(app);
                let app = DaemonApp::bootstrap(config).unwrap();
                assert!(
                    app.room_computer_revoked.lock().unwrap().is_empty(),
                    "MP-11 bulk restore must persist"
                );
            }
        })
    });
}
