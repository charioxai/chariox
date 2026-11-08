//! A decision a holder passed keeps its new holder until that holder hands it on: whatever wakes
//! a former holder later, it is never told again and never takes the decision back.
use super::reparent::cards;
use super::*;
use crate::runtime::state::decision_holder::DecisionHolder;

/// Live (uiqa r5 hk-5): the decision was passed up to the user, then its first holder woke for
/// another child's `child_finished`. The card stays the user's, in the top panel.
#[tokio::test]
async fn a_passed_decision_stays_passed_when_its_first_holder_wakes_for_a_sibling() {
    let t = tree(
        "decision-pass-wake",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let sibling = t
        .agent_at("s", Some(&t.p), AgentPermissionLevel::Yolo)
        .await;
    let mut answer = t.raise(permission("perm-pw", &t.c)).await;
    for holder in [&t.p, &t.g] {
        assert_eq!(
            t.runtime
                .pass_child_decision(&t.room, holder, "perm-pw")
                .await
                .unwrap(),
            ChildDecisionStatus::Passed
        );
    }
    assert_eq!(t.holder("perm-pw"), DecisionHolder::User);
    assert_eq!(t.panels(), [(t.g.clone(), cards(&["perm-pw"]))]);

    t.end_turn_as(&sibling, false).await;

    assert_eq!(
        t.holder("perm-pw"),
        DecisionHolder::User,
        "the wake gave the decision back"
    );
    assert_eq!(
        t.panels(),
        [(t.g.clone(), cards(&["perm-pw"]))],
        "the user's card is still in the top panel"
    );
    assert_eq!(t.needs().1, 1, "the user's decision is still counted");
    for former in [&t.p, &t.g] {
        let events = t.decision_events(former);
        assert_eq!(events.len(), 1, "a former holder is never told again");
        assert!(t.all_closed(former), "{events:?}");
    }
    assert!(
        answer.try_recv().is_err(),
        "the asker still waits, for the user"
    );
}
