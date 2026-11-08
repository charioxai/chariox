//! MP-08 / MP-09 / MP-10 / MP-11: an agent's inbox keeps its closed rows for good, so finding the
//! event of one decision it holds must not read the inbox from its first row.
use super::*;
use crate::durable_state::agent_lifecycle::{occurrence, INBOX_ROWS_READ};

#[tokio::test]
async fn finding_a_held_decisions_event_does_not_read_the_whole_inbox() {
    let t = tree(
        "decision-inbox-reads",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    t.raise(permission("perm-reads", &t.c)).await;
    assert_eq!(t.decision_events(&t.p).len(), 1, "the parent holds it");
    let store = &t.runtime.owned.durable_state_store;
    for index in 0..2000 {
        let Outcome::Event(event) = store
            .agent_lifecycle(Operation::Occur(occurrence(
                &t.room,
                &t.p,
                "sibling",
                &format!("done-{index}"),
                "child_finished",
                serde_json::json!({"n": index}),
            )))
            .unwrap()
        else {
            panic!("event")
        };
        store
            .agent_lifecycle(Operation::Ack {
                room: t.room.clone(),
                agent: t.p.clone(),
                sequence: event.sequence,
                handled: true,
                now: 1,
            })
            .unwrap();
    }
    INBOX_ROWS_READ.with(|n| n.set(0));
    t.runtime
        .move_seen_decisions_up(&t.room, &t.p)
        .await
        .unwrap();
    let read = INBOX_ROWS_READ.with(|n| n.get());
    assert!(
        read <= 16,
        "finding one decision's event read {read} rows of a 2000-row inbox"
    );
}
