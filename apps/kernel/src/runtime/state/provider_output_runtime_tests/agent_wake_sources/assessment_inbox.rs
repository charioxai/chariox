//! A completion check has no work tools; urgent events wait for an ordinary turn.
use super::*;
use crate::durable_state::agent_lifecycle::occurrence;

#[tokio::test]
async fn a12_completion_check_keeps_urgent_inbox_pending() {
    let f = fixture("a12-assessment-urgent-inbox", true).await;
    let session = f.runtime.owned.session_store.get_session(&f.room).unwrap();
    let check = crate::session::PromptQueueItem::new(
        "goal-check-inbox",
        "assessment-client",
        &f.agent,
        "Check the original objective",
        crate::session::PromptStatus::Running,
    )
    .with_prompt_origin(crate::session::PromptOrigin::CompletionCheck);
    let owner = &f.runtime.owned.prompt_state_owner;
    let original = owner.active_prompt_for_agent(&session, &f.agent).unwrap();
    assert!(owner.replace_active_prompt_if_matches(&session, &f.agent, &original, check.clone(),));
    assert_eq!(
        owner
            .active_prompt_for_agent(&session, &f.agent)
            .unwrap()
            .prompt_origin(),
        crate::session::PromptOrigin::CompletionCheck,
    );
    f.begin(check.id());
    let mut event = occurrence(
        &f.room,
        &f.agent,
        "child",
        "child-needs-permission",
        "message",
        serde_json::json!({"message":"Handle the child's permission"}),
    );
    event.urgent = true;
    let Outcome::Event(event) = f.store().agent_lifecycle(Operation::Occur(event)).unwrap() else {
        panic!("inbox occurrence must be persisted");
    };
    f.runtime
        .deliver_agent_inbox(&f.room, &f.agent)
        .await
        .unwrap();
    let pending = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
    assert_eq!(pending[0].sequence, event.sequence);
    assert_eq!(pending[0].state, "pending");
    assert!(
        pending[0].attempted_at_ms.is_none(),
        "an urgent inbox event must not enter the tool-free completion check"
    );
    assert!(
        f.runtime
            .prepare_local_active_prompt_steer_dispatch(&f.room, &check)
            .unwrap()
            .is_none(),
        "steer preparation must also fence a check selected after inbox discovery"
    );

    let work = check
        .clone()
        .with_prompt_origin(crate::session::PromptOrigin::Chariox);
    assert!(owner.replace_active_prompt_if_matches(&session, &f.agent, &check, work));
    f.runtime
        .deliver_agent_inbox(&f.room, &f.agent)
        .await
        .unwrap();
    let delivered = f.store().agent_inbox(&f.room, &f.agent, 0).unwrap();
    assert_eq!(delivered[0].sequence, event.sequence);
    assert!(
        delivered[0].attempted_at_ms.is_some(),
        "the same event must become deliverable when ordinary work resumes"
    );
}
