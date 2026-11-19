//! A decision a parent holds reaches it in its wake prompt with what it needs to answer, and
//! stays open until it is answered: the wake settles nothing for it.
use super::*;

#[tokio::test]
async fn a_held_decision_is_quoted_in_the_parents_wake_and_stays_open() {
    let t = tree(
        "decision-wake-text",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    t.raise(permission("perm-wake", &t.c)).await;
    let event = t.decision_events(&t.p).remove(0);
    let prompt_id = event
        .prompt_id
        .clone()
        .expect("steered into the parent's turn");
    let entries = t
        .runtime
        .owned
        .operational_history_store
        .load_session_history_entries(&t.room, Some(&t.p))
        .unwrap();
    let wake = entries
        .iter()
        .find(|entry| {
            entry.merge_key.as_deref()
                == Some(crate::history::steering_prompt_merge_key(&prompt_id).as_str())
        })
        .unwrap_or_else(|| panic!("no steered wake row: {entries:?}"));
    for expected in [
        "perm-wake",
        "Allow command?",
        "Allow",
        "stays open until you answer",
    ] {
        assert!(
            wake.text.contains(expected),
            "missing {expected}: {}",
            wake.text
        );
    }
    assert!(!wake.text.contains("chariox.events.inbox"), "{}", wake.text);
    assert!(
        !matches!(event.state.as_str(), "acknowledged" | "handled"),
        "an unanswered decision stays open: {event:?}"
    );
    let now = t.decision_events(&t.p).remove(0);
    assert!(
        !matches!(now.state.as_str(), "acknowledged" | "handled"),
        "an unanswered decision stays open: {now:?}"
    );
}
