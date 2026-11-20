//! A child's end is told once, by its `child_finished`; the receipt of the same end that follows
//! arrives already handled. When the parent handled the `child_finished` before that receipt, the
//! receipt still settles the delegate obligation it reports, so the parent's task can end.
use super::*;
use tests::Fixture;

#[test]
fn child_end_recovery_returns_only_undelivered_delegation_results() {
    let f = Fixture::new();
    f.store.agent_ledger_sql_for_test(
        "INSERT INTO agent_tasks(task_id,room_id,agent_id,prompt_id,payload)
           VALUES ('parent-task','room','parent','parent-prompt',
                   '{\"state\":\"done\",\"obligations\":[{}]}');
         WITH delegation(sequence) AS (VALUES (1),(2),(3),(4))
         INSERT INTO agent_tasks(task_id,room_id,agent_id,prompt_id,payload)
           SELECT 'agent-event-child-' || sequence,'room','child','prompt-' || sequence,
                  '{\"state\":\"done\"}' FROM delegation;
         INSERT INTO agent_inbox(sequence,room_id,agent_id,source_id,occurrence_id,payload)
           VALUES (1,'room','child','parent','brief-1','{}'),
                  (2,'room','child','parent','brief-2','{}'),
                  (3,'room','child','parent','brief-3','{}'),
                  (4,'room','child','parent','brief-4','{}'),
                  (5,'room','parent','child','task-terminal-agent-event-child-1','{\"state\":\"pending\"}'),
                  (6,'room','parent','child','child-finished-agent-event-child-2','{}'),
                  (7,'room','parent','child','task-terminal-agent-event-child-3','{\"state\":\"handled\"}');
         INSERT INTO agent_delegation_messages(sequence,task_id)
           VALUES (1,'parent-task'),(2,'parent-task'),(3,'parent-task'),(4,'parent-task');
         INSERT INTO agent_source_occurrences(room_id,source_id,occurrence_id,success,public_answer)
           SELECT room_id,task_id,'task-terminal-' || task_id,1,
                  json_object('answer',prompt_id) FROM agent_tasks WHERE agent_id='child';
         INSERT INTO agent_source_occurrences(room_id,source_id,occurrence_id,success)
           VALUES ('room','worker-result-returned:agent-event-child-4','parent',1);",
    );

    let ends = f.store.agent_undelivered_delegate_ends().unwrap();
    assert_eq!(ends.len(), 1);
    assert_eq!(ends[0].task, "agent-event-child-1");
    assert_eq!(ends[0].parent, "parent");
    assert_eq!(
        ends[0].answer,
        Some(serde_json::json!({"answer":"prompt-1"}))
    );
}

#[test]
fn a_receipt_after_the_handled_child_finished_settles_the_delegate() {
    let f = Fixture::new();
    f.begin("p");
    f.apply(Operation::RegisterObligation {
        owner: "owner".into(),
        room: "room".into(),
        agent: "parent".into(),
        prompt: "p".into(),
        run: Some("run".into()),
        id: "delegate".into(),
        kind: "delegate".into(),
        resource: Some("child".into()),
        now: 1,
    });
    f.apply(Operation::DispatchReceipt {
        id: "delegate".into(),
        accepted: true,
        resource: Some("child".into()),
    });
    f.apply(Operation::Begin {
        owner: "owner".into(),
        room: "room".into(),
        agent: "child".into(),
        prompt: "ct".into(),
        run: None,
        now: 2,
    });
    f.apply(Operation::BindDelegate {
        parent_task: "p".into(),
        child_task: "ct".into(),
    });
    // A deadline stops the child: the parent hears child_finished first and handles it at once.
    let Outcome::Event(finished) = f.apply(Operation::Occur(occurrence(
        "room",
        "parent",
        "child",
        "child-finished-ct",
        "child_finished",
        serde_json::json!({"child_task_id": "ct", "success": false, "deadline_reached": true}),
    ))) else {
        panic!("child_finished recorded")
    };
    f.apply(Operation::Ack {
        room: "room".into(),
        agent: "parent".into(),
        sequence: finished.sequence,
        handled: true,
        now: 3,
    });
    // Then the receipt of the child's task end lands, folded into that child_finished.
    f.apply(Operation::SourceOutcome {
        room: "room".into(),
        source: "ct".into(),
        occurrence: "task-terminal-ct".into(),
        success: false,
        public_answer: None,
        now: 4,
    });
    let delegate = f
        .task()
        .obligations
        .into_iter()
        .find(|o| o.id == "delegate")
        .unwrap();
    assert_eq!(delegate.status, "satisfied", "{delegate:?}");
    let Outcome::Settled { task, correction } = f.settle("p", true) else {
        panic!("the turn settles")
    };
    assert!(!correction, "nothing is left for the parent to resolve");
    assert_eq!(task.state, ExecutionState::Done, "{task:?}");
}
