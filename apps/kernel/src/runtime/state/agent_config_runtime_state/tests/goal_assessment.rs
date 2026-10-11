// MP-08 / MP-09 / MP-10 / MP-11 A12-S02: actual native context includes safe actionable receipts.
use super::*;
#[tokio::test]
async fn a12_assessment_context_exposes_unhandled_correlated_receipts() {
    use crate::durable_state::agent_lifecycle::{Operation, Registration};
    let (_app, runtime, room, agent) = agent_config_runtime().await;
    let store = &runtime.owned.durable_state_store;
    let apply = |op| store.agent_lifecycle(op).unwrap();
    apply(Operation::Begin {
        owner: "owner".into(),
        room: room.clone(),
        agent: agent.clone(),
        prompt: "goal-work".into(),
        run: Some("run".into()),
        now: 1,
    });
    apply(Operation::StartGoal {
        task: "goal-work".into(),
        owner: "owner".into(),
        objective: "Verify delegate artifacts".into(),
        minutes: 1,
        now: 1,
    });
    apply(Operation::RegisterObligation {
        owner: "owner".into(),
        room: room.clone(),
        agent: agent.clone(),
        prompt: "goal-work".into(),
        run: Some("run".into()),
        id: "delegate".into(),
        kind: "delegate".into(),
        resource: Some("child".into()),
        now: 1,
    });
    apply(Operation::Subscribe {
        task: "goal-work".into(),
        prompt: "goal-work".into(),
        registration: Registration {
            id: "completion".into(),
            task_id: "goal-work".into(),
            source_id: "child".into(),
            obligation_id: Some("delegate".into()),
            source_cursor: 0,
            live: true,
        },
    });
    apply(Operation::SourceOutcome {
        room: room.clone(),
        source: "child".into(),
        occurrence: "finished".into(),
        success: true,
        public_answer: Some(serde_json::json!({"private_body":"DO_NOT_INCLUDE_RAW_RECEIPT_BODY"})),
        now: 2,
    });
    let events = store
        .agent_work_inbox(&room, &agent, 0, Some("goal-work"))
        .unwrap();
    assert_eq!(events.len(), 1);
    apply(Operation::Attempt {
        room: room.clone(),
        agent: agent.clone(),
        sequence: events[0].sequence,
        prompt: "event-turn".into(),
        target: Some("goal-work".into()),
        work: Some("goal-work".into()),
        run: Some("run".into()),
        now: 2,
    });
    apply(Operation::Receipt {
        room: room.clone(),
        agent: agent.clone(),
        sequence: events[0].sequence,
        state: "accepted".into(),
        now: 2,
    });
    apply(Operation::Settle {
        room: room.clone(),
        agent: agent.clone(),
        prompt: "goal-work".into(),
        run: "run".into(),
        has_answer: true,
        cancelled: false,
        now: 3,
    });
    let task = store
        .agent_tasks(Some(&room), Some(&agent))
        .unwrap()
        .into_iter()
        .find(|t| t.goal.is_some())
        .unwrap();
    let context = runtime
        .owned
        .goal_continuation_context(&task)
        .unwrap()
        .unwrap();
    assert!(
        context.contains("\"unhandled_receipts\""),
        "native assessment needs pending public receipt refs after sources become non-live"
    );
    assert!(context.contains("\"state\":\"accepted\""));
    assert!(context.contains("\"sequence\":"));
    assert!(
        !context.contains("DO_NOT_INCLUDE_RAW_RECEIPT_BODY"),
        "only safe receipt metadata/references may enter assessment context"
    );
    assert!(
        context.contains("handled:true"),
        "continue normal handling/verification before owner-blocking on terminal sources"
    );
}
