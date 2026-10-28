use super::*;

#[test]
fn retirement_upgrade_from_old_checkpoint_preserves_history_and_event_order() {
    check_old_retirement_database(true);
}

#[test]
fn retirement_upgrade_from_old_event_log_preserves_history_and_event_order() {
    check_old_retirement_database(false);
}

fn check_old_retirement_database(checkpoint: bool) {
    let config = DaemonConfig::for_tests();
    let worktree = crate::test_support::TestWorktree::new("retirement-upgrade");
    let mut ended = Vec::new();
    let mut old_checkpoint = None;
    {
        let harness = LocalRouterTestHarness::with_config(config.clone());
        for _ in 0..3 {
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
                LocalDaemonResponse::SessionEnded { session } => ended.push(session),
                other => panic!("unexpected end: {other:?}"),
            }
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
        // An older database has no retired index or migration receipt.
        connection
            .execute_batch(
                "DROP TABLE retired_sessions;
            DELETE FROM durable_state_metadata WHERE metadata_key='retired_sessions_backfill';",
            )
            .unwrap();
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
