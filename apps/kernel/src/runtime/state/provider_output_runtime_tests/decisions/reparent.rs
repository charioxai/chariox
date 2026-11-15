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

    /// A new agent, under `parent` when given, with its own permission level.
    pub(super) async fn agent_at(
        &self,
        alias: &str,
        parent: Option<&str>,
        level: AgentPermissionLevel,
    ) -> String {
        let mut app = self.app.lock().await;
        let mut request =
            crate::agent::CreateAgentRequest::new(&self.room, "dev-stub").with_alias(alias);
        if let Some(parent) = parent {
            request = request.with_parent_agent_id(parent);
        }
        let agent = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(request)
            .unwrap()
            .id()
            .to_owned();
        crate::test_support::admit_room_test_turn(&mut app, &self.room, &agent);
        app.agents()
            .update_agent_config(&agent, None, Some(Some(level)), None, None)
            .unwrap();
        agent
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
    pub(super) fn panels(&self) -> Vec<(String, Vec<String>)> {
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

    pub(super) fn holder(&self, id: &str) -> DecisionHolder {
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

pub(super) fn cards(ids: &[&str]) -> Vec<String> {
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
            .await
            .unwrap(),
        ChildDecisionStatus::Answered
    );
    assert!(done.try_recv().is_ok());
    let q = t.room_agent("q").await;

    t.place(&t.p, Some(&q)).await;
    assert_eq!(
        t.panels(),
        vec![(
            q.clone(),
            cards(&["reparent-perm-held", "reparent-secret-d"])
        )],
        "both cards follow the subtree to q's panel, the held one too"
    );
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
    assert_eq!(
        t.panels(),
        vec![(t.g.clone(), cards(&["reparent-perm-moved"]))],
        "g holds it and the user sees it in g's panel"
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

#[tokio::test]
async fn a_holder_made_ineligible_by_its_new_parent_hands_the_decision_to_the_user() {
    let t = tree(
        "reparent-ineligible",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let q = t.agent_at("q", None, AgentPermissionLevel::Required).await;
    let _open = t.raise(permission("reparent-perm-ineligible", &t.c)).await;
    assert_eq!(
        t.holder("reparent-perm-ineligible"),
        DecisionHolder::Agent(t.p.clone())
    );
    assert_eq!(
        t.panels(),
        vec![(t.g.clone(), cards(&["reparent-perm-ineligible"]))]
    );

    // p stays above c, but under a Required root it would ask the user itself.
    t.place(&t.p, Some(&q)).await;
    assert_eq!(t.holder("reparent-perm-ineligible"), DecisionHolder::User);
    assert!(t.all_closed(&t.p), "p's event is withdrawn");
    assert!(t.decision_events(&q).is_empty());
    assert_eq!(
        t.panels(),
        vec![(q, cards(&["reparent-perm-ineligible"]))],
        "the user sees the card under the new root"
    );
}

#[tokio::test]
async fn a_subtree_moved_under_a_lower_authority_root_does_not_keep_an_ineligible_holder() {
    let t = tree(
        "reparent-ineligible-subtree",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let q = t.agent_at("q", None, AgentPermissionLevel::Required).await;
    let _open = t.raise(permission("reparent-perm-deep", &t.c)).await;
    assert_eq!(
        t.holder("reparent-perm-deep"),
        DecisionHolder::Agent(t.p.clone())
    );

    t.place(&t.g, Some(&q)).await;
    assert_eq!(t.holder("reparent-perm-deep"), DecisionHolder::User);
    assert!(t.all_closed(&t.p));
    assert_eq!(t.panels(), vec![(q, cards(&["reparent-perm-deep"]))]);
}

#[tokio::test]
async fn an_ineligible_holder_hands_the_decision_to_an_eligible_ancestor() {
    let t = tree(
        "reparent-eligible-ancestor",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let q = t.agent_at("q", None, AgentPermissionLevel::Yolo).await;
    let r = t
        .agent_at("r", Some(&q), AgentPermissionLevel::Required)
        .await;
    let _open = t.raise(permission("reparent-perm-up", &t.c)).await;

    // p now sits under the Required r; q above r is the nearest ancestor that may hold.
    t.place(&t.p, Some(&r)).await;
    assert_eq!(
        t.holder("reparent-perm-up"),
        DecisionHolder::Agent(q.clone())
    );
    assert!(t.all_closed(&t.p), "p's event is withdrawn");
    assert_eq!(t.decision_events(&q).len(), 1, "q is told once");
    assert_eq!(
        t.panels(),
        vec![(q, cards(&["reparent-perm-up"]))],
        "q holds it and the user sees it in q's panel"
    );
}

#[tokio::test]
async fn a_decision_that_returns_to_a_holder_is_announced_again() {
    let t = tree(
        "reparent-return",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let q = t.agent_at("q", None, AgentPermissionLevel::Yolo).await;
    let _open = t.raise(permission("reparent-perm-return", &t.c)).await;
    assert_eq!(t.decision_events(&t.p).len(), 1);

    t.place(&t.c, Some(&q)).await;
    assert_eq!(
        t.holder("reparent-perm-return"),
        DecisionHolder::Agent(q.clone())
    );
    assert!(t.all_closed(&t.p), "p's first event is withdrawn");
    assert_eq!(t.decision_events(&q).len(), 1);

    t.place(&t.c, Some(&t.p)).await;
    assert_eq!(
        t.holder("reparent-perm-return"),
        DecisionHolder::Agent(t.p.clone())
    );
    assert!(t.all_closed(&q), "q's event is withdrawn");
    let events = t.decision_events(&t.p);
    assert_eq!(events.len(), 2, "p hears of the decision again");
    assert_ne!(events[0].occurrence_id, events[1].occurrence_id);
    assert_ne!(events[1].state, "expired", "the new event is not withdrawn");

    // The same tenure announced again is still one event.
    t.runtime
        .announce_child_decision(&t.room, "reparent-perm-return")
        .await
        .unwrap();
    assert_eq!(t.decision_events(&t.p).len(), 2);
}

/// The reparent fix hands on a decision whose holder lost its authority under a new parent
/// chain. The same loss through a permission change leaves the decision with the holder: its
/// answer is refused as `NotHolder`, yet it still holds the decision and the user has no card.
#[tokio::test]
async fn a_holder_lowered_by_a_permission_change_does_not_keep_a_decision_it_may_not_answer() {
    let t = tree(
        "lowered-holder-keeps",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let mut answer = t.raise(permission("perm-lowered-keeps", &t.c)).await;
    assert_eq!(
        t.holder("perm-lowered-keeps"),
        DecisionHolder::Agent(t.p.clone())
    );
    t.app
        .lock()
        .await
        .agents()
        .update_agent_config(
            &t.g,
            None,
            Some(Some(AgentPermissionLevel::Required)),
            None,
            None,
        )
        .unwrap();
    let status = t
        .runtime
        .answer_child_decision(&t.room, &t.p, "perm-lowered-keeps", "allow")
        .await
        .unwrap();
    assert_eq!(status, ChildDecisionStatus::NotHolder);
    assert!(answer.try_recv().is_err(), "the decision is still pending");
    assert_ne!(
        t.holder("perm-lowered-keeps"),
        DecisionHolder::Agent(t.p.clone()),
        "p was told it does not hold the decision but still holds it; user panels: {:?}",
        t.panels()
    );
}

/// Review of #1281: the user lowers the room agent, so the parent holding its child's decision
/// may no longer answer it. The decision moves on at the config change, not when the parent next
/// tries to answer: the user's card is listed in the top parent's panel at once.
#[tokio::test]
async fn lowering_the_holders_authority_hands_the_decision_to_the_user_at_once() {
    let t = tree(
        "lowered-config-update",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let mut answer = t.raise(permission("perm-cfg", &t.c)).await;
    assert_eq!(t.holder("perm-cfg"), DecisionHolder::Agent(t.p.clone()));
    t.runtime
        .update_agent_config(
            &t.room,
            &t.g,
            USER,
            None,
            Some(Some(AgentPermissionLevel::Required)),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(t.holder("perm-cfg"), DecisionHolder::User);
    assert_eq!(t.panels(), vec![(t.g.clone(), cards(&["perm-cfg"]))]);
    assert!(t.all_closed(&t.p), "p's event is withdrawn");
    assert!(t.decision_events(&t.g).is_empty(), "g is not told");
    assert_eq!(t.needs().1, 1);
    assert!(answer.try_recv().is_err(), "the asker waits for the user");
}

/// Review of #1281: each new room agent takes the previous one under it, then the top root is
/// placed under the oldest leaf, beyond any bounded walk. The depth bound refuses the long chain
/// calmly, the cycle is refused, and every agent still reaches a room agent.
#[tokio::test]
async fn a_move_cannot_exceed_the_tree_depth_or_close_a_cycle() {
    let t = tree(
        "reparent-depth",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let mut roots = Vec::new();
    for n in 0..18 {
        roots.push(
            t.agent_at(&format!("r{n}"), None, AgentPermissionLevel::Yolo)
                .await,
        );
    }
    let place = |agent: &str, parent: &str| {
        t.runtime.set_agent_parent(
            USER,
            crate::local::SetAgentParentRequest {
                session_id: t.room.clone(),
                agent_id: agent.into(),
                parent_agent_id: Some(parent.into()),
            },
        )
    };
    let mut refused = Vec::new();
    for pair in roots.windows(2) {
        if let Err(error) = place(&pair[0], &pair[1]).await {
            refused.push(error.to_string());
        }
    }
    assert!(
        !refused.is_empty(),
        "a chain deeper than the limit is refused"
    );
    assert!(
        refused
            .iter()
            .all(|e| e.contains("levels below a room agent")),
        "{refused:?}"
    );
    assert!(place(&roots[17], &roots[0]).await.is_err());
    let agents = t.runtime.owned.agent_store.get_session_agents(&t.room);
    for agent in &agents {
        let above = crate::agent::authority_limit::ancestors(agent, &agents);
        assert!(above.len() <= 4, "every agent reaches a room agent");
        assert!(above
            .last()
            .is_none_or(|top| top.parent_agent_id().is_none()));
    }
}

/// Pre-review: a room never has more room agents than its `max_agents`. A sub-agent is not
/// counted while it is one (A4), so placing it as a room agent must be held to the same limit a
/// new room agent or an unarchived one is.
#[tokio::test]
async fn promoting_a_sub_agent_to_a_room_agent_is_held_to_max_agents() {
    let worktree = crate::test_support::TestWorktree::new("promote-max-agents");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    config.user_config.workflow.session_default_max_agents = Some(1);
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let (session, g) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    crate::test_support::admit_room_test_turn(&mut app, session.id(), g.id());
    let s = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("s")
                .with_parent_agent_id(g.id()),
        )
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let promoted = runtime
        .set_agent_parent(
            USER,
            crate::local::SetAgentParentRequest {
                session_id: session.id().into(),
                agent_id: s.id().into(),
                parent_agent_id: None,
            },
        )
        .await;
    let room_agents = app
        .lock()
        .await
        .agents()
        .get_session_agents(session.id())
        .iter()
        .filter(|agent| !agent.archived() && agent.parent_agent_id().is_none())
        .count();
    assert!(
        promoted.is_err(),
        "a room at max_agents = 1 gets a second room agent ({room_agents} now): {promoted:?}"
    );
}

impl Tree {
    /// The room default set to `level`, through the client's session config path.
    async fn set_room_default(&self, attachment: &str, level: &str) {
        self.runtime
            .update_session_config(
                &self.room,
                attachment,
                std::collections::BTreeMap::from([("agents.permissions".into(), level.into())]),
                false,
            )
            .await
            .unwrap();
    }

    /// p holds c's open question and parks on it; p follows the room default (Yolo), c keeps its
    /// own Yolo, and g keeps its own Yolo only when `g_keeps_override`. Then the room default is
    /// lowered to Required.
    async fn lower_the_room_default_under_a_held_question(
        &self,
        id: &str,
        g_keeps_override: bool,
    ) -> tokio::sync::oneshot::Receiver<PendingInteractionResolution> {
        let attachment = {
            let mut app = self.app.lock().await;
            app.agents()
                .update_agent_config(
                    &self.c,
                    None,
                    Some(Some(AgentPermissionLevel::Yolo)),
                    None,
                    None,
                )
                .unwrap();
            crate::app::KernelSessionService::new(&mut app)
                .attach(crate::attachment::AttachRequest::new(
                    &self.room,
                    "config-client",
                    crate::attachment::ClientCapabilityLevel::FullTerminal,
                ))
                .unwrap()
                .id()
                .to_owned()
        };
        self.set_room_default(&attachment, "yolo").await;
        {
            let app = self.app.lock().await;
            let followers: &[&str] = if g_keeps_override {
                &[&self.p]
            } else {
                &[&self.p, &self.g]
            };
            for agent in followers {
                app.agents()
                    .update_agent_config(agent, None, Some(None), None, None)
                    .unwrap();
            }
        }
        let answer = self.raise(permission(id, &self.c)).await;
        assert_eq!(self.holder(id), DecisionHolder::Agent(self.p.clone()));
        // The holder parks on its child, so the room default may change under it; g parks too
        // when it follows the room default, since a running turn refuses the change.
        self.park(&self.p).await;
        if !g_keeps_override {
            self.park(&self.g).await;
        }
        self.set_room_default(&attachment, "required").await;
        answer
    }

    /// `agent` yields and its prompt settles, so it runs no turn.
    async fn park(&self, agent: &str) {
        self.end_turn_yielded(agent).await;
        let run = self
            .runtime
            .owned
            .provider_store
            .get_run_for_agent(&self.room, agent)
            .unwrap();
        self.runtime
            .owned
            .complete_local_prompt_without_advance(&self.room, agent, Some(run.id()))
            .unwrap();
    }
}

/// Pre-review: the room default lowers every agent without its own level, as lowering one agent
/// does. With g following it too (and parked), no ancestor may hold the question: it is the
/// user's at once.
#[tokio::test]
async fn lowering_the_room_default_hands_the_decision_to_the_user_at_once() {
    let t = tree(
        "lowered-room-default-user",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let mut answer = t
        .lower_the_room_default_under_a_held_question("perm-room", false)
        .await;
    assert_eq!(
        t.holder("perm-room"),
        DecisionHolder::User,
        "a holder the room default lowered to Required keeps the decision"
    );
    assert_eq!(t.panels(), vec![(t.g.clone(), cards(&["perm-room"]))]);
    assert_eq!(t.needs().1, 1, "the user's decision is counted");
    assert!(t.all_closed(&t.p), "the former holder's event is withdrawn");
    assert!(t.decision_events(&t.g).is_empty(), "g may not hold it");
    assert!(answer.try_recv().is_err(), "the asker waits for the user");
    t.runtime
        .owned
        .resolve_runtime_interaction(&t.room, "perm-room", "allow", None, None, false)
        .unwrap();
    assert_eq!(
        answer.try_recv().unwrap().choice_id.as_deref(),
        Some("allow"),
        "the user's one answer reaches the asker"
    );
    assert_eq!(
        t.runtime
            .answer_child_decision(&t.room, &t.p, "perm-room", "deny")
            .await
            .unwrap(),
        ChildDecisionStatus::SettledByUser,
        "the lowered former holder cannot answer as well"
    );
}

/// Same, but g keeps its own Yolo: g is the next ancestor that may hold the question, so it gets
/// the question with one event, and p's is withdrawn.
#[tokio::test]
async fn lowering_the_room_default_hands_the_decision_to_an_ancestor_with_its_own_level() {
    let t = tree(
        "lowered-room-default-g",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let mut answer = t
        .lower_the_room_default_under_a_held_question("perm-room-g", true)
        .await;
    assert_eq!(
        t.holder("perm-room-g"),
        DecisionHolder::Agent(t.g.clone()),
        "a holder the room default lowered to Required keeps the decision"
    );
    let events = t.decision_events(&t.g);
    assert_eq!(events.len(), 1, "g is told once: {events:?}");
    assert_eq!(events[0].payload["interaction_id"], "perm-room-g");
    assert!(t.all_closed(&t.p), "the former holder's event is withdrawn");
    assert_eq!(t.needs().1, 0, "nothing waits for the user");
    assert!(answer.try_recv().is_err(), "the asker waits for g");
}
