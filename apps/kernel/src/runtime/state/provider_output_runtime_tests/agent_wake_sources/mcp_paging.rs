use super::*;
use serde_json::{json, Value};

const WAKES: &str = "chariox.events.wakes";
const AGENTS: &str = "chariox.list_session_agents";

async fn auth(f: &Fixture) -> String {
    f.app
        .lock()
        .await
        .providers
        .get_run(&f.run)
        .unwrap()
        .runtime_mcp_auth_token()
        .unwrap()
        .to_string()
}

fn mcp_payload(result: crate::transport::runtime_tools::RuntimeToolResult) -> Value {
    assert!(result.ok, "{}", result.payload);
    let (content, _) = crate::transport::mcp_server::runtime_tool_content_for_test(result.payload);
    let text = content[0]["text"].as_str().unwrap();
    assert!(
        text.len() < 64 * 1024 && !text.contains("Result exceeded the 64 KiB MCP text limit"),
        "MCP must return a complete page"
    );
    serde_json::from_str(text).unwrap()
}

#[tokio::test]
async fn mcp_paging_retains_access_to_five_hundred_wake_receipts() {
    let f = fixture("paging-wakes", true).await;
    let task = f.begin(&f.prompt_id);
    let token = auth(&f).await;
    let start = crate::session::unix_epoch_ms();
    let mut timers = Vec::new();
    for i in 0..25 {
        let mut wake = f.wake(&task, &format!("timer-{i:02}"));
        wake.kind = "timer".into();
        wake.state = "scheduled".into();
        // This valid label measures UTF-8 bytes rather than character count.
        wake.label = "🕰".repeat(200);
        wake.created_at_ms = start;
        wake.next_due_ms = Some(start + 1000);
        wake.interval_ms = Some(60_000);
        f.store()
            .agent_lifecycle(Operation::ArmWake {
                task: task.task_id.clone(),
                prompt: task.prompt_id.clone(),
                wake: Box::new(wake),
            })
            .unwrap();
        timers.push(format!("timer-{i:02}"));
    }

    let mut receipts = Vec::new();
    let mut after = 0;
    for i in 0..20 {
        let now = start + 1000 + i * 60_000;
        f.store()
            .agent_lifecycle(Operation::FireWakes { now })
            .unwrap();
        for event in f.store().agent_inbox(&f.room, &f.agent, after).unwrap() {
            after = event.sequence;
            receipts.push(after);
            f.store()
                .agent_lifecycle(Operation::Ack {
                    room: f.room.clone(),
                    agent: f.agent.clone(),
                    sequence: after,
                    handled: true,
                    now,
                })
                .unwrap();
        }
    }
    assert_eq!(receipts.len(), 500);

    let mut args = json!({});
    let mut seen = Vec::new();
    loop {
        let page = mcp_payload(call(&f, &token, WAKES, &task, args).await.unwrap());
        assert!(page["scheduler"]["stalled"].is_boolean());
        if seen.is_empty() {
            assert_eq!(page["wakes"][0]["id"], timers.last().unwrap().as_str());
        }
        let rows = page["receipts"].as_array().unwrap();
        assert!(!rows.is_empty());
        seen.extend(rows.iter().map(|row| row["sequence"].as_u64().unwrap()));
        assert!(seen.len() <= receipts.len(), "cursor advanced");
        let cursor = &page["next_receipts_cursor"];
        if cursor.is_null() {
            break;
        }
        args = json!({"receipts_cursor": cursor});
    }
    receipts.reverse();
    assert_eq!(seen, receipts);
}

#[tokio::test]
async fn mcp_paging_keeps_room_agents_and_task_history_reachable() {
    let f = fixture_with_agent_limit("paging-agents", true, Some(64)).await;
    let primary = f.begin(&f.prompt_id);
    let token = auth(&f).await;
    let mut agents = vec![f.agent.clone()];
    let now = crate::session::unix_epoch_ms();
    for i in 1..60 {
        let mut app = f.app.lock().await;
        let peer = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(f.room.as_str(), "dev-stub")
                    .with_alias(format!("peer-{i}")),
            )
            .unwrap();
        let id = peer.id().to_string();
        drop(app);
        for run in 0..3 {
            let prompt = format!("past-task-{i}-{run}");
            let run = format!("past-run-{i}-{run}");
            f.store()
                .agent_lifecycle(Operation::Begin {
                    owner: "local".into(),
                    room: f.room.clone(),
                    agent: id.clone(),
                    prompt: prompt.clone(),
                    run: Some(run.clone()),
                    now,
                })
                .unwrap();
            f.store()
                .agent_lifecycle(Operation::Settle {
                    room: f.room.clone(),
                    agent: id.clone(),
                    prompt,
                    run,
                    has_answer: true,
                    cancelled: false,
                    now,
                })
                .unwrap();
        }
        agents.push(id);
    }

    let page = |arguments| async {
        mcp_payload(
            f.runtime
                .dispatch_authenticated_runtime_tool_call(&token, AGENTS, arguments)
                .await
                .unwrap(),
        )
    };

    let first = page(json!({})).await;
    assert_eq!(first["agent_tasks"].as_array().unwrap().len(), 1);
    assert_eq!(first["agent_tasks"][0]["task_id"], primary.task_id);
    let mut roster = Vec::new();
    let mut current = first;
    loop {
        let rows = current["agents"].as_array().unwrap();
        assert!(!rows.is_empty());
        for row in rows {
            assert!(row["state"].is_string());
            assert!(row["queued_prompt_count"].is_number());
            roster.push(row["id"].as_str().unwrap().to_string());
        }
        assert!(roster.len() <= 60, "cursor advanced");
        let cursor = &current["next_agents_cursor"];
        if cursor.is_null() {
            break;
        }
        current = page(json!({"agents_cursor": cursor})).await;
    }
    let created = f.runtime.session_agents(&f.room);
    let newest = created
        .iter()
        .max_by_key(|a| (a.created_at_ms(), a.id()))
        .unwrap();
    let oldest = created
        .iter()
        .min_by_key(|a| (a.created_at_ms(), a.id()))
        .unwrap();
    assert_eq!(roster.first().map(String::as_str), Some(newest.id()));
    assert_eq!(roster.last().map(String::as_str), Some(oldest.id()));
    roster.sort();
    agents.sort();
    assert_eq!(roster, agents);

    let mut args = json!({"include_history": true});
    let mut tasks = Vec::new();
    loop {
        let current = page(args).await;
        let rows = current["agent_tasks"].as_array().unwrap();
        assert!(!rows.is_empty());
        tasks.extend(
            rows.iter()
                .map(|row| row["task_id"].as_str().unwrap().to_string()),
        );
        assert!(tasks.len() <= 178, "cursor advanced");
        let cursor = &current["next_tasks_cursor"];
        if cursor.is_null() {
            break;
        }
        args = json!({"include_history": true, "tasks_cursor": cursor});
    }
    assert_eq!(tasks.first().unwrap(), "past-task-59-2");
    assert_eq!(tasks.last().unwrap(), &primary.task_id);
    assert_eq!(tasks.len(), 178);
}

#[tokio::test]
async fn mcp_paging_inbox_only_marks_events_it_can_return() {
    let f = fixture("paging-inbox", true).await;
    let task = f.begin(&f.prompt_id);
    let token = auth(&f).await;
    f.arm(&task, "large-events").unwrap();
    for i in 0..6 {
        f.store()
            .agent_lifecycle(Operation::WakeOccurred {
                id: "large-events".into(),
                occurrence: format!("test-event-{i}"),
                kind: "test-event".into(),
                payload: json!({"text": "x".repeat(14 * 1024)}),
                summary: None,
                end: None,
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
    }
    let first = mcp_payload(
        call(
            &f,
            &token,
            "chariox.events.inbox",
            &task,
            json!({"limit": 64}),
        )
        .await
        .unwrap(),
    );
    let rows = first["events"].as_array().unwrap();
    assert!(!rows.is_empty() && rows.len() < 6);
    assert_eq!(first["more"], true);
    let unread = f
        .store()
        .agent_unread_inbox(&f.room, &f.agent, 0, u64::MAX >> 1, None)
        .unwrap();
    assert_eq!(unread.len(), 6 - rows.len());
    let second = mcp_payload(
        call(
            &f,
            &token,
            "chariox.events.inbox",
            &task,
            json!({"after": first["next_after"]}),
        )
        .await
        .unwrap(),
    );
    assert_eq!(second["events"].as_array().unwrap().len(), unread.len());
    assert_eq!(second["more"], false);
}

#[tokio::test]
async fn mcp_paging_subscriptions_puts_live_registrations_ahead_of_history() {
    let f = fixture("paging-subscriptions", true).await;
    let task = f.begin(&f.prompt_id);
    let token = auth(&f).await;
    let start = crate::session::unix_epoch_ms();
    for i in 0..240 {
        let id = format!("test-cancelled-registration-{i:04}");
        let mut wake = f.wake(&task, &id);
        wake.kind = "timer".into();
        wake.state = "scheduled".into();
        wake.created_at_ms = start;
        wake.next_due_ms = Some(start + 60_000);
        f.store()
            .agent_lifecycle(Operation::ArmWake {
                task: task.task_id.clone(),
                prompt: task.prompt_id.clone(),
                wake: Box::new(wake),
            })
            .unwrap();
        f.store()
            .agent_lifecycle(Operation::CancelWake {
                id,
                task: task.task_id.clone(),
                prompt: Some(task.prompt_id.clone()),
                pending: false,
            })
            .unwrap();
    }
    let live = f.arm(&task, "live-registration").unwrap();
    let first = mcp_payload(
        call(&f, &token, "chariox.events.subscriptions", &task, json!({}))
            .await
            .unwrap(),
    );
    assert_eq!(first["registrations"].as_array().unwrap().len(), 1);
    assert_eq!(first["registrations"][0]["id"], live.registration_id);
    assert_eq!(first["task"]["obligations"].as_array().unwrap().len(), 1);
}
