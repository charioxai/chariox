//! An agent that follows a peer (neither above nor below it) hears of the peer's turns as
//! `peer_event`, through the same subscribe tool, inbox and wake as every other source.
use super::child_finished::{family, Family};
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
    assert!(registration["registration_id"].is_string(), "{registration}");
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

    let session = team.runtime.owned.session_store.get_session(&team.room).unwrap();
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
