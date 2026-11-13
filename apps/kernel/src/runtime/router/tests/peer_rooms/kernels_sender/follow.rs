//! Following a peer across kernels: A's room agent follows B's worker through its own subscribe
//! tool, and B's turn ends reach A as one `peer_event` each, stamped with where the peer runs.
use super::*;
use crate::durable_state::agent_lifecycle::{InboxEvent, Operation};
use crate::transport::relay_peer::{
    PeerAgentAddress, PeerAgentOutcome, PeerAgentStamp, PeerOutcomeAck, RelayPeerResponse,
    RELAY_PEER_PROTOCOL_VERSION,
};
use crate::transport::relay_peer_agents::PeerEventKind;

pub(super) mod offline;
mod pre_enrollment;

/// A's room agent gets a durable task for its running turn, as an admitted turn has.
async fn with_task(kernel: &Kernel) -> String {
    let app = kernel.app.lock().await;
    app.durable_state_store()
        .agent_lifecycle(Operation::Begin {
            owner: kernel.agent.owner_user_id().into(),
            room: kernel.room.clone(),
            agent: kernel.agent.id().into(),
            prompt: kernel.prompt.clone(),
            run: None,
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap();
    app.durable_state_store()
        .agent_tasks(Some(&kernel.room), Some(kernel.agent.id()))
        .unwrap()
        .pop()
        .unwrap()
        .task_id
}

impl Kernel {
    async fn subscribe(
        &self,
        task: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let mut args = args;
        args["task_id"] = task.into();
        args["origin_prompt_id"] = self.prompt.clone().into();
        match self
            .router
            .dispatch_authenticated_runtime_tool_call(&self.token, "chariox.events.subscribe", args)
            .await
        {
            Ok(result) if result.ok => Ok(result.payload),
            Ok(result) => Err(result.payload.to_string()),
            Err(error) => Err(error.to_string()),
        }
    }

    async fn tool(
        &self,
        name: &str,
        task: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let mut args = args;
        args["task_id"] = task.into();
        args["origin_prompt_id"] = self.prompt.clone().into();
        match self
            .router
            .dispatch_authenticated_runtime_tool_call(&self.token, name, args)
            .await
        {
            Ok(result) if result.ok => Ok(result.payload),
            Ok(result) => Err(result.payload.to_string()),
            Err(error) => Err(error.to_string()),
        }
    }

    async fn peer_events(&self) -> Vec<InboxEvent> {
        self.app
            .lock()
            .await
            .durable_state_store()
            .agent_inbox(&self.room, self.agent.id(), 0)
            .unwrap()
            .into_iter()
            .filter(|event| event.kind == "peer_event")
            .collect()
    }

    /// Waits for the outcome delivery a settlement starts off its own path.
    async fn peer_events_after_delivery(&self, count: usize) -> Vec<InboxEvent> {
        for _ in 0..100 {
            let events = self.peer_events().await;
            if events.len() >= count {
                return events;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        self.peer_events().await
    }

    async fn outbox(
        &self,
    ) -> Vec<crate::durable_state::agent_lifecycle::peer_follows::PeerOutboxRow> {
        self.app
            .lock()
            .await
            .durable_state_store()
            .peer_agent_outbox_due(
                crate::session::unix_epoch_ms() + 60 * 60 * 1_000,
                crate::session::unix_epoch_ms(),
            )
            .unwrap()
    }

    async fn outbox_drained(&self) -> bool {
        for _ in 0..100 {
            if self.outbox().await.is_empty() {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        false
    }

    async fn end_turn(&self, cancelled: bool) {
        self.router
            .runtime_state
            .end_agent_turn_for_tests(&self.room, self.agent.id(), cancelled)
            .await
            .expect("the turn settles");
    }

    async fn peer_request(&self, from: &str, request: RelayPeerRequest) -> RelayPeerResponse {
        self.router
            .relay_peer_agent(from, OWNER, request)
            .await
            .expect("the peer request answers")
    }
}

/// A follows B's worker with `kernel`; answers A's task and the follow's registration.
async fn followed(test: &str) -> (Kernel, Kernel, String, serde_json::Value) {
    let (a, b) = two_kernels(test, AgentPermissionLevel::Yolo, AgentPermissionLevel::Yolo);
    let task = with_task(&a).await;
    let follow = a
        .subscribe(
            &task,
            serde_json::json!({"source_id": "worker", "room": b.room, "kernel": b.name}),
        )
        .await
        .expect("A follows a peer on another kernel");
    (a, b, task, follow)
}

fn source_of(b: &Kernel) -> String {
    format!("{}/{}/{}", b.id, b.room, b.agent.id())
}

fn subscribe_request(subscription: &str, generation: u64, room: &str) -> RelayPeerRequest {
    RelayPeerRequest::SubscribePeerAgent {
        protocol_version: RELAY_PEER_PROTOCOL_VERSION,
        request_id: format!("{subscription}-{generation}"),
        sender: PeerAgentStamp {
            kernel_id: "follower".into(),
            room_id: "follower-room".into(),
            agent_id: "follower-agent".into(),
        },
        target: PeerAgentAddress {
            room: room.into(),
            agent: "worker".into(),
        },
        subscription_id: subscription.into(),
        generation,
    }
}

#[test]
fn a_followed_peer_turn_end_on_another_kernel_reaches_the_follower_as_one_stamped_peer_event() {
    run_peer_test("peer-follow-ended", || async {
        let (a, b, _task, follow) = followed("peer-follow-ended").await;
        assert_eq!(follow["kernel"], b.id.as_str(), "{follow}");
        assert_eq!(follow["agent"], b.agent.id(), "{follow}");
        assert_eq!(follow["source_id"], source_of(&b).as_str(), "{follow}");

        b.end_turn(false).await;
        let events = a.peer_events_after_delivery(1).await;
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].source_id, source_of(&b));
        assert_eq!(events[0].payload["event"], "turn_ended", "{:?}", events[0]);
        assert!(events[0].payload.get("public_answer").is_none());
        assert!(
            b.outbox_drained().await,
            "delivered outcomes leave B's outbox"
        );

        let read = a
            .router
            .runtime_state
            .read_agent_inbox(&a.room, a.agent.id(), &serde_json::json!({}))
            .unwrap();
        let view = read["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|event| event["kind"] == "peer_event")
            .cloned()
            .unwrap();
        assert_eq!(view["source"]["kind"], "agent", "{view}");
        assert_eq!(view["source"]["kernel_id"], b.id.as_str(), "{view}");
        assert_eq!(view["source"]["room"], b.room.as_str(), "{view}");
        assert_eq!(view["source"]["relation"], "peer", "{view}");
        assert!(
            view["summary"]
                .as_str()
                .unwrap()
                .contains(&format!("{}/{}", b.id, b.agent.id())),
            "{view}"
        );
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_follow_on_another_kernel_reports_every_turn_of_the_peer() {
    run_peer_test("peer-follow-every-turn", || async {
        let (a, mut b) = two_kernels(
            "peer-follow-every-turn",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        let task = with_task(&a).await;
        a.subscribe(
            &task,
            serde_json::json!({"source_id": "worker", "room": b.room, "kernel": b.name}),
        )
        .await
        .expect("A follows a peer on another kernel");
        b.end_turn(false).await;
        assert_eq!(a.peer_events_after_delivery(1).await.len(), 1);
        // The follower's own turn is over; the follow is not.
        a.end_turn(false).await;

        (b.token, b.prompt) = running_turn(&mut *b.app.lock().await, &b.room, b.agent.id());
        b.end_turn(true).await;
        let events = a.peer_events_after_delivery(2).await;
        let kinds: Vec<_> = events.iter().map(|e| e.payload["event"].clone()).collect();
        assert_eq!(kinds, ["turn_ended", "turn_failed"], "{events:?}");
        assert!(b.outbox_drained().await);
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_failed_turn_of_a_followed_peer_on_another_kernel_says_so() {
    run_peer_test("peer-follow-failed", || async {
        let (a, b, _task, _follow) = followed("peer-follow-failed").await;
        b.end_turn(true).await;
        let events = a.peer_events_after_delivery(1).await;
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].payload["event"], "turn_failed", "{:?}", events[0]);
        unlink(&[&a, &b]);
    });
}

#[test]
fn the_same_peer_outcome_delivered_twice_is_one_inbox_row() {
    run_peer_test("peer-follow-duplicate", || async {
        let (a, b, _task, follow) = followed("peer-follow-duplicate").await;
        // B's answer was lost: A is unreachable when the turn ends, and B keeps the outcome.
        peer_agent_link_for_tests::unregister(&a.id);
        b.end_turn(false).await;
        let owed = b.outbox().await;
        assert_eq!(owed.len(), 1, "{owed:?}");
        let deliver = RelayPeerRequest::DeliverPeerAgentOutcome {
            protocol_version: RELAY_PEER_PROTOCOL_VERSION,
            delivery_id: owed[0].delivery_id.clone(),
            outcome: PeerAgentOutcome::Event {
                subscription_id: follow["registration_id"].as_str().unwrap().into(),
                source: PeerAgentStamp {
                    kernel_id: b.id.clone(),
                    room_id: b.room.clone(),
                    agent_id: b.agent.id().into(),
                },
                occurrence_id: owed[0].occurrence_id.clone(),
                event: PeerEventKind::TurnEnded,
            },
        };
        let first = a.peer_request(&b.id, deliver.clone()).await;
        let again = a.peer_request(&b.id, deliver).await;
        let acks = [first, again].map(|answer| match answer {
            RelayPeerResponse::PeerAgentOutcomeAck { ack, .. } => ack,
            other => panic!("{other:?}"),
        });
        assert_eq!(acks, [PeerOutcomeAck::Accepted, PeerOutcomeAck::Duplicate]);
        assert_eq!(a.peer_events().await.len(), 1);
        unlink(&[&b]);
    });
}

#[test]
fn an_older_generation_of_a_follow_changes_nothing() {
    run_peer_test("peer-follow-generation", || async {
        let (a, b) = two_kernels(
            "peer-follow-generation",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        unlink(&[&a]);
        let held = b
            .peer_request(&a.id, subscribe_request("follow-g", 3, &b.room))
            .await;
        assert!(
            matches!(
                held,
                RelayPeerResponse::PeerAgentSubscribed { generation: 3, .. }
            ),
            "{held:?}"
        );
        let older = b
            .peer_request(&a.id, subscribe_request("follow-g", 2, &b.room))
            .await;
        assert!(
            matches!(
                older,
                RelayPeerResponse::PeerAgentSubscribed { generation: 3, .. }
            ),
            "the newer generation holds: {older:?}"
        );
        b.peer_request(
            &a.id,
            RelayPeerRequest::UnsubscribePeerAgent {
                protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                subscription_id: "follow-g".into(),
                generation: 2,
            },
        )
        .await;
        b.end_turn(false).await;
        let owed = b.outbox().await;
        assert_eq!(
            owed.len(),
            1,
            "an older unsubscribe left the follow: {owed:?}"
        );
        assert_eq!(owed[0].subscription_id, "follow-g");
        assert_eq!(owed[0].subscriber_kernel_id, a.id);
        unlink(&[&b]);
    });
}

#[test]
fn another_kernel_of_the_owner_cannot_end_someone_elses_follow() {
    run_peer_test("peer-follow-intruder", || async {
        let (a, b) = two_kernels(
            "peer-follow-intruder",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        unlink(&[&a]);
        b.peer_request(&a.id, subscribe_request("follow-i", 1, &b.room))
            .await;
        b.peer_request(
            "peer-follow-intruder-c",
            RelayPeerRequest::UnsubscribePeerAgent {
                protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                subscription_id: "follow-i".into(),
                generation: 9,
            },
        )
        .await;
        let taken = b
            .router
            .relay_peer_agent(
                "peer-follow-intruder-c",
                OWNER,
                subscribe_request("follow-i", 9, &b.room),
            )
            .await;
        assert!(
            taken.is_err(),
            "a subscription id is its kernel's own: {taken:?}"
        );
        b.end_turn(false).await;
        let owed = b.outbox().await;
        assert_eq!(owed.len(), 1, "{owed:?}");
        assert_eq!(owed[0].subscriber_kernel_id, a.id);

        // The kernel that follows ends its own follow.
        b.peer_request(
            &a.id,
            RelayPeerRequest::UnsubscribePeerAgent {
                protocol_version: RELAY_PEER_PROTOCOL_VERSION,
                subscription_id: "follow-i".into(),
                generation: 1,
            },
        )
        .await;
        unlink(&[&b]);
    });
}

#[test]
fn a_card_raised_and_answered_on_the_followed_kernel_gives_no_outcome() {
    run_peer_test("peer-follow-card", || async {
        use crate::session::{
            RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionKind,
            RuntimeInteractionLevel,
        };
        let (a, b, _task, _follow) = followed("peer-follow-card").await;
        let card = RuntimeInteraction::new(
            "perm-follow",
            b.agent.id(),
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
        );
        let _answer = b
            .router
            .runtime_state
            .create_runtime_interaction(&b.room, card)
            .await
            .unwrap();
        b.router
            .runtime_state
            .resolve_runtime_interaction(&b.room, "perm-follow", "allow", None)
            .await
            .unwrap();
        b.router.runtime_state.deliver_peer_agent_outcomes().await;
        assert!(b.outbox().await.is_empty());
        assert!(
            a.peer_events().await.is_empty(),
            "{:?}",
            a.peer_events().await
        );
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_followed_peer_deleted_on_its_kernel_is_a_lost_source_for_the_follower() {
    run_peer_test("peer-follow-lost", || async {
        let (a, b) = two_kernels(
            "peer-follow-lost",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        // B's worker is idle: its removal ends no turn, and only the follow can tell A.
        b.end_turn(false).await;
        let task = with_task(&a).await;
        a.subscribe(
            &task,
            serde_json::json!({"source_id": "worker", "room": b.room, "kernel": b.name}),
        )
        .await
        .expect("A follows an idle peer on another kernel");
        let destroy =
            crate::local::LocalDaemonRequest::DestroyAgent(crate::local::DestroyAgentRequest {
                session_id: b.room.clone(),
                agent_id: b.agent.id().into(),
            });
        b.as_owner("destroy-followed", destroy).await;
        b.router
            .runtime_state
            .sweep_agent_lifecycle_for_test()
            .await
            .unwrap();
        let events = a.peer_events_after_delivery(1).await;
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].payload["event"], "source_lost", "{:?}", events[0]);
        assert_eq!(events[0].source_id, source_of(&b));
        unlink(&[&a, &b]);
    });
}

impl Kernel {
    /// The owner acts here, as from any of its clients.
    async fn as_owner(&self, command_id: &str, request: crate::local::LocalDaemonRequest) {
        let source = crate::runtime::command::KernelCommandSource::DaemonBackground;
        let caller = crate::runtime::command::KernelCaller {
            user_id: Some(OWNER.into()),
            ..crate::runtime::command::KernelCaller::for_source(&source)
        };
        let command = KernelCommand::from_local_request_with_caller(
            command_id, source, caller, None, None, &request,
        );
        self.router.dispatch(command, request).await.unwrap();
    }
}

#[test]
fn a_peer_spawned_on_another_kernel_is_followed_by_its_creator_unless_no_follow() {
    run_peer_test("peer-follow-spawn", || async {
        let (a, b) = two_kernels(
            "peer-follow-spawn",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        with_task(&a).await;
        let spawned = a
            .command(format!(
                "agent spawn reviewer --room {} --kernel {}",
                b.room, b.name
            ))
            .await
            .expect("a followed peer spawns on the other kernel");
        let registration = spawned["follow_registration_id"]
            .as_str()
            .unwrap_or_else(|| panic!("{spawned}"))
            .to_owned();
        let peer = spawned["response"]["spawned"]["agent"]
            .as_str()
            .unwrap()
            .to_owned();
        let quiet = a
            .command(format!(
                "agent spawn quiet --room {} --kernel {} --no-follow",
                b.room, b.name
            ))
            .await
            .expect("an unfollowed peer spawns on the other kernel");
        assert!(quiet["follow_registration_id"].is_null(), "{quiet}");

        running_turn(&mut *b.app.lock().await, &b.room, &peer);
        b.router
            .runtime_state
            .end_agent_turn_for_tests(&b.room, &peer, false)
            .await
            .unwrap();
        let events = a.peer_events_after_delivery(1).await;
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].source_id, format!("{}/{}/{peer}", b.id, b.room));
        assert_eq!(events[0].payload["event"], "turn_ended");
        let held = a
            .app
            .lock()
            .await
            .durable_state_store()
            .agent_registration(&registration)
            .unwrap()
            .unwrap();
        assert!(
            held.live,
            "a follow goes on after the turn it reports: {held:?}"
        );
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_keyed_spawn_repeated_in_a_later_turn_gives_that_turn_its_own_follow() {
    run_peer_test("peer-follow-keyed", || async {
        let (mut a, b) = two_kernels(
            "peer-follow-keyed",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        let keyed = format!(
            "agent spawn tester --room {} --kernel {} --command-id spawn-tester-1",
            b.room, b.name
        );
        let first_task = with_task(&a).await;
        let first = a.command(keyed.clone()).await.expect("a keyed spawn");
        let first_follow = first["follow_registration_id"].as_str().unwrap().to_owned();
        a.end_turn(false).await;

        (a.token, a.prompt) = running_turn(&mut *a.app.lock().await, &a.room, a.agent.id());
        let later_task = with_task(&a).await;
        let again = a
            .command(keyed)
            .await
            .expect("the same keyed spawn in a later turn");
        assert_eq!(again["response"]["spawned"]["duplicate"], true);
        let peer = again["response"]["spawned"]["agent"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(first["response"]["spawned"]["agent"], peer.as_str());
        let later_follow = again["follow_registration_id"].as_str().unwrap().to_owned();
        assert_ne!(later_follow, first_follow, "each turn holds its own follow");

        let registrations = |task: String| {
            let app = a.app.clone();
            async move {
                app.lock()
                    .await
                    .durable_state_store()
                    .agent_registrations(&task)
                    .expect("registrations read clean")
            }
        };
        let earlier = registrations(first_task.clone()).await;
        assert!(
            earlier
                .iter()
                .any(|held| held.id == first_follow && held.task_id == first_task),
            "{earlier:?}"
        );
        let later = registrations(later_task.clone()).await;
        assert!(
            later
                .iter()
                .any(|held| held.id == later_follow && held.task_id == later_task && held.live),
            "{later:?}"
        );

        running_turn(&mut *b.app.lock().await, &b.room, &peer);
        b.router
            .runtime_state
            .end_agent_turn_for_tests(&b.room, &peer, false)
            .await
            .unwrap();
        let events = a.peer_events_after_delivery(1).await;
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].source_id, format!("{}/{}/{peer}", b.id, b.room));
        assert!(b.outbox_drained().await);
        let later = registrations(later_task.clone()).await;
        assert!(
            later
                .iter()
                .any(|held| held.id == later_follow && held.live),
            "the later turn's follow took the outcome and goes on: {later:?}"
        );
        registrations(first_task).await;
        unlink(&[&a, &b]);
    });
}

/// `kernel` follows as asked, then its worker's turn ends and the outcome goes out before the
/// follow's answer is back on the following kernel.
fn link_ending_the_turn_before_answering(kernel: &Kernel) {
    let router = kernel.router.clone();
    let (room, agent) = (kernel.room.clone(), kernel.agent.id().to_owned());
    peer_agent_link_for_tests::register(
        &kernel.id,
        &kernel.name,
        Arc::new(
            move |from: String, owner: String, request: RelayPeerRequest| {
                let router = router.clone();
                let (room, agent) = (room.clone(), agent.clone());
                Box::pin(async move {
                    let follow = matches!(request, RelayPeerRequest::SubscribePeerAgent { .. });
                    let answer = router.relay_peer_agent(&from, &owner, request).await;
                    if follow {
                        router
                            .runtime_state
                            .end_agent_turn_for_tests(&room, &agent, false)
                            .await?;
                        router.runtime_state.deliver_peer_agent_outcomes().await;
                    }
                    answer
                })
            },
        ),
    );
}

#[test]
fn a_turn_end_that_overtakes_the_follow_answer_still_reaches_the_follower_once() {
    run_peer_test("peer-follow-overtaken", || async {
        let (a, mut b) = two_kernels(
            "peer-follow-overtaken",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        link_ending_the_turn_before_answering(&b);
        let task = with_task(&a).await;
        a.subscribe(
            &task,
            serde_json::json!({"source_id": "worker", "room": b.room, "kernel": b.name}),
        )
        .await
        .expect("A follows a peer on another kernel");
        let events = a.peer_events_after_delivery(1).await;
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(events[0].source_id, source_of(&b));
        assert_eq!(events[0].payload["event"], "turn_ended");
        assert!(b.outbox_drained().await);
        assert_eq!(a.peer_events().await.len(), 1);

        // The overtaking turn end did not close the follow: the next turn reaches it too.
        (b.token, b.prompt) = running_turn(&mut *b.app.lock().await, &b.room, b.agent.id());
        b.end_turn(false).await;
        let events = a.peer_events_after_delivery(2).await;
        assert_eq!(events.len(), 2, "{events:?}");
        let held = a
            .app
            .lock()
            .await
            .durable_state_store()
            .agent_registrations(&task)
            .unwrap();
        assert!(held.iter().any(|r| r.peer && r.live), "{held:?}");
        unlink(&[&a, &b]);
    });
}

#[test]
fn a_follow_kept_past_its_turn_is_listed_and_unsubscribed_in_a_later_turn() {
    run_peer_test("peer-follow-unsubscribe-later", || async {
        let (mut a, mut b) = two_kernels(
            "peer-follow-unsubscribe-later",
            AgentPermissionLevel::Yolo,
            AgentPermissionLevel::Yolo,
        );
        let task = with_task(&a).await;
        let follow = a
            .subscribe(
                &task,
                serde_json::json!({"source_id": "worker", "room": b.room, "kernel": b.name}),
            )
            .await
            .expect("A follows a peer on another kernel");
        let id = follow["registration_id"].as_str().unwrap().to_owned();
        b.end_turn(false).await;
        let delivered = a.peer_events_after_delivery(1).await;
        assert_eq!(delivered.len(), 1);
        // A peer event still pending when A's turn ends wakes A; this turn handles it, so A is
        // idle until the later turn below.
        a.tool(
            "chariox.events.ack",
            &task,
            serde_json::json!({"sequence": delivered[0].sequence, "handled": true}),
        )
        .await
        .expect("A handles the peer event in its turn");
        a.end_turn(false).await;

        (a.token, a.prompt) = running_turn(&mut *a.app.lock().await, &a.room, a.agent.id());
        let second = with_task(&a).await;
        let listed = a
            .tool(
                "chariox.events.subscriptions",
                &second,
                serde_json::json!({}),
            )
            .await
            .expect("the later turn lists subscriptions");
        assert!(
            listed["peer_follows"]
                .as_array()
                .is_some_and(|f| f.iter().any(|r| r["id"] == id.as_str())),
            "the follow kept past its turn is listed: {listed}"
        );
        a.tool(
            "chariox.events.unsubscribe",
            &second,
            serde_json::json!({"registration_id": id}),
        )
        .await
        .expect("the later turn ends the follow");

        (b.token, b.prompt) = running_turn(&mut *b.app.lock().await, &b.room, b.agent.id());
        b.end_turn(false).await;
        assert!(b.outbox_drained().await);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(
            a.peer_events().await.len(),
            1,
            "no wake after the unsubscribe"
        );
        assert!(b.outbox().await.is_empty(), "B no longer owes the follower");
        unlink(&[&a, &b]);
    });
}
