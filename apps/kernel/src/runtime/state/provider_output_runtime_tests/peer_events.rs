//! An agent that follows a peer (neither above nor below it) hears of the peer's turns as
//! `peer_event`, through the same subscribe tool, inbox and wake as every other source.
use super::child_finished::{family, family_with_history_root, Family};
use super::*;
use crate::durable_state::agent_lifecycle::{InboxEvent, Operation, Outcome};

/// Subscribes kid `follower` to kid `source` through the runtime tool entry.
async fn follow(team: &Family, follower: usize, source: &str) -> serde_json::Value {
    let kid = &team.kids[follower];
    let token = team
        .runtime
        .owned
        .provider_store
        .get_run(&kid.child_run)
        .unwrap()
        .runtime_mcp_auth_token()
        .unwrap()
        .to_owned();
    team.runtime
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "chariox.events.subscribe",
            serde_json::json!({
                "task_id": kid.task.task_id,
                "origin_prompt_id": kid.child_prompt.id(),
                "source_id": source,
            }),
        )
        .await
        .unwrap()
        .payload
}

/// Ends the turn of kid `source` the way a provider settlement does.
async fn finish(team: &Family, source: usize, cancelled: bool) {
    let kid = &team.kids[source];
    let store = &team.runtime.owned.durable_state_store;
    let Outcome::Settled { task, correction } = store
        .agent_lifecycle(Operation::Settle {
            room: team.room.clone(),
            agent: kid.child.clone(),
            prompt: kid.child_prompt.id().into(),
            run: kid.child_run.clone(),
            has_answer: !cancelled,
            cancelled,
            now: 2,
        })
        .unwrap()
    else {
        panic!("settled")
    };
    team.runtime
        .owned
        .complete_local_prompt_without_advance(&team.room, &kid.child, Some(&kid.child_run))
        .unwrap();
    team.runtime
        .finish_agent_task_settlement(Some((task, correction)), &kid.child_prompt)
        .await
        .unwrap();
}

fn inbox(team: &Family, agent: &str) -> Vec<InboxEvent> {
    team.runtime
        .owned
        .durable_state_store
        .agent_inbox(&team.room, agent, 0)
        .unwrap()
        .into_iter()
        .filter(|event| event.source_id != team.parent)
        .collect()
}

#[tokio::test]
async fn a_peers_finished_turn_reaches_its_follower_as_a_peer_event() {
    let team = family("peer-event-finished", 2, None, true).await;
    let (follower, peer) = (team.kids[0].child.clone(), team.kids[1].child.clone());
    let registration = follow(&team, 0, &peer).await;
    assert!(
        registration["registration_id"].is_string(),
        "{registration}"
    );
    finish(&team, 1, false).await;

    let events = inbox(&team, &follower);
    assert_eq!(events.len(), 1, "{events:?}");
    let event = &events[0];
    assert_eq!(event.kind, "peer_event", "{event:?}");
    assert_eq!(event.source_id, peer);
    assert_eq!(event.payload["event"], "turn_ended", "{event:?}");
    assert!(
        event.payload.get("public_answer").is_none(),
        "a peer's answer text is not in its event: {event:?}"
    );

    let read = team
        .runtime
        .read_agent_inbox(&team.room, &follower, &serde_json::json!({}))
        .unwrap();
    let view = &read["events"][0];
    assert_eq!(view["kind"], "peer_event");
    assert_eq!(view["source"]["kind"], "agent");
    assert_eq!(view["source"]["id"], peer.as_str());
    assert_eq!(view["source"]["alias"], "child2");
    assert_eq!(view["source"]["relation"], "peer");
    assert!(view["summary"].as_str().unwrap().chars().count() <= 160);
    assert_eq!(read["untrusted"], true);
}

/// The follower is not told what the peer answered, but where to read the exact turn.
#[tokio::test]
async fn a_peer_event_points_at_the_exact_turn_without_its_text() {
    let team = family("peer-event-turn", 2, None, true).await;
    let (follower, peer) = (team.kids[0].child.clone(), team.kids[1].child.clone());
    follow(&team, 0, &peer).await;
    finish(&team, 1, false).await;

    let events = inbox(&team, &follower);
    assert_eq!(events.len(), 1, "{events:?}");
    let turn = &events[0].payload["turn"];
    assert_eq!(turn["agent_id"], peer.as_str(), "{:?}", events[0]);
    assert_eq!(
        turn["turn_ref"],
        team.kids[1].child_prompt.id(),
        "{:?}",
        events[0]
    );
    assert!(events[0].payload.get("public_answer").is_none());
    assert!(turn.get("excerpt").is_none(), "{:?}", events[0]);
}

/// A task keeps its id while its turns change: after a correction the event names the turn that
/// ended the task, which the history tool opens, and not the task's first one.
#[tokio::test]
async fn a_peer_event_after_a_correction_opens_the_turn_that_ended_the_task() {
    use crate::durable_state::agent_lifecycle::Registration;
    use crate::history::{HistoryEventKind, HistoryEventRole, HistoryEventTurnContext};

    let history_root =
        std::env::temp_dir().join(format!("chariox-peer-turn-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&history_root).unwrap();
    let team = family_with_history_root(
        "peer-event-corrected-turn",
        2,
        None,
        true,
        Some(history_root.join("history")),
    )
    .await;
    let (follower, peer) = (team.kids[0].child.clone(), team.kids[1].child.clone());
    follow(&team, 0, &peer).await;
    let store = &team.runtime.owned.durable_state_store;
    let first = &team.kids[1];
    let task_id = first.task.task_id.clone();
    let owner = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap()
        .owner_user_id()
        .to_string();

    // The peer ends its first turn with a delegate it never released: it is corrected once.
    store
        .agent_lifecycle(Operation::RegisterObligation {
            owner: "local".into(),
            room: team.room.clone(),
            agent: peer.clone(),
            prompt: first.child_prompt.id().into(),
            run: Some(first.child_run.clone()),
            id: "late".into(),
            kind: "delegate".into(),
            resource: Some("late-worker".into()),
            now: 2,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Subscribe {
            task: task_id.clone(),
            prompt: first.child_prompt.id().into(),
            registration: Registration {
                id: "late-reg".into(),
                task_id: task_id.clone(),
                source_id: "late-worker".into(),
                obligation_id: Some("late".into()),
                source_cursor: 0,
                live: true,
                peer: false,
            },
        })
        .unwrap();
    let Outcome::Settled {
        correction: true, ..
    } = store
        .agent_lifecycle(Operation::Settle {
            room: team.room.clone(),
            agent: peer.clone(),
            prompt: first.child_prompt.id().into(),
            run: first.child_run.clone(),
            has_answer: true,
            cancelled: false,
            now: 3,
        })
        .unwrap()
    else {
        panic!("the unfinished first turn is corrected")
    };
    store
        .agent_lifecycle(Operation::ReleaseSpawn {
            room: team.room.clone(),
            source: "late-worker".into(),
        })
        .unwrap();
    let corrected = store
        .agent_task(&task_id)
        .unwrap()
        .unwrap()
        .pending_prompt_id
        .expect("the correction turn is pending");
    store
        .agent_lifecycle(Operation::Begin {
            owner: "local".into(),
            room: team.room.clone(),
            agent: peer.clone(),
            prompt: corrected.clone(),
            run: Some(first.child_run.clone()),
            now: 4,
        })
        .unwrap();
    let Outcome::Settled { task, correction } = store
        .agent_lifecycle(Operation::Settle {
            room: team.room.clone(),
            agent: peer.clone(),
            prompt: corrected.clone(),
            run: first.child_run.clone(),
            has_answer: true,
            cancelled: false,
            now: 5,
        })
        .unwrap()
    else {
        panic!("settled")
    };
    assert_ne!(
        task.task_id, task.prompt_id,
        "the task outlived its first turn"
    );
    assert_eq!(task.prompt_id, corrected);

    let append = |prompt: &str, text: &str| {
        team.runtime
            .owned
            .operational_history_store
            .append_operational_event(
                HistoryEventKind::ProviderOutput,
                Some(HistoryEventRole::Assistant),
                Some(text.into()),
                Default::default(),
                HistoryEventTurnContext {
                    session_id: Some(team.room.clone()),
                    agent_id: Some(peer.clone()),
                    turn_id: Some(prompt.into()),
                    prompt_id: Some(prompt.into()),
                    provider_run_id: Some(first.child_run.clone()),
                    public_history_owner_user_id: Some(owner.clone()),
                    ..Default::default()
                },
            )
            .unwrap()
    };
    append(&task_id, "the first turn, before the correction");
    append(&corrected, "the corrected turn that ended the task");
    team.runtime
        .owned
        .complete_local_prompt_without_advance(&team.room, &peer, Some(&first.child_run))
        .unwrap();
    team.runtime
        .finish_agent_task_settlement(Some((task, correction)), &first.child_prompt)
        .await
        .unwrap();

    team.runtime
        .flush_provider_history(&first.child_run)
        .await
        .unwrap();
    let events = inbox(&team, &follower);
    let turn = &events[0].payload["turn"];
    assert_eq!(turn["turn_ref"], corrected.as_str(), "{:?}", events[0]);
    let read = team
        .runtime
        .dispatch_meta_runtime_tool_call_for_agent(
            &team.room,
            &follower,
            "chariox.history.turn",
            serde_json::json!({"agent_ref": peer, "turn_ref": turn["turn_ref"]}),
        )
        .await
        .unwrap();
    assert!(read.ok, "{read:?}");
    let text = read.payload["events"].to_string();
    assert!(
        text.contains("the corrected turn that ended the task"),
        "{text}"
    );
    assert!(!text.contains("before the correction"), "{text}");
    drop(team);
    std::fs::remove_dir_all(history_root).unwrap();
}

/// What a steer asked for is answered like a message: the steerer hears the peer's reply.
#[tokio::test]
async fn a_steer_that_asked_for_the_reply_gets_the_peers_answer_as_its_reply() {
    let team = family("peer-steer-reply", 2, None, true).await;
    let (steerer, peer) = (team.kids[0].child.clone(), team.kids[1].child.clone());
    let store = &team.runtime.owned.durable_state_store;
    let mut steer = crate::durable_state::agent_lifecycle::occurrence(
        &team.room,
        &peer,
        &steerer,
        "steer",
        "message",
        serde_json::json!({"message":"stop editing"}),
    );
    steer.urgent = true;
    steer.reply_requested = true;
    let Outcome::Event(sent) = store
        .agent_lifecycle(Operation::Send {
            task: team.kids[0].task.task_id.clone(),
            prompt: team.kids[0].child_prompt.id().into(),
            event: steer,
        })
        .unwrap()
    else {
        panic!("steer not admitted")
    };
    let target = team.kids[1].child_prompt.id().to_owned();
    store
        .agent_lifecycle(Operation::Attempt {
            room: team.room.clone(),
            agent: peer.clone(),
            sequence: sent.sequence,
            prompt: format!("agent-event-{peer}-{}", sent.sequence),
            target: Some(target),
            run: Some(team.kids[1].child_run.clone()),
            now: 2,
            work: None,
            through: 0,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Receipt {
            room: team.room.clone(),
            agent: peer.clone(),
            sequence: sent.sequence,
            state: "accepted".into(),
            now: 3,
        })
        .unwrap();
    // The peer reads the steer during its turn, as a steer that was never shown is not answered.
    store
        .agent_lifecycle(Operation::MarkShown {
            room: team.room.clone(),
            agent: peer.clone(),
            sequences: vec![sent.sequence],
        })
        .unwrap();
    finish(&team, 1, false).await;

    let heard = inbox(&team, &steerer);
    let reply = heard
        .iter()
        .find(|event| event.kind == "source_completed")
        .unwrap_or_else(|| panic!("the steerer hears the reply: {heard:?}"));
    assert_eq!(
        reply.source_id,
        format!("agent-event-{peer}-{}", sent.sequence)
    );
    assert_eq!(reply.payload["public_answer"]["agent_id"], peer.as_str());
    assert!(reply.payload["public_answer"]["excerpt"].is_string());
}

#[tokio::test]
async fn a_peers_failed_turn_says_so_and_never_the_peers_answer() {
    let team = family("peer-event-failed", 2, None, true).await;
    let (follower, peer) = (team.kids[0].child.clone(), team.kids[1].child.clone());
    follow(&team, 0, &peer).await;
    finish(&team, 1, true).await;

    let events = inbox(&team, &follower);
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].kind, "peer_event");
    assert_eq!(events[0].payload["event"], "turn_failed", "{:?}", events[0]);
    assert_eq!(events[0].payload["success"], false);
    assert!(events[0].payload.get("public_answer").is_none());
}

fn card(id: &str, agent: &str) -> crate::session::RuntimeInteraction {
    use crate::session::{
        RuntimeInteraction, RuntimeInteractionChoice, RuntimeInteractionKind,
        RuntimeInteractionLevel,
    };
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

#[tokio::test]
async fn a_decision_a_peer_raises_never_reaches_its_follower() {
    let team = family("peer-event-decision", 2, None, true).await;
    let (follower, peer) = (team.kids[0].child.clone(), team.kids[1].child.clone());
    follow(&team, 0, &peer).await;
    let _answer = team
        .runtime
        .create_runtime_interaction(&team.room, card("perm-peer", &peer))
        .await
        .unwrap();
    assert!(
        inbox(&team, &follower).is_empty(),
        "the card belongs to the peer's holders: {:?}",
        inbox(&team, &follower)
    );
    let read = team
        .runtime
        .read_agent_inbox(&team.room, &follower, &serde_json::json!({}))
        .unwrap();
    assert!(
        !read.to_string().contains("perm-peer"),
        "the follower's inbox does not name the card: {read}"
    );
}

/// Three peers end their turns together: the follower's one wake carries all three events and
/// starts one model turn.
#[tokio::test]
async fn three_peers_finishing_together_give_one_wake_and_one_turn() {
    let team = family("peer-event-batch", 4, None, true).await;
    let follower = team.kids[0].child.clone();
    let peers: Vec<String> = team.kids[1..].iter().map(|k| k.child.clone()).collect();
    let mut registrations = Vec::new();
    for peer in &peers {
        let reply = follow(&team, 0, peer).await;
        registrations.push(reply["registration_id"].as_str().unwrap().to_owned());
    }
    let kid = &team.kids[0];
    let token = team
        .runtime
        .owned
        .provider_store
        .get_run(&kid.child_run)
        .unwrap()
        .runtime_mcp_auth_token()
        .unwrap()
        .to_owned();
    let now = crate::session::unix_epoch_ms();
    team.runtime
        .dispatch_authenticated_runtime_tool_call(
            &token,
            "chariox.events.yield",
            serde_json::json!({
                "task_id": kid.task.task_id,
                "origin_prompt_id": kid.child_prompt.id(),
                "registration_ids": registrations,
                "inbox_cursor": 0,
                "deadline_ms": now + 600_000,
                "reason": "following three peers",
            }),
        )
        .await
        .unwrap();
    let store = &team.runtime.owned.durable_state_store;
    store
        .agent_lifecycle(Operation::Settle {
            room: team.room.clone(),
            agent: follower.clone(),
            prompt: kid.child_prompt.id().into(),
            run: kid.child_run.clone(),
            has_answer: true,
            cancelled: false,
            now: 2,
        })
        .unwrap();
    team.runtime
        .owned
        .complete_local_prompt_without_advance(&team.room, &follower, Some(&kid.child_run))
        .unwrap();

    // The ledger side of three settlements, then the one delivery the sweep makes.
    for peer in &peers {
        store
            .agent_lifecycle(Operation::SourceOutcome {
                now,
                public_answer: None,
                room: team.room.clone(),
                source: peer.clone(),
                occurrence: format!("task-terminal-{peer}"),
                success: true,
            })
            .unwrap();
    }
    let events = inbox(&team, &follower);
    assert_eq!(events.len(), 3, "{events:?}");
    assert!(events.iter().all(|event| event.kind == "peer_event"));
    team.runtime
        .deliver_agent_inbox(&team.room, &follower)
        .await
        .unwrap();

    let session = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap();
    let prompt = team
        .runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &follower)
        .expect("the wake starts one turn");
    assert!(
        prompt.prompt().contains("3 peer event") && prompt.prompt().chars().count() <= 240,
        "{}",
        prompt.prompt()
    );
    let batch = store
        .agent_events_for_prompt(&team.room, &follower, prompt.id())
        .unwrap();
    assert_eq!(batch.len(), 3, "three events, one prompt id");
    let read = team
        .runtime
        .read_agent_inbox(&team.room, &follower, &serde_json::json!({}))
        .unwrap();
    let sources: Vec<&str> = read["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["source"]["id"].as_str().unwrap())
        .collect();
    for peer in &peers {
        assert!(sources.contains(&peer.as_str()), "{sources:?}");
    }
}

#[tokio::test]
async fn a_peer_address_resolves_only_inside_the_callers_own_rooms() {
    let worktree = crate::test_support::TestWorktree::new("peer-source-resolver");
    let mut config = crate::config::DaemonConfig::for_tests();
    config.room_agent_tools = true;
    let mut app = DaemonApp::bootstrap(config).unwrap();
    let mut rooms = Vec::new();
    for owner in [None, None, Some("someone-else")] {
        let mut request = worktree.session_request();
        if let Some(owner) = owner {
            request = request.with_owner_user_id(owner);
        }
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(request)
            .unwrap();
        rooms.push((session.id().to_owned(), agent.id().to_owned()));
    }
    let owner = app
        .sessions()
        .session_owner_user_id(&rooms[0].0)
        .expect("the room has an owner");
    let (own_room, own_agent) = (rooms[1].0.clone(), rooms[1].1.clone());
    crate::test_support::admit_room_test_turn(&mut app, &own_room, &own_agent);
    let sub = crate::app::KernelSessionService::new(&mut app)
        .spawn_agent(
            crate::agent::CreateAgentRequest::new(&own_room, "dev-stub")
                .with_alias("hidden-helper".to_owned())
                .with_parent_agent_id(&own_agent),
        )
        .unwrap();
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;

    let found = runtime
        .resolve_peer_agent(&owner, &own_room, &own_agent)
        .unwrap();
    assert_eq!(
        (found.session_id(), found.id()),
        (own_room.as_str(), own_agent.as_str())
    );
    // A sub-agent is hidden from outside its team, another owner's room and a room that is not
    // there are not addressable, and an empty name names nobody: one refusal for all of them.
    let refusals = [sub.id(), "hidden-helper", "@hidden-helper", ""]
        .into_iter()
        .map(|reference| runtime.resolve_peer_agent(&owner, &own_room, reference))
        .chain(
            [rooms[2].0.as_str(), "no-such-room"]
                .into_iter()
                .map(|room| runtime.resolve_peer_agent(&owner, room, &rooms[2].1)),
        );
    for refusal in refusals {
        let refusal = refusal.unwrap_err().to_string();
        assert!(
            refusal.contains("agent is unavailable in that room"),
            "{refusal}"
        );
    }
}
