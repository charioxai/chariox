use super::*;

#[tokio::test]
async fn session_reads_only_visible_wake_payloads_and_keeps_every_armed_wake() {
    let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(CreateSessionRequest::new("workspace", "worktree"))
        .unwrap();
    let room = session.id().to_owned();
    let mut agents = vec![agent.id().to_owned()];
    for alias in ["second", "third"] {
        agents.push(
            spawn_test_agent(&mut app, &room, alias, "claude-code")
                .id()
                .to_owned(),
        );
    }
    let mut db = rusqlite::Connection::open(app.durable_state_store().path()).unwrap();
    let tx = db.transaction().unwrap();
    let now = crate::session::unix_epoch_ms();
    let insert = |id: &str, recipient: &str, state: &str, at: u64, command: serde_json::Value| {
        let payload = serde_json::json!({
            "id":id, "task_id":"retained-task", "room_id":room, "agent_id":recipient,
            "registration_id":format!("completion-{id}"), "kind":"process", "label":"build",
            "state":state, "created_at_ms":at, "verified_at_ms":at,
            "next_due_ms":now + 3_600_000, "command":command, "fire_count":0, "missed_fires":0
        });
        tx.execute("INSERT INTO agent_wakes(id,task_id,room_id,agent_id,payload) VALUES(?1,'retained-task',?2,?3,?4)",
            rusqlite::params![id, room, recipient, payload.to_string()]).unwrap();
    };
    for index in 0..2048 {
        let recipient = if index >= 1984 {
            agents[0].clone()
        } else {
            format!("former-agent-{}", index / 64)
        };
        // An invisible old command must not prevent an otherwise valid Room read.
        let command = if index == 0 {
            serde_json::json!(42)
        } else {
            serde_json::json!(["x".repeat(16_384)])
        };
        insert(
            &format!("finished-{index}"),
            &recipient,
            "exited",
            now - 1_000,
            command,
        );
    }
    // Expired history is excluded even when it has the newest row id.
    insert(
        "expired",
        &agents[0],
        "fired",
        now - 86_400_001,
        serde_json::json!([]),
    );
    for index in 0..75 {
        insert(
            &format!("armed-{index}"),
            &agents[index / 25],
            ["scheduled", "starting", "running", "cancelling"][index % 4],
            now - 86_400_001,
            serde_json::json!([]),
        );
    }
    tx.commit().unwrap();
    drop(db);
    let router = CommandRouter::with_interactive_capacity(Arc::new(Mutex::new(app)), 1);
    let request = LocalDaemonRequest::GetSessionState(GetSessionStateRequest { session_id: room });
    let command = KernelCommand::from_local_request("visible-wake-history", None, None, &request);
    let LocalDaemonResponse::SessionState { session, .. } = router
        .dispatch(command, request)
        .await
        .expect("an invisible historical payload must not be decoded during a Room read")
    else {
        panic!("expected session state")
    };
    let ids: Vec<_> = session
        .agent_wakes()
        .iter()
        .map(|wake| wake.id.as_str())
        .collect();
    let expected: Vec<_> = (1984..2048)
        .map(|i| format!("finished-{i}"))
        .chain((0..75).map(|i| format!("armed-{i}")))
        .collect();
    assert_eq!(
        ids, expected,
        "keep the last64 recent finished wakes and every armed wake in row order"
    );
}
