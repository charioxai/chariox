//! A decision a parent agent answers leaves the same `interaction.answered` record a person's
//! answer does, and it names the parent.
use super::*;

#[tokio::test]
async fn a_parents_answer_is_recorded_with_the_parent_named() {
    let t = tree(
        "decision-audit",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let _answer = t.raise(permission("perm-audit", &t.c)).await;
    let status = t
        .runtime
        .answer_child_decision(&t.room, &t.p, "perm-audit", "allow")
        .await
        .unwrap();
    assert_eq!(status, ChildDecisionStatus::Answered);
    let audit = t
        .runtime
        .owned
        .durable_state_store
        .load_subject_events_by_kind("perm-audit", "interaction.answered", 10)
        .unwrap();
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(audit[0].payload["answered_by_agent"], t.p);
    assert_eq!(audit[0].payload["child_agent_id"], t.c);
    assert_eq!(audit[0].payload["choice_id"], "allow");
    assert_eq!(audit[0].payload["session_id"], t.room);
}

#[tokio::test]
async fn a_refused_parent_answer_leaves_no_record() {
    let t = tree(
        "decision-audit-refused",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let _answer = t.raise(permission("perm-audit-refused", &t.c)).await;
    let status = t
        .runtime
        .answer_child_decision(&t.room, &t.g, "perm-audit-refused", "allow")
        .await
        .unwrap();
    assert_eq!(status, ChildDecisionStatus::NotHolder);
    let audit = t
        .runtime
        .owned
        .durable_state_store
        .load_subject_events_by_kind("perm-audit-refused", "interaction.answered", 10)
        .unwrap();
    assert!(audit.is_empty(), "{audit:?}");
}
