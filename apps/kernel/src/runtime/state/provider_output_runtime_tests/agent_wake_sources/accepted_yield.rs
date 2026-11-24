use super::*;

#[tokio::test]
async fn accepted_yield_allows_same_turn_inbox_and_ack() {
    let f = fixture("accepted-yield-inbox", true).await;
    let task = f.begin(&f.prompt_id);
    let token = runtime_token(&f).await;
    let wake = f.arm(&task, "accepted-yield-source").unwrap();
    let now = crate::session::unix_epoch_ms();
    let deadline = now + 60_000;
    let yield_args = serde_json::json!({
        "registration_ids": [wake.registration_id],
        "inbox_cursor": 0,
        "deadline_ms": deadline,
        "reason": "Wait for the first result"
    });
    let result = call(
        &f,
        &token,
        "chariox.events.yield",
        &task,
        yield_args.clone(),
    )
    .await
    .unwrap();
    assert_eq!(result.payload["yield_requested"], true);

    f.runtime
        .wake_sink(KIND)
        .occurred(f.occurrence(&wake.id, "result", Some(WakeEnd::Succeeded)))
        .unwrap();
    let events = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
    let event = events.iter().find(|e| e.source_id == wake.id).unwrap();
    let inbox = call(
        &f,
        &token,
        "chariox.events.inbox",
        &task,
        serde_json::json!({"sequence": event.sequence}),
    )
    .await
    .expect("inbox remains available until the native turn ends");
    assert_eq!(inbox.payload["events"][0]["source"]["id"], wake.id);
    assert_eq!(inbox.payload["events"][0]["payload"], event.payload);

    let ack = call(
        &f,
        &token,
        "chariox.events.ack",
        &task,
        serde_json::json!({"sequence": event.sequence, "handled": true}),
    )
    .await
    .unwrap();
    assert_eq!(ack.payload["acknowledged"], event.sequence);
    let events = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
    assert_eq!(
        events
            .iter()
            .find(|e| e.sequence == event.sequence)
            .unwrap()
            .state,
        "handled"
    );

    let mut second_yield = yield_args;
    second_yield["reason"] = "Replace the first wait".into();
    second_yield["deadline_ms"] = (deadline + 60_000).into();
    for (tool, args) in [
        ("chariox.events.yield", second_yield),
        (
            "chariox.events.timer",
            serde_json::json!({"label": "Unrequested timer", "delay_ms": 60_000}),
        ),
    ] {
        let refusal = call(&f, &token, tool, &task, args)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            refusal.contains("Wait for the first result"),
            "{tool}: {refusal}"
        );
    }
    let accepted = f.task(&task.task_id).wait.unwrap();
    assert_eq!(accepted.deadline_ms, Some(deadline));
    assert_eq!(accepted.registration_ids, [wake.registration_id]);
    assert_eq!(
        f.store().agent_registrations(&task.task_id).unwrap().len(),
        1
    );

    let Outcome::Settled {
        task: settled,
        correction,
    } = f
        .store()
        .agent_lifecycle(Operation::Settle {
            room: f.room.clone(),
            agent: f.agent.clone(),
            prompt: task.prompt_id,
            run: f.run.clone(),
            has_answer: false,
            cancelled: false,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap()
    else {
        panic!("expected native settlement");
    };
    assert_eq!(settled.state, ExecutionState::Done);
    assert!(settled.wait.is_none() && !correction);
    assert_eq!(f.task(&task.task_id).state, ExecutionState::Done);
}
