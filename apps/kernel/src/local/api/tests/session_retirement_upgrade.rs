use super::*;

#[test]
fn retirement_upgrade_from_old_checkpoint_preserves_history_and_event_order() {
    check_old_retirement_database(true, false);
}

#[test]
fn retirement_upgrade_from_old_event_log_preserves_history_and_event_order() {
    check_old_retirement_database(false, false);
}

#[test]
fn retirement_rollback_with_checkpoint_revalidates_existing_receipt() {
    check_old_retirement_database(true, true);
}

#[test]
fn retirement_rollback_with_event_log_revalidates_existing_receipt() {
    check_old_retirement_database(false, true);
}

#[test]
fn retirement_rollback_reconciles_restores_and_deletes_of_indexed_rooms() {
    let config = DaemonConfig::for_tests();
    let worktree = crate::test_support::TestWorktree::new("retirement-rollback");
    let ended = {
        let harness = LocalRouterTestHarness::with_config(config.clone());
        (0..3)
            .map(|_| create_ended_upgrade_session(&harness, &worktree))
            .collect::<Vec<_>>()
    };
    {
        // Verify the prefix containing all three ended Rooms before rollback.
        let harness = LocalRouterTestHarness::with_config(config.clone());
        harness
            .dispatch(LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
                session_ref: ended[1].id().to_string(),
                workspace_id: None,
            }))
            .unwrap();
        harness
            .dispatch(LocalDaemonRequest::RestoreProject(RestoreProjectRequest {
                project_id: ended[2].project_id().to_string(),
            }))
            .unwrap();
    }
    {
        // A predecessor leaves the previously indexed versions untouched even
        // when its later events delete or restore those Rooms.
        let connection = rusqlite::Connection::open(config.durable_state_path()).unwrap();
        for session in &ended[1..] {
            connection.execute(
                "INSERT INTO retired_sessions(owner_id,session_id,project_id,payload) VALUES (?1,?2,?3,?4)",
                rusqlite::params![config.daemon_id,session.id(),session.project_id(),serde_json::to_string(session).unwrap()],
            ).unwrap();
        }
    }
    let harness = LocalRouterTestHarness::with_config(config.clone());
    assert!(
        harness
            .dispatch(LocalDaemonRequest::GetSessionState(
                GetSessionStateRequest {
                    session_id: ended[1].id().to_string(),
                }
            ))
            .is_err(),
        "a rollback delete must remove the previously indexed Room"
    );
    harness.with_app(|app| {
        let store = app.durable_state_store();
        assert!(store
            .retired_session(&config.daemon_id, ended[0].id())
            .unwrap()
            .is_some());
        for session in &ended[1..] {
            assert!(
                store
                    .retired_session(&config.daemon_id, session.id())
                    .unwrap()
                    .is_none(),
                "a later restore or delete must remove the stale indexed record"
            );
        }
    });
}

fn check_old_retirement_database(checkpoint: bool, retain_receipt: bool) {
    let config = DaemonConfig::for_tests();
    let worktree = crate::test_support::TestWorktree::new("retirement-upgrade");
    let indexed_before_rollback = retain_receipt.then(|| {
        let harness = LocalRouterTestHarness::with_config(config.clone());
        create_ended_upgrade_session(&harness, &worktree)
    });
    let mut ended = Vec::new();
    let mut old_checkpoint = None;
    {
        let harness = LocalRouterTestHarness::with_config(config.clone());
        for _ in 0..3 {
            ended.push(create_ended_upgrade_session(&harness, &worktree));
        }
        if checkpoint {
            harness.with_app(|app| {
                app.save_durable_state_snapshot().unwrap();
                let store = app.durable_state_store();
                let snapshot = store.latest_snapshot().unwrap().unwrap();
                let mut payload = snapshot.payload;
                // Protocol 636 checkpoints kept ended Rooms in `sessions`.
                payload["sessions"] = serde_json::to_value(&ended).unwrap();
                old_checkpoint =
                    Some((snapshot.sequence, serde_json::to_string(&payload).unwrap()));
            });
        }
        harness
            .dispatch(LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
                session_ref: ended[1].id().to_string(),
                workspace_id: None,
            }))
            .unwrap();
        harness
            .dispatch(LocalDaemonRequest::RestoreProject(RestoreProjectRequest {
                project_id: ended[2].project_id().to_string(),
            }))
            .unwrap();
    }
    {
        let connection = rusqlite::Connection::open(config.durable_state_path()).unwrap();
        if retain_receipt {
            // A predecessor running after an upgrade leaves the receipt intact,
            // but does not index its newly ended Rooms.
            assert_eq!(
                connection
                    .query_row(
                        "SELECT count(*) FROM durable_state_metadata
                     WHERE metadata_key='retired_sessions_backfill'",
                        [],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                1
            );
            for session in &ended {
                connection
                    .execute(
                        "DELETE FROM retired_sessions WHERE session_id=?1",
                        [session.id()],
                    )
                    .unwrap();
            }
        } else {
            // An older database has no retired index or migration receipt.
            connection
                .execute_batch(
                    "DROP TABLE retired_sessions;
            DELETE FROM durable_state_metadata WHERE metadata_key='retired_sessions_backfill';",
                )
                .unwrap();
        }
        connection
            .execute_batch(
                "DELETE FROM durable_state_snapshots;
            DELETE FROM durable_state_checkpoint_entities;
            DELETE FROM durable_state_checkpoint_manifest;
            DELETE FROM durable_state_owner_checkpoint_entities;
            DELETE FROM durable_state_owner_checkpoint_manifest;",
            )
            .unwrap();
        if let Some((sequence, payload)) = old_checkpoint {
            connection.execute("INSERT INTO durable_state_snapshots(sequence,timestamp_ms,payload_json) VALUES (?1,1,?2)",
                rusqlite::params![sequence as i64,payload]).unwrap();
            // The ended Rooms now exist only in the old checkpoint; later
            // restore/delete events must still supersede that checkpoint.
            connection
                .execute(
                    "DELETE FROM durable_state_events WHERE sequence<=?1",
                    [sequence as i64],
                )
                .unwrap();
        }
    }
    let harness = LocalRouterTestHarness::with_config(config.clone());
    if let Some(session) = indexed_before_rollback {
        harness
            .dispatch(LocalDaemonRequest::GetSessionState(
                GetSessionStateRequest {
                    session_id: session.id().to_string(),
                },
            ))
            .expect("previously indexed Room must survive reconciliation");
    }
    harness.with_app(|app| {
        assert!(
            app.agent_runtime_projection_store()
                .list_for_session(ended[0].id())
                .is_empty(),
            "replaying an end must remove the Room's agent runtime projections"
        );
    });
    let loaded = harness
        .dispatch(LocalDaemonRequest::GetSessionState(
            GetSessionStateRequest {
                session_id: ended[0].id().to_string(),
            },
        ))
        .expect("older ended Room must remain readable after upgrade");
    assert!(
        matches!(loaded, LocalDaemonResponse::SessionState { session, .. }
        if session.status() == crate::session::SessionStatus::Ended)
    );
    assert!(
        harness
            .dispatch(LocalDaemonRequest::GetSessionState(
                GetSessionStateRequest {
                    session_id: ended[1].id().to_string(),
                }
            ))
            .is_err(),
        "a later delete must win over the older checkpoint/end event"
    );
    harness.with_app(|app| {
        let store = app.durable_state_store();
        assert!(store
            .retired_session(&config.daemon_id, ended[1].id())
            .unwrap()
            .is_none());
        assert!(
            store
                .retired_session(&config.daemon_id, ended[2].id())
                .unwrap()
                .is_none(),
            "a later restore must not leave an ended record in the index"
        );
    });
    harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: ended[0].id().to_string(),
                client_id: "upgrade-client".into(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("older ended Room must remain attachable");
    harness
        .dispatch(LocalDaemonRequest::DeleteSession(DeleteSessionRequest {
            session_ref: ended[0].id().to_string(),
            workspace_id: None,
        }))
        .expect("upgraded Room must remain deletable");
    drop(harness);
    let reopened = LocalRouterTestHarness::with_config(config);
    assert!(
        reopened
            .dispatch(LocalDaemonRequest::GetSessionState(
                GetSessionStateRequest {
                    session_id: ended[0].id().to_string(),
                }
            ))
            .is_err(),
        "a second boot must not backfill a subsequently deleted Room"
    );
}

fn create_ended_upgrade_session(
    harness: &LocalRouterTestHarness,
    worktree: &crate::test_support::TestWorktree,
) -> crate::session::RuntimeSession {
    let session = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            worktree
                .session_request()
                .with_project_selection(SessionProjectSelection::New { name: None }),
        ))
        .unwrap()
    {
        LocalDaemonResponse::SessionCreated { session, .. } => session,
        other => panic!("unexpected create: {other:?}"),
    };
    match harness
        .dispatch(LocalDaemonRequest::EndSession(EndSessionRequest {
            session_id: session.id().to_string(),
        }))
        .unwrap()
    {
        LocalDaemonResponse::SessionEnded { session } => session,
        other => panic!("unexpected end: {other:?}"),
    }
}
