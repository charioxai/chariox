//! The user moves an agent in the tree: the open decisions of the moved subtree move with it.
//! Tree: g (room agent) -> p -> c, plus a second room agent q.
use super::*;
use crate::runtime::state::decision_holder::DecisionHolder;

const USER: &str = crate::session::DEFAULT_LOCAL_USER_ID;

fn secret(id: &str, agent: &str) -> RuntimeInteraction {
    RuntimeInteraction::new(
        id,
        agent,
        RuntimeInteractionKind::Choice,
        RuntimeInteractionLevel::Warning,
        Some("Token".into()),
        "Enter the token",
        vec![RuntimeInteractionChoice::new(
            "cancel", "Cancel", "cancel", None,
        )],
        Some(RuntimeInteractionCustomChoice::secret(
            "secret",
            "Token",
            None,
            Some(1),
            Some(80),
        )),
        None,
        None,
    )
}

impl Tree {
    async fn room_agent(&self, alias: &str) -> String {
        let mut app = self.app.lock().await;
        crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(&self.room, "dev-stub").with_alias(alias),
            )
            .unwrap()
            .id()
            .to_owned()
    }

    async fn place(&self, agent: &str, parent: Option<&str>) {
        self.runtime
            .set_agent_parent(
                USER,
                crate::local::SetAgentParentRequest {
                    session_id: self.room.clone(),
                    agent_id: agent.into(),
                    parent_agent_id: parent.map(str::to_owned),
                },
            )
            .await
            .unwrap();
    }

    /// What the user's client shows: the projected session's room agents, each with the open
    /// cards the shared client lookup (`shown_in_agent_id ?? agent_id`) finds in its panel.
    fn panels(&self) -> Vec<(String, Vec<String>)> {
        let session = self
            .runtime
            .owned
            .session_snapshot(&self.room)
            .unwrap()
            .redacted_for_user(USER);
        session
            .agents()
            .iter()
            .map(|agent| {
                let mut cards = session
                    .active_interactions()
                    .iter()
                    .filter(|card| card.shown_in_agent_id().or(card.agent_id()) == Some(agent.id()))
                    .map(|card| card.id().to_owned())
                    .collect::<Vec<_>>();
                cards.sort();
                (agent.id().to_owned(), cards)
            })
            .filter(|(_, cards)| !cards.is_empty())
            .collect()
    }

    fn holder(&self, id: &str) -> DecisionHolder {
        self.runtime
            .owned
            .pending_interactions
            .write()
            .get(id)
            .unwrap()
            .holder
            .clone()
    }
}

fn cards(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

#[tokio::test]
async fn a_room_agent_placed_under_another_takes_its_open_cards_to_the_new_top_panel() {
    let t = tree(
        "reparent-root",
        AgentPermissionLevel::Required,
        AgentPermissionLevel::Required,
    )
    .await;
    t.raise(secret("reparent-secret-c", &t.c)).await;
    t.raise(permission("reparent-perm-p", &t.p)).await;
    t.raise(permission("reparent-perm-g", &t.g)).await;
    let all = cards(&["reparent-perm-g", "reparent-perm-p", "reparent-secret-c"]);
    assert_eq!(t.panels(), vec![(t.g.clone(), all.clone())]);
    let q = t.room_agent("q").await;
    let follower = Follower::follow(&t);

    t.place(&t.g, Some(&q)).await;
    assert_eq!(
        t.panels(),
        vec![(q.clone(), all.clone())],
        "every card is in q's panel, none is left with g or lost"
    );
    assert_eq!(
        t.session().active_interactions().len(),
        3,
        "no card is duplicated"
    );
    for id in ["reparent-perm-g", "reparent-perm-p", "reparent-secret-c"] {
        assert_eq!(t.holder(id), DecisionHolder::User);
        assert!(
            follower
                .told(&t)
                .iter()
                .any(|(agent, card, _)| agent.as_deref() == Some(q.as_str())
                    && card.as_deref() == Some(id)),
            "a follower hears {id} in q's panel"
        );
    }

    t.place(&t.g, None).await;
    assert_eq!(
        t.panels(),
        vec![(t.g.clone(), all)],
        "detached, g shows them again"
    );
    assert_eq!(t.session().active_interactions().len(), 3);
}

#[tokio::test]
async fn a_deeper_subtree_moves_its_cards_and_keeps_a_holder_still_above_the_asker() {
    let t = tree(
        "reparent-subtree",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let d = crate::app::KernelSessionService::new(&mut *t.app.lock().await)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(&t.room, "dev-stub")
                .with_alias("d")
                .with_parent_agent_id(&t.c),
        )
        .unwrap()
        .id()
        .to_owned();
    t.raise(permission("reparent-perm-held", &t.c)).await;
    t.raise(secret("reparent-secret-d", &d)).await;
    let mut done = t.raise(permission("reparent-perm-done", &t.p)).await;
    assert_eq!(
        t.holder("reparent-perm-held"),
        DecisionHolder::Agent(t.p.clone())
    );
    assert_eq!(
        t.runtime
            .answer_child_decision(&t.room, &t.g, "reparent-perm-done", "allow")
            .unwrap(),
        ChildDecisionStatus::Answered
    );
    assert!(done.try_recv().is_ok());
    let q = t.room_agent("q").await;

    t.place(&t.p, Some(&q)).await;
    assert_eq!(t.panels(), vec![(q.clone(), cards(&["reparent-secret-d"]))]);
    assert_eq!(
        t.holder("reparent-perm-held"),
        DecisionHolder::Agent(t.p.clone()),
        "p is still above c"
    );
    assert_eq!(t.decision_events(&t.p).len(), 1);
    assert!(!t.all_closed(&t.p), "p's event stays open");
    assert!(t.decision_events(&q).is_empty());
    assert!(
        !t.runtime
            .owned
            .pending_interactions
            .write()
            .contains_key("reparent-perm-done"),
        "the answered one is untouched"
    );
    assert_eq!(
        t.decision_events(&t.g).len(),
        1,
        "g was told once, for the answered one"
    );
}

#[tokio::test]
async fn a_holder_no_longer_above_the_asker_hands_the_decision_on() {
    let t = tree(
        "reparent-holder",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let mut open = t.raise(permission("reparent-perm-moved", &t.c)).await;
    assert_eq!(
        t.holder("reparent-perm-moved"),
        DecisionHolder::Agent(t.p.clone())
    );

    // c moves from p to g: p is no longer above c, g is.
    t.place(&t.c, Some(&t.g)).await;
    assert_eq!(
        t.holder("reparent-perm-moved"),
        DecisionHolder::Agent(t.g.clone())
    );
    assert!(t.all_closed(&t.p), "p's event is withdrawn");
    assert_eq!(t.decision_events(&t.g).len(), 1, "g is told once");
    assert!(
        t.panels().is_empty(),
        "an agent holds it, the user sees nothing"
    );

    // c detached: no agent is above it, so the user holds it in c's own panel.
    t.place(&t.c, None).await;
    assert_eq!(t.holder("reparent-perm-moved"), DecisionHolder::User);
    assert!(t.all_closed(&t.g), "g's event is withdrawn");
    assert_eq!(
        t.panels(),
        vec![(t.c.clone(), cards(&["reparent-perm-moved"]))]
    );
    assert_eq!(t.session().active_interactions().len(), 1);
    assert!(open.try_recv().is_err(), "still waiting for an answer");
}
