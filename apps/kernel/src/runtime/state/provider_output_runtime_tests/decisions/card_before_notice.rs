//! Row 717: a decision card that registered is reported as created. Telling its holder is a notice
//! that can fail (the holder's turn cannot start yet) without the card being refused.
use super::*;

/// The same Room as `tree`, but the holder `p` has a provider run and no turn: a decision the agent below
/// it raises must START a turn for the holder to tell it, which is what the retained-publication barrier
/// refuses (a holder that is mid-turn is steered instead, and never reaches the barrier).
async fn tree_with_idle_holder(label: &str) -> Tree {
    let worktree = crate::test_support::TestWorktree::new(label);
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, g) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    crate::test_support::admit_room_test_turn(&mut app, session.id(), g.id());
    let p = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("p")
                .with_parent_agent_id(g.id()),
        )
        .unwrap();
    // A child is spawned during its parent's admitted turn ...
    crate::test_support::admit_room_test_turn(&mut app, session.id(), p.id());
    let c = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("c")
                .with_parent_agent_id(p.id()),
        )
        .unwrap();
    crate::test_support::admit_room_test_turn(&mut app, session.id(), c.id());
    // ... and the parent's turn then ends: the holder keeps its provider run and has no turn.
    app.complete_active_prompt(session.id(), p.id(), None)
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    Tree {
        runtime,
        app,
        room: session.id().into(),
        g: g.id().into(),
        p: p.id().into(),
        c: c.id().into(),
        _worktree: worktree,
    }
}

/// The holder's turn cannot start while a retained publication kernel has not validated its runtime
/// preparation. The card of the agent below it is still created, listed and answerable, and the
/// holder's child-decision event stays stored for the next delivery.
#[tokio::test]
async fn a_card_is_created_when_its_holder_cannot_be_told_yet() {
    let mut t = tree_with_idle_holder("card-before-notice").await;
    t.runtime.owned.publication_activation =
        Arc::new(crate::runtime::state::publication_activation::PublicationActivation::new(true));
    assert!(
        t.runtime.owned.require_publication_activation().is_err(),
        "the barrier is up: no holder turn starts"
    );
    let id = "machine-acquisition-row717";
    let card = RuntimeInteraction::for_kernel_operation(
        id,
        id,
        "Acquire a machine",
        "Acquire a build machine?",
        vec![
            RuntimeInteractionChoice::new("approve", "Approve", "approve", None),
            RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
        ],
    )
    .with_related_agent_id(t.c.clone());
    let owner = t.session().owner_user_id().to_owned();
    let _answer = t
        .runtime
        .create_kernel_operation_interaction(
            crate::runtime::state::CardKind::NativePermissionInput,
            &t.room,
            &owner,
            card,
        )
        .await
        .expect("a registered card is a created card, whether or not its holder could be told");
    assert!(
        t.session()
            .active_interactions()
            .iter()
            .any(|card| card.id() == id),
        "the card is listed"
    );
    assert!(
        !t.decision_events(&t.p).is_empty(),
        "the holder's child-decision event is stored for its next delivery"
    );
    // The notice is not lost: once the holder's turn can start, its NEXT delivery, through the real
    // delivery path, takes the stored event.
    let before = t.decision_events(&t.p);
    assert_eq!(before.len(), 1, "{before:?}");
    t.runtime.owned.publication_activation =
        Arc::new(crate::runtime::state::publication_activation::PublicationActivation::new(false));
    assert!(t.runtime.owned.require_publication_activation().is_ok());
    // The refused start left a retry record 30 s ahead; make it due (the existing test-ledger helper,
    // as `agent_start_retry_tests` does) instead of sleeping.
    t.runtime
        .owned
        .durable_state_store
        .agent_ledger_sql_for_test(&format!(
            "UPDATE agent_inbox_start_failures SET retry_at_ms=1 WHERE sequence={}",
            before[0].sequence
        ));
    t.runtime
        .deliver_agent_inbox_for_test(&t.room, &t.p)
        .await
        .expect("the delivery that the barrier refused now runs");
    let after = t.decision_events(&t.p);
    assert_eq!(after.len(), 1, "{after:?}");
    assert_ne!(
        after[0].state, before[0].state,
        "the holder's next delivery took the stored event: {before:?} -> {after:?}"
    );
    assert_ne!(after[0].state, "expired", "{after:?}");
    let status = t
        .runtime
        .answer_owner_room_card(&t.g, &t.room, id, "approve")
        .unwrap();
    assert_eq!(
        status,
        ChildDecisionStatus::Answered,
        "the card answers whether or not its holder was told"
    );
}
