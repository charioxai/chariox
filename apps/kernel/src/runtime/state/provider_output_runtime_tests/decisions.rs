//! One decision is one card: a sub-agent's decision goes to the nearest parent that may answer
//! it and otherwise reaches the user as the same interaction, listed with the top parent only.
//! Tree: g (room agent) -> p -> c. Every test raises through `create_runtime_interaction`.
use super::*;
use crate::durable_state::agent_lifecycle::{InboxEvent, Operation, Outcome};
use crate::provider::AgentPermissionLevel;
use crate::runtime::state::decision_holder::{user_only, UserOnly};
use crate::session::{
    RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionCustomChoice,
    RuntimeInteractionKind, RuntimeInteractionLevel,
};

struct Tree {
    runtime: KernelRuntimeState,
    app: Arc<Mutex<DaemonApp>>,
    room: String,
    g: String,
    p: String,
    c: String,
    /// Keeps the room's working folder for agents spawned later.
    _worktree: crate::test_support::TestWorktree,
}

async fn tree(label: &str, g_level: AgentPermissionLevel, p_level: AgentPermissionLevel) -> Tree {
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
    crate::test_support::admit_room_test_turn(&mut app, session.id(), p.id());
    let c = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(session.id(), "dev-stub")
                .with_alias("c")
                .with_parent_agent_id(p.id()),
        )
        .unwrap();
    crate::test_support::admit_room_test_turn(&mut app, session.id(), c.id());
    for (agent, level) in [(g.id(), g_level), (p.id(), p_level)] {
        app.agents()
            .update_agent_config(agent, None, Some(Some(level)), None, None)
            .unwrap();
    }
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

fn permission(id: &str, agent: &str) -> RuntimeInteraction {
    RuntimeInteraction::new(
        id,
        agent,
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Warning,
        Some("Allow command?".into()),
        "Run `ls`?",
        vec![
            RuntimeInteractionChoice::new("allow", "Allow", "allow", None),
            RuntimeInteractionChoice::new("deny", "Deny", "deny", None),
        ],
        None,
        None,
        None,
    )
}

impl Tree {
    async fn raise(
        &self,
        interaction: RuntimeInteraction,
    ) -> tokio::sync::oneshot::Receiver<PendingInteractionResolution> {
        self.runtime
            .create_runtime_interaction(&self.room, interaction)
            .await
            .unwrap()
    }

    fn decision_events(&self, agent: &str) -> Vec<InboxEvent> {
        self.runtime
            .owned
            .durable_state_store
            .agent_inbox(&self.room, agent, 0)
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == "child_decision")
            .collect()
    }

    /// A delivered event is closed as handled by the steer itself; a withdrawn one expires.
    fn all_closed(&self, agent: &str) -> bool {
        self.decision_events(agent)
            .iter()
            .all(|event| matches!(event.state.as_str(), "expired" | "handled"))
    }

    fn session(&self) -> crate::session::RuntimeSession {
        self.runtime
            .owned
            .session_store
            .get_session(&self.room)
            .unwrap()
    }

    /// Settles `agent`'s admitted turn in the store: cancelled, or done with an answer.
    fn settle_turn(
        &self,
        agent: &str,
        has_answer: bool,
        cancelled: bool,
    ) -> (
        (
            crate::durable_state::agent_lifecycle::AgentTaskExecution,
            bool,
        ),
        crate::session::PromptQueueItem,
    ) {
        let session = self.session();
        let prompt = session.active_prompt_for_agent(agent).unwrap().clone();
        let run = self
            .runtime
            .owned
            .provider_store
            .get_run_for_agent(&self.room, agent)
            .unwrap();
        let store = &self.runtime.owned.durable_state_store;
        store
            .agent_lifecycle(Operation::Begin {
                owner: "local".into(),
                room: self.room.clone(),
                agent: agent.into(),
                prompt: prompt.id().into(),
                run: Some(run.id().into()),
                now: 1,
            })
            .unwrap();
        let Outcome::Settled { task, correction } = store
            .agent_lifecycle(Operation::Settle {
                room: self.room.clone(),
                agent: agent.into(),
                prompt: prompt.id().into(),
                run: run.id().into(),
                has_answer,
                cancelled,
                now: 2,
            })
            .unwrap()
        else {
            panic!("settled")
        };
        ((task, correction), prompt)
    }

    /// Settles `agent`'s admitted turn through the real settlement path.
    async fn end_turn_as(&self, agent: &str, cancelled: bool) {
        let (settlement, prompt) = self.settle_turn(agent, !cancelled, cancelled);
        self.runtime
            .finish_agent_task_settlement(Some(settlement), &prompt)
            .await
            .unwrap();
    }

    /// `agent` yields on a live source and ends its admitted turn parked, answering nothing.
    async fn end_turn_yielded(&self, agent: &str) {
        let session = self.session();
        let prompt = session.active_prompt_for_agent(agent).unwrap().clone();
        let run = self
            .runtime
            .owned
            .provider_store
            .get_run_for_agent(&self.room, agent)
            .unwrap();
        let store = &self.runtime.owned.durable_state_store;
        let now = crate::session::unix_epoch_ms();
        store
            .agent_lifecycle(Operation::Begin {
                owner: "local".into(),
                room: self.room.clone(),
                agent: agent.into(),
                prompt: prompt.id().into(),
                run: Some(run.id().into()),
                now,
            })
            .unwrap();
        let task = store
            .agent_tasks(Some(&self.room), Some(agent))
            .unwrap()
            .pop()
            .unwrap();
        // Like `events.yield`: one live registration per open obligation (here the delegation
        // of the child), each on that obligation's completion source.
        let registrations = task
            .obligations
            .iter()
            .filter(|obligation| obligation.requires_resolution())
            .map(
                |obligation| crate::durable_state::agent_lifecycle::Registration {
                    id: format!("{agent}-{}", obligation.id),
                    task_id: task.task_id.clone(),
                    source_id: obligation.completion_source().unwrap().into(),
                    obligation_id: Some(obligation.id.clone()),
                    source_cursor: 0,
                    live: true,
                    peer: false,
                },
            )
            .collect::<Vec<_>>();
        assert!(!registrations.is_empty(), "the holder waits on its child");
        let ids = registrations.iter().map(|r| r.id.clone()).collect();
        for registration in registrations {
            store
                .agent_lifecycle(Operation::Subscribe {
                    task: task.task_id.clone(),
                    prompt: task.task_id.clone(),
                    registration,
                })
                .unwrap();
        }
        store
            .agent_lifecycle(Operation::Yield {
                task: task.task_id.clone(),
                prompt: task.task_id.clone(),
                registrations: ids,
                cursor: 0,
                deadline: Some(now + 7_200_000),
                reason: "waiting for the child".into(),
                now,
            })
            .unwrap();
        let Outcome::Settled { task, correction } = store
            .agent_lifecycle(Operation::Settle {
                room: self.room.clone(),
                agent: agent.into(),
                prompt: prompt.id().into(),
                run: run.id().into(),
                has_answer: false,
                cancelled: false,
                now,
            })
            .unwrap()
        else {
            panic!("settled")
        };
        assert_eq!(
            task.state,
            crate::durable_state::agent_lifecycle::ExecutionState::Waiting
        );
        self.runtime
            .finish_agent_task_settlement(Some((task, correction)), &prompt)
            .await
            .unwrap();
    }

    /// Settles `agent`'s admitted turn as cancelled through the real settlement path.
    async fn end_turn(&self, agent: &str) {
        self.end_turn_as(agent, true).await
    }

    fn present(&self, id: &str) -> (bool, bool) {
        (
            self.session()
                .active_interactions()
                .iter()
                .any(|i| i.id() == id),
            self.runtime
                .owned
                .pending_interactions
                .write()
                .get(id)
                .is_some(),
        )
    }
}

use crate::runtime::state::ChildDecisionStatus;

#[tokio::test]
async fn a_delegable_permission_goes_to_the_nearest_parent_that_may_answer() {
    let t = tree(
        "decision-parent",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let mut answer = t.raise(permission("perm-1", &t.c)).await;
    let events = t.decision_events(&t.p);
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(events[0].urgent);
    assert_eq!(events[0].payload["interaction_id"], "perm-1");
    assert_eq!(events[0].payload["needs_answer"], true);
    assert!(
        t.decision_events(&t.g).is_empty(),
        "only the holder is told"
    );
    let session = t.session();
    let card = session
        .active_interactions()
        .iter()
        .find(|i| i.id() == "perm-1")
        .unwrap();
    assert_eq!(
        card.shown_in_agent_id(),
        None,
        "no user card while an agent holds it"
    );
    assert!(answer.try_recv().is_err(), "nobody has answered yet");
}

#[tokio::test]
async fn a_required_parent_is_skipped() {
    let t = tree(
        "decision-required",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Required,
    )
    .await;
    t.raise(permission("perm-2", &t.c)).await;
    assert!(
        t.decision_events(&t.p).is_empty(),
        "a Required parent would ask the user itself"
    );
    assert_eq!(t.decision_events(&t.g).len(), 1);
}

#[tokio::test]
async fn the_user_holds_it_as_the_same_interaction_listed_with_the_top_parent_only() {
    let t = tree(
        "decision-user",
        AgentPermissionLevel::Required,
        AgentPermissionLevel::Required,
    )
    .await;
    t.raise(permission("perm-3", &t.c)).await;
    assert!(t.decision_events(&t.p).is_empty() && t.decision_events(&t.g).is_empty());
    let session = t.session();
    assert_eq!(
        session
            .active_interactions()
            .iter()
            .filter(|i| i.id() == "perm-3")
            .count(),
        1,
        "one object"
    );
    let card = session
        .active_interactions()
        .iter()
        .find(|i| i.id() == "perm-3")
        .unwrap();
    assert_eq!(
        card.agent_id(),
        Some(t.c.as_str()),
        "the card still belongs to the child"
    );
    assert_eq!(card.shown_in_agent_id(), Some(t.g.as_str()));
    assert!(
        session.interactions_listed_for(&t.c).is_empty(),
        "never in the child's pane"
    );
    assert!(session.interactions_listed_for(&t.p).is_empty());
    assert_eq!(session.interactions_listed_for(&t.g)[0].id(), "perm-3");
}

// UIQA-S4a: an ordinary session response must name an asker hidden from the Room roster.
#[tokio::test]
async fn session_snapshot_names_hidden_asker_before_any_tree_read() {
    let t = tree(
        "decision-asker-snapshot",
        AgentPermissionLevel::Required,
        AgentPermissionLevel::Required,
    )
    .await;
    t.raise(permission("snapshot-card", &t.c)).await;
    let request = crate::local::GetSessionStateRequest {
        session_id: t.room.clone(),
    };
    let response = t.runtime.session_state_response(request).unwrap();
    let wire = serde_json::to_value(response).unwrap();
    let session = &wire["SessionState"]["session"];
    let card = session["active_interactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["id"] == "snapshot-card")
        .unwrap();
    assert_eq!(card["agent_id"], t.c);
    assert_eq!(card["asker_name"], "c");
    let projected = t
        .runtime
        .session_snapshot(&t.room)
        .await
        .unwrap()
        .redacted_for_user(crate::session::DEFAULT_LOCAL_USER_ID);
    assert!(projected.agents().iter().all(|agent| agent.id() != t.c));
    let wire = serde_json::to_value(projected).unwrap();
    assert_eq!(wire["active_interactions"][0]["asker_name"], "c");
}

#[tokio::test]
async fn only_the_current_holder_may_answer() {
    let t = tree(
        "decision-holder-only",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let mut answer = t.raise(permission("perm-4", &t.c)).await;
    let refused = t
        .runtime
        .answer_child_decision(&t.room, &t.g, "perm-4", "allow")
        .await
        .unwrap();
    assert_eq!(refused, ChildDecisionStatus::NotHolder);
    let refused = t
        .runtime
        .answer_child_decision(&t.room, &t.c, "perm-4", "allow")
        .await
        .unwrap();
    assert_eq!(
        refused,
        ChildDecisionStatus::NotHolder,
        "the asker never answers itself"
    );
    assert!(
        answer.try_recv().is_err(),
        "a refused answer settles nothing"
    );
    let done = t
        .runtime
        .answer_child_decision(&t.room, &t.p, "perm-4", "allow")
        .await
        .unwrap();
    assert_eq!(done, ChildDecisionStatus::Answered);
    assert_eq!(
        answer.try_recv().unwrap().choice_id.as_deref(),
        Some("allow")
    );
    assert!(
        t.all_closed(&t.p),
        "the holder's event is withdrawn once settled"
    );
    assert!(t.session().active_interactions().is_empty());
}

#[tokio::test]
async fn a_peer_never_answers_a_decision_in_its_room_or_another() {
    let t = tree(
        "decision-peer-never",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let (neighbour, outsider) = {
        let mut app = t.app.lock().await;
        let neighbour = crate::app::KernelSessionService::new(&mut app)
            .spawn_agent(
                crate::agent::CreateAgentRequest::new(&t.room, "dev-stub").with_alias("neighbour"),
            )
            .unwrap();
        let (other_room, outsider) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "decision-peer-other",
                std::env::temp_dir().to_string_lossy(),
            ))
            .unwrap();
        (
            neighbour.id().to_owned(),
            (other_room.id().to_owned(), outsider.id().to_owned()),
        )
    };
    let mut answer = t.raise(permission("perm-peer", &t.c)).await;
    for (room, agent) in [
        (t.room.as_str(), neighbour.as_str()),
        (t.room.as_str(), outsider.1.as_str()),
        (outsider.0.as_str(), outsider.1.as_str()),
    ] {
        for status in [
            t.runtime
                .answer_child_decision(room, agent, "perm-peer", "allow")
                .await
                .unwrap(),
            t.runtime
                .pass_child_decision(room, agent, "perm-peer")
                .await
                .unwrap(),
        ] {
            assert!(
                matches!(
                    status,
                    ChildDecisionStatus::NotHolder | ChildDecisionStatus::UnknownInteraction
                ),
                "{status:?}"
            );
        }
    }
    assert!(answer.try_recv().is_err(), "a peer settles nothing");
    assert_eq!(t.session().active_interactions().len(), 1);
    assert_eq!(
        t.runtime
            .answer_child_decision(&t.room, &t.p, "perm-peer", "allow")
            .await
            .unwrap(),
        ChildDecisionStatus::Answered
    );
}

#[tokio::test]
async fn the_user_and_the_holder_answering_at_once_give_one_winner() {
    let t = tree(
        "decision-race",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let mut answer = t.raise(permission("perm-5", &t.c)).await;
    let handle = tokio::runtime::Handle::current();
    let (agent, user) = std::thread::scope(|scope| {
        let agent = scope.spawn(|| {
            handle.block_on(
                t.runtime
                    .answer_child_decision(&t.room, &t.p, "perm-5", "allow"),
            )
        });
        let user = scope.spawn(|| {
            t.runtime
                .owned
                .resolve_runtime_interaction(&t.room, "perm-5", "deny", None, None, false)
        });
        (agent.join().unwrap().unwrap(), user.join().unwrap())
    });
    assert!(
        (agent == ChildDecisionStatus::Answered) != user.is_ok(),
        "exactly one answer wins: agent {agent:?}, user {user:?}"
    );
    if agent != ChildDecisionStatus::Answered {
        // The late agent hears a typed status, not text.
        assert_eq!(agent, ChildDecisionStatus::SettledByUser);
    }
    let winner = if user.is_ok() { "deny" } else { "allow" };
    assert_eq!(
        answer.try_recv().unwrap().choice_id.as_deref(),
        Some(winner)
    );
    let late = t
        .runtime
        .answer_child_decision(&t.room, &t.p, "perm-5", "allow")
        .await
        .unwrap();
    assert_eq!(late, ChildDecisionStatus::SettledByUser);
}

#[tokio::test]
async fn a_parent_that_ends_mid_hold_hands_the_decision_up_with_no_model_turn() {
    let t = tree(
        "decision-ends",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let _answer = t.raise(permission("perm-6", &t.c)).await;
    assert_eq!(t.decision_events(&t.p).len(), 1);
    t.end_turn(&t.p).await;
    assert!(t.all_closed(&t.p), "the former holder's event is withdrawn");
    assert_eq!(
        t.decision_events(&t.g).len(),
        1,
        "the holder moved up to the grandparent"
    );
}

#[tokio::test]
async fn the_last_holder_ending_hands_the_same_interaction_to_the_user() {
    let t = tree(
        "decision-last",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Required,
    )
    .await;
    t.raise(permission("perm-9", &t.c)).await;
    assert_eq!(
        t.decision_events(&t.g).len(),
        1,
        "p is Required, so g holds it"
    );
    t.runtime.move_decisions_up(&t.room, &t.g).await.unwrap();
    assert!(t.all_closed(&t.g));
    let session = t.session();
    assert_eq!(session.interactions_listed_for(&t.g)[0].id(), "perm-9");
    assert!(session.interactions_listed_for(&t.c).is_empty());
    assert_eq!(
        t.runtime
            .answer_child_decision(&t.room, &t.g, "perm-9", "allow")
            .await
            .unwrap(),
        ChildDecisionStatus::NotHolder
    );
}

#[tokio::test]
async fn a_pass_hands_the_decision_to_the_next_holder() {
    let t = tree(
        "decision-pass",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    t.raise(permission("perm-7", &t.c)).await;
    assert_eq!(
        t.runtime
            .pass_child_decision(&t.room, &t.g, "perm-7")
            .await
            .unwrap(),
        ChildDecisionStatus::NotHolder
    );
    assert_eq!(
        t.runtime
            .pass_child_decision(&t.room, &t.p, "perm-7")
            .await
            .unwrap(),
        ChildDecisionStatus::Passed
    );
    assert!(t.all_closed(&t.p));
    assert_eq!(t.decision_events(&t.g).len(), 1);
    assert_eq!(
        t.runtime
            .answer_child_decision(&t.room, &t.g, "perm-7", "deny")
            .await
            .unwrap(),
        ChildDecisionStatus::Answered
    );
}

#[tokio::test]
async fn lowering_the_holder_is_rechecked_when_it_answers() {
    let t = tree(
        "decision-recheck",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let mut answer = t.raise(permission("perm-8", &t.c)).await;
    // Lowering the room agent lowers the whole team at once.
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
        .answer_child_decision(&t.room, &t.p, "perm-8", "allow")
        .await
        .unwrap();
    assert_eq!(
        status,
        ChildDecisionStatus::NotHolder,
        "p now runs at the lowered limit and cannot approve for itself"
    );
    assert!(answer.try_recv().is_err(), "the decision is still pending");
    assert_eq!(t.session().active_interactions().len(), 1);
}

#[tokio::test]
async fn free_text_answers_a_decision_that_takes_text() {
    let t = tree(
        "decision-text",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let question = RuntimeInteraction::new(
        "ask-1",
        &t.c,
        RuntimeInteractionKind::Choice,
        RuntimeInteractionLevel::Info,
        Some("Which branch?".into()),
        "Which branch?",
        vec![RuntimeInteractionChoice::new("skip", "Skip", "skip", None)],
        Some(RuntimeInteractionCustomChoice::new(
            "text",
            "Branch",
            None,
            Some(1),
            Some(80),
        )),
        None,
        None,
    );
    let mut answer = t.raise(question).await;
    assert_eq!(
        t.runtime
            .answer_child_decision(&t.room, &t.p, "ask-1", "main")
            .await
            .unwrap(),
        ChildDecisionStatus::Answered
    );
    let resolved = answer.try_recv().unwrap();
    assert_eq!(resolved.choice_id.as_deref(), Some("text"));
    assert_eq!(resolved.reply.as_deref(), Some("main"));
}

#[tokio::test]
async fn a_secret_request_reaches_the_user_even_with_a_yolo_parent() {
    let t = tree(
        "decision-secret",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let secret = RuntimeInteraction::new(
        "secret-1",
        &t.c,
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
    );
    t.raise(secret).await;
    assert!(t.decision_events(&t.p).is_empty() && t.decision_events(&t.g).is_empty());
    assert_eq!(
        t.session().interactions_listed_for(&t.g)[0].id(),
        "secret-1"
    );
}

/// One case per user-only kind: none of them is ever delegated.
#[test]
fn every_user_only_kind_is_the_users() {
    let pending = || PendingInteraction::for_test();
    let kernel = |id: &str, operation: &str| {
        RuntimeInteraction::for_kernel_operation(id, operation, "t", "m", Vec::new())
    };
    let mut credential = pending();
    credential.terminal_credential_owner = Some("owner".into());
    let mut user_domain = pending();
    user_domain.user_domain_interaction = Some(kernel("ud-1", "ud-1"));
    let passkey = RuntimeInteraction::for_kernel_operation(
        "pk-1",
        "critical:pk-1",
        "t",
        "m",
        vec![
            RuntimeInteractionChoice::new("approve", "Approve", "approve", None)
                .requiring_passkey(),
        ],
    );
    let critical = RuntimeInteraction::new(
        "crit-1",
        "agent-1",
        RuntimeInteractionKind::Permission,
        RuntimeInteractionLevel::Critical,
        None,
        "m",
        vec![RuntimeInteractionChoice::new(
            "allow", "Allow", "allow", None,
        )],
        None,
        None,
        None,
    );
    let secret = RuntimeInteraction::new(
        "sec-1",
        "agent-1",
        RuntimeInteractionKind::Choice,
        RuntimeInteractionLevel::Info,
        None,
        "m",
        Vec::new(),
        Some(RuntimeInteractionCustomChoice::secret(
            "secret",
            "S",
            None,
            Some(1),
            Some(8),
        )),
        None,
        None,
    );
    let table: Vec<(
        &str,
        PendingInteraction,
        RuntimeInteraction,
        Option<UserOnly>,
    )> = vec![
        ("secret value", pending(), secret, Some(UserOnly::Secret)),
        (
            "credential prompt",
            credential,
            permission("cp-1", "agent-1"),
            Some(UserOnly::Secret),
        ),
        (
            "sudo",
            pending(),
            kernel("sudo:1", "sudo:1"),
            Some(UserOnly::Sudo),
        ),
        ("passkey", pending(), passkey, Some(UserOnly::Passkey)),
        (
            "payment",
            pending(),
            kernel("vault-card-1", "vault-card-1"),
            Some(UserOnly::Payment),
        ),
        // MP-08 / MP-09 / MP-10 / MP-11: a payment an agent's fill raises in its own Room is still the owner's alone.
        (
            "agent's payment card",
            pending(),
            RuntimeInteraction::new(
                "vault-card-3",
                "agent-1",
                RuntimeInteractionKind::Choice,
                RuntimeInteractionLevel::Critical,
                Some("Use payment card".into()),
                "m",
                vec![
                    RuntimeInteractionChoice::new("allow", "Allow once", "allow", None)
                        .requiring_passkey(),
                ],
                None,
                None,
                None,
            ),
            Some(UserOnly::Payment),
        ),
        // MP-08 / MP-09 / MP-10 / MP-11: a payment stays a payment when approving needs the passkey.
        (
            "payment with passkey",
            pending(),
            RuntimeInteraction::for_kernel_operation(
                "vault-card-2",
                "vault-card-2",
                "t",
                "m",
                vec![
                    RuntimeInteractionChoice::new("allow", "Allow once", "allow", None)
                        .requiring_passkey(),
                ],
            ),
            Some(UserOnly::Payment),
        ),
        (
            "sign-in",
            pending(),
            kernel("handoff-1", "handoff-1"),
            Some(UserOnly::SignIn),
        ),
        (
            "access grant",
            pending(),
            kernel("ag-1", "access-grant:1"),
            Some(UserOnly::AccessGrant),
        ),
        (
            "user-domain browser",
            user_domain,
            permission("ud-2", "agent-1"),
            Some(UserOnly::UserDomain),
        ),
        ("critical", pending(), critical, Some(UserOnly::Critical)),
        (
            "machine or other kernel operation",
            pending(),
            kernel("op-1", "start-machine:1"),
            Some(UserOnly::KernelOperation),
        ),
        (
            "unclassified kernel decision",
            pending(),
            kernel("x-1", "whatever"),
            Some(UserOnly::KernelOperation),
        ),
        (
            "ordinary permission",
            pending(),
            permission("ok-1", "agent-1"),
            None,
        ),
    ];
    for (name, pending, interaction, expected) in table {
        assert_eq!(user_only(&pending, &interaction), expected, "{name}");
    }
}

/// What an external agent following the whole room is told about decisions, in order:
/// (event agent, interaction id, shown_in, state). `before` subscribes first (the live path);
/// a later subscriber gets the open decisions replayed.
struct Follower {
    grant: String,
}

impl Follower {
    fn follow(t: &Tree) -> Self {
        let grant = t.runtime.insert_access_grant_for_test(&t.room);
        let target = crate::local::EventTarget {
            kernel: t.runtime.owned.config_projection.snapshot().daemon_id,
            session_id: t.room.clone(),
            agent_id: None,
        };
        t.runtime
            .subscribe_outside_part(&grant, true, vec![target])
            .unwrap();
        Self { grant }
    }

    fn told(&self, t: &Tree) -> Vec<(Option<String>, Option<String>, Option<String>)> {
        t.runtime
            .outside_store()
            .outside_list(&self.grant, None, 100)
            .unwrap()
            .unwrap()
            .events
            .into_iter()
            .filter_map(|event| match event.value {
                crate::local::OutsideEventValue::Interaction {
                    state: crate::local::EventInteractionState::Pending,
                    shown_in,
                    ..
                } => Some((event.agent_id, event.refs.interaction_id, shown_in)),
                _ => None,
            })
            .collect()
    }
}

#[tokio::test]
async fn a_decision_the_user_holds_for_a_sub_agent_is_reported_in_the_room_agents_panel() {
    let t = tree(
        "decision-outside-user",
        AgentPermissionLevel::Required,
        AgentPermissionLevel::Required,
    )
    .await;
    let before = Follower::follow(&t);
    t.raise(permission("perm-ea1", &t.c)).await;
    let record = (
        Some(t.g.clone()),
        Some("perm-ea1".to_string()),
        Some(t.g.clone()),
    );
    assert_eq!(
        before.told(&t),
        std::slice::from_ref(&record),
        "live: same id, shown in the room agent"
    );
    let later = Follower::follow(&t);
    assert_eq!(
        later.told(&t),
        [record],
        "replay of the open decision says the same"
    );
}

#[tokio::test]
async fn a_decision_an_ancestor_holds_is_not_listed_to_the_user_until_it_reaches_the_user() {
    let t = tree(
        "decision-outside-agent",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let before = Follower::follow(&t);
    t.raise(permission("perm-ea2", &t.c)).await;
    assert_eq!(t.decision_events(&t.p).len(), 1, "p holds it");
    assert!(
        before.told(&t).is_empty(),
        "held by an agent: nothing listed live"
    );
    assert!(
        Follower::follow(&t).told(&t).is_empty(),
        "nor replayed to a later follower"
    );
    t.runtime.move_decisions_up(&t.room, &t.p).await.unwrap();
    assert!(
        before.told(&t).is_empty(),
        "g holds it now: still not the user's"
    );
    t.runtime.move_decisions_up(&t.room, &t.g).await.unwrap();
    assert_eq!(
        before.told(&t),
        [(
            Some(t.g.clone()),
            Some("perm-ea2".to_string()),
            Some(t.g.clone())
        )],
        "the user holds it: one record, same id, shown in the room agent"
    );
}

/// The holder changes, the interaction stays: it is the same pending decision, still waited on by
/// its asker (a receiver nobody holds is an abandoned decision, which the kernel sweeps away).
#[tokio::test]
async fn the_decision_survives_when_its_holder_ends_its_turn() {
    let t = tree(
        "decision-survives",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let mut answer = t.raise(permission("perm-s1", &t.c)).await;
    t.end_turn(&t.p).await;
    assert_eq!(
        t.present("perm-s1"),
        (true, true),
        "still listed and still pending"
    );
    assert!(answer.try_recv().is_err(), "the asker still waits");
    assert_eq!(
        t.runtime
            .answer_child_decision(&t.room, &t.g, "perm-s1", "allow")
            .await
            .unwrap(),
        ChildDecisionStatus::Answered,
        "the next holder answers it"
    );
    assert_eq!(
        answer.try_recv().unwrap().choice_id.as_deref(),
        Some("allow")
    );
}

/// A decision whose asker went away is abandoned: the kernel sweeps it, whoever holds it.
#[tokio::test]
async fn a_decision_nobody_waits_for_is_withdrawn_with_its_holder_event() {
    let t = tree(
        "decision-abandoned",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    drop(t.raise(permission("perm-s2", &t.c)).await);
    assert_eq!(t.decision_events(&t.p).len(), 1);
    t.runtime.owned.withdraw_stale_agent_interactions();
    assert_eq!(t.present("perm-s2"), (false, false));
    assert!(
        t.all_closed(&t.p),
        "the holder's event is withdrawn with it"
    );
}

/// A turn that ends without finishing leaves its task waiting on the owner: that agent cannot act,
/// so the decision it held moves up as for any other ended turn (drill: a parent with a live child).
#[tokio::test]
async fn a_holder_whose_task_is_blocked_hands_the_decision_up() {
    let t = tree(
        "decision-blocked",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let _answer = t.raise(permission("perm-b1", &t.c)).await;
    assert_eq!(t.decision_events(&t.p).len(), 1);
    let ((task, _), prompt) = t.settle_turn(&t.p, false, false);
    let Outcome::Task(blocked) = t
        .runtime
        .owned
        .durable_state_store
        .agent_lifecycle(Operation::Block {
            task: task.task_id,
            prompt: prompt.id().into(),
            reason: "unfinished child".into(),
        })
        .unwrap()
    else {
        panic!("blocked")
    };
    assert_eq!(
        blocked.state,
        crate::durable_state::agent_lifecycle::ExecutionState::Blocked
    );
    let settlement = (blocked, false);
    t.runtime
        .finish_agent_task_settlement(Some(settlement), &prompt)
        .await
        .unwrap();
    assert!(t.all_closed(&t.p), "the former holder's event is withdrawn");
    assert_eq!(t.decision_events(&t.g).len(), 1, "g holds it now");
    assert_eq!(t.present("perm-b1"), (true, true));
}

impl Tree {
    /// The nodes of the top agent's tree as (alias, state, needs_user), with the user count.
    fn needs(&self) -> (Vec<(String, crate::local::AgentTreeState, bool)>, u32) {
        let request = crate::local::ListDescendantsRequest {
            session_id: self.room.clone(),
            agent_id: self.g.clone(),
        };
        let crate::local::LocalDaemonResponse::AgentTree { tree } = self
            .runtime
            .list_descendants(crate::session::DEFAULT_LOCAL_USER_ID, request)
            .unwrap()
        else {
            panic!("agent tree expected")
        };
        let nodes = tree
            .nodes
            .iter()
            .map(|node| {
                (
                    node.alias.clone().unwrap_or_default(),
                    node.state,
                    node.needs_user,
                )
            })
            .collect();
        (nodes, tree.totals.needs_user)
    }
}

// A decision an ancestor agent holds keeps the node waiting but is no card for the user.
#[tokio::test]
async fn a_node_whose_decision_an_agent_holds_does_not_need_the_user() {
    use crate::local::AgentTreeState::Waiting;
    let t = tree(
        "tree-held-by-agent",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let _answer = t.raise(permission("perm-held", &t.c)).await;
    let (nodes, users) = t.needs();
    assert!(
        nodes.contains(&("c".into(), Waiting, false)),
        "p holds it: {nodes:?}"
    );
    assert_eq!(users, 0);
}

// The parent ends and the decision climbs to the user: now the node needs the user.
#[tokio::test]
async fn a_decision_that_moves_up_to_the_user_makes_the_node_need_the_user() {
    use crate::local::AgentTreeState::Waiting;
    let t = tree(
        "tree-moves-up",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Required,
    )
    .await;
    let _answer = t.raise(permission("perm-up", &t.c)).await;
    assert!(
        t.needs().0.contains(&("c".into(), Waiting, false)),
        "g holds it"
    );
    t.runtime.move_decisions_up(&t.room, &t.g).await.unwrap();
    let (nodes, users) = t.needs();
    assert!(nodes.contains(&("c".into(), Waiting, true)), "{nodes:?}");
    assert_eq!(users, 1);
}

// Two pending decisions: the one the user holds counts, the one an agent holds does not.
#[tokio::test]
async fn only_the_nodes_the_user_holds_a_decision_of_need_the_user() {
    use crate::local::AgentTreeState::Waiting;
    let t = tree(
        "tree-mixed",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let secret = RuntimeInteraction::new(
        "secret-mixed",
        &t.c,
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
    );
    t.raise(secret).await;
    let _answer = t.raise(permission("perm-mixed", &t.p)).await;
    let (nodes, users) = t.needs();
    assert!(nodes.contains(&("c".into(), Waiting, true)), "{nodes:?}");
    assert!(nodes.contains(&("p".into(), Waiting, false)), "{nodes:?}");
    assert_eq!(users, 1);
}

// An orchestrating parent's inbox outgrows one page of rows; its settled decision events still close.
#[tokio::test]
async fn a_settled_decision_event_beyond_the_first_inbox_page_is_still_closed() {
    use crate::durable_state::agent_lifecycle as ledger;
    let t = tree(
        "decision-long-inbox",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    for n in 0..ledger::INBOX_PAGE + 2 {
        t.runtime
            .owned
            .durable_state_store
            .agent_lifecycle(Operation::Occur(ledger::occurrence(
                &t.room,
                &t.p,
                &t.g,
                &format!("earlier-{n}"),
                "message",
                serde_json::json!({"message": format!("earlier {n}")}),
            )))
            .unwrap();
    }
    t.raise(permission("perm-long", &t.c)).await;
    let open_decisions = |t: &Tree| {
        t.runtime
            .owned
            .durable_state_store
            .agent_work_inbox(&t.room, &t.p, ledger::INBOX_PAGE as u64, None)
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == "child_decision")
            .filter(|event| !matches!(event.state.as_str(), "expired" | "handled"))
            .count()
    };
    assert_eq!(open_decisions(&t), 1, "p was told");
    let done = t
        .runtime
        .answer_child_decision(&t.room, &t.p, "perm-long", "allow")
        .await
        .unwrap();
    assert_eq!(done, ChildDecisionStatus::Answered);
    assert_eq!(
        open_decisions(&t),
        0,
        "the settled decision is still an open event in the holder's inbox"
    );
}

#[tokio::test]
async fn a_decision_the_holder_answered_can_be_acknowledged_by_sequence() {
    let t = tree(
        "decision-ack-after-answer",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    t.raise(permission("perm-ack", &t.c)).await;
    let sequence = t.decision_events(&t.p)[0].sequence;
    let done = t
        .runtime
        .answer_child_decision(&t.room, &t.p, "perm-ack", "allow")
        .await
        .unwrap();
    assert_eq!(done, ChildDecisionStatus::Answered);
    let acked = t
        .runtime
        .ack_agent_inbox(
            &t.room,
            &t.p,
            &serde_json::json!({"sequence": sequence, "handled": true}),
            1,
        )
        .await
        .expect("an answered decision no longer needs an answer");
    assert_eq!(acked["acknowledged"], sequence);
}

/// The holder's turn is settling and its output pump holds the run's provider lane: the child's
/// approval request must not wait for it, and must reach the holder once its turn has ended.
#[tokio::test]
async fn a_decision_raised_while_the_holders_turn_settles_does_not_wait_for_its_lane() {
    let t = tree(
        "decision-settling-holder",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let run = t
        .runtime
        .owned
        .provider_store
        .get_run_for_agent(&t.room, &t.p)
        .unwrap();
    let lane = t.runtime.provider_runtime_lanes.acquire(run.id()).await;
    let raised = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        t.raise(permission("perm-settling", &t.c)),
    )
    .await;
    assert!(
        raised.is_ok(),
        "the child's approval request waited for the holder's settling turn"
    );
    let mut answer = raised.unwrap();
    let events = t.decision_events(&t.p);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_ne!(
        events[0].state, "submitting",
        "the steer that sent nothing is not left in flight: {events:?}"
    );
    assert!(answer.try_recv().is_err(), "nobody has answered yet");
    drop(lane);
    assert_eq!(
        t.runtime
            .answer_child_decision(&t.room, &t.p, "perm-settling", "allow")
            .await
            .unwrap(),
        ChildDecisionStatus::Answered,
        "the holder still answers the same decision"
    );
    assert_eq!(
        answer.try_recv().unwrap().choice_id.as_deref(),
        Some("allow")
    );
}

impl Tree {
    /// `agent` reads its inbox in its turn, the way the `events.inbox` tool does.
    fn read_inbox(&self, agent: &str) {
        self.runtime
            .read_agent_inbox(&self.room, agent, &serde_json::json!({}))
            .unwrap();
    }
}

/// Live (uiqa r5 hk-10): the parent read its child's decision, said it would leave it to the
/// person and yielded on the child. Nobody else would ever see the decision: it moves up as for
/// any ended turn, and the user's card is listed in the top panel.
#[tokio::test]
async fn a_holder_that_yields_without_answering_hands_the_decision_on() {
    let t = tree(
        "decision-yield",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let mut answer = t.raise(permission("perm-yield", &t.c)).await;
    assert_eq!(t.decision_events(&t.p).len(), 1, "p holds it and is told");

    t.read_inbox(&t.p);
    t.end_turn_yielded(&t.p).await;
    assert!(t.all_closed(&t.p), "p's event is withdrawn");
    assert_eq!(t.decision_events(&t.g).len(), 1, "g holds it and is told");
    assert!(t.session().interactions_listed_for(&t.g).is_empty());

    t.read_inbox(&t.g);
    t.end_turn_yielded(&t.g).await;
    assert!(t.all_closed(&t.g));
    assert_eq!(
        t.session().interactions_listed_for(&t.g)[0].id(),
        "perm-yield",
        "nobody above g: the user sees the card in g's panel"
    );
    assert_eq!(t.needs().1, 1, "the user's decision is counted");
    for former in [&t.p, &t.g] {
        assert_eq!(
            t.runtime
                .answer_child_decision(&t.room, former, "perm-yield", "allow")
                .await
                .unwrap(),
            ChildDecisionStatus::NotHolder,
            "a former holder that wakes later no longer holds it"
        );
    }
    assert!(
        answer.try_recv().is_err(),
        "the asker still waits, for the user"
    );
}

/// A holder that yields before its turn read the decision is woken for it: it keeps the decision.
#[tokio::test]
async fn a_holder_that_yields_before_reading_the_decision_keeps_it() {
    let t = tree(
        "decision-yield-unread",
        AgentPermissionLevel::Yolo,
        AgentPermissionLevel::Yolo,
    )
    .await;
    let _answer = t.raise(permission("perm-unread", &t.c)).await;
    t.end_turn_yielded(&t.p).await;
    let events = t.decision_events(&t.p);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].state, "pending", "the next wake delivers it");
    assert!(t.decision_events(&t.g).is_empty());
    assert_eq!(
        t.runtime
            .answer_child_decision(&t.room, &t.p, "perm-unread", "allow")
            .await
            .unwrap(),
        ChildDecisionStatus::Answered
    );
}

mod answer_audit;
mod asked_without_waiting;
mod detail;
mod holder_inbox_reads;
mod reparent;
mod restored;
mod tenure;
