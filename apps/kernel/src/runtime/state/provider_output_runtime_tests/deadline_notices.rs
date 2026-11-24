//! TL1 / TL2 / TL4: a time limit is announced before and after it ends, and what it does not
//! stop is said: a sub-agent keeps working and is told who hears it now.
use super::child_finished::team_with;

async fn set_deadline(team: &super::child_finished::Team, agent: &str, deadline_ms: u64) {
    let owner = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap()
        .owner_user_id()
        .to_string();
    team.runtime
        .set_agent_deadline(
            &owner,
            crate::local::SetAgentDeadlineRequest {
                session_id: team.room.clone(),
                agent_id: agent.to_string(),
                deadline_ms: Some(deadline_ms),
            },
        )
        .await
        .unwrap();
}

fn kinds(team: &super::child_finished::Team, agent: &str) -> Vec<String> {
    team.runtime
        .owned
        .durable_state_store
        .agent_inbox(&team.room, agent, 0)
        .unwrap()
        .into_iter()
        .map(|event| event.kind)
        .collect()
}

fn notices(team: &super::child_finished::Team, agent: &str) -> Vec<String> {
    team.runtime.agent_notices_for_test(&team.room, agent)
}

// TL1: the Room and the agent hear of a limit 10 minutes and 1 minute before it ends, once each.
#[tokio::test]
async fn a_limit_is_announced_ten_minutes_and_one_minute_ahead_once_each() {
    let team = team_with("deadline-warnings", None, true).await;
    let now = crate::session::unix_epoch_ms();
    set_deadline(&team, &team.parent, now + 9 * 60_000).await;
    for _ in 0..2 {
        team.runtime.tick_agent_deadlines(now).await.unwrap();
    }
    let ahead = notices(&team, &team.parent);
    assert_eq!(
        ahead
            .iter()
            .filter(|notice| notice.contains("ends in about 10 minutes"))
            .count(),
        1,
        "{ahead:?}"
    );
    assert!(
        ahead
            .iter()
            .any(|notice| notice.contains("`agent deadline")),
        "the warning says how to extend: {ahead:?}"
    );
    assert!(
        kinds(&team, &team.parent).contains(&"time_limit_warning".to_string()),
        "the agent hears of it while it has work"
    );
    // The last minute is a second warning, not a repeat.
    team.runtime
        .tick_agent_deadlines(now + 8 * 60_000 + 10_000)
        .await
        .unwrap();
    let last = notices(&team, &team.parent);
    assert_eq!(
        last.iter()
            .filter(|notice| notice.contains("ends in about 1 minute"))
            .count(),
        1,
        "{last:?}"
    );
    // An extension is a new limit: it is announced again when it comes near.
    set_deadline(&team, &team.parent, now + 9 * 60_000 + 30_000).await;
    team.runtime
        .tick_agent_deadlines(now + 9 * 60_000)
        .await
        .unwrap();
    assert_eq!(
        notices(&team, &team.parent)
            .iter()
            .filter(|notice| notice.contains("ends in about 1 minute"))
            .count(),
        2
    );
}

// TL2 + TL4: the Room is told what stopped and what did not; the sub-agent goes on and learns
// that no agent above it runs.
#[tokio::test]
async fn a_reached_limit_says_what_stopped_and_leaves_the_sub_agent_told() {
    let team = team_with("deadline-reached-notice", None, true).await;
    set_deadline(&team, &team.parent, crate::session::unix_epoch_ms() + 30).await;
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    for _ in 0..2 {
        team.runtime
            .tick_agent_deadlines(crate::session::unix_epoch_ms())
            .await
            .unwrap();
    }
    let room = notices(&team, &team.parent);
    let ended = room
        .iter()
        .filter(|notice| notice.contains("time limit ended at"))
        .collect::<Vec<_>>();
    assert_eq!(ended.len(), 1, "told once: {room:?}");
    let text = ended[0];
    assert!(text.contains("Stopped:"), "{text}");
    assert!(text.contains("Its sub-agents were not stopped"), "{text}");
    assert!(
        text.contains("`agent deadline") && text.contains("then prompts it"),
        "the next step is named: {text}"
    );
    // The child is still working and was told its parent no longer hears it.
    assert_ne!(
        team.runtime
            .owned
            .provider_store
            .get_run(&team.child_run)
            .unwrap()
            .state(),
        crate::provider::ProviderRunState::Ended
    );
    let told = team
        .runtime
        .owned
        .durable_state_store
        .agent_inbox(&team.room, &team.child, 0)
        .unwrap()
        .into_iter()
        .find(|event| event.kind == "parent_time_limit_ended")
        .expect("the child hears that its parent's limit ended");
    let message = told.payload["message"].as_str().unwrap();
    assert!(
        message.contains("You keep running") && message.contains("No agent above you runs now"),
        "{message}"
    );
}

// TL2: a cleanup that takes more than one tick still reports what the limit found to stop.
#[tokio::test]
async fn a_notice_after_a_cleanup_retry_still_names_what_the_limit_stopped() {
    let team = team_with("deadline-notice-retry", None, true).await;
    set_deadline(&team, &team.parent, crate::session::unix_epoch_ms() + 30).await;
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    let deadline = team
        .runtime
        .owned
        .durable_state_store
        .agent_deadline(&team.room, &team.parent)
        .unwrap()
        .unwrap();
    // The first attempt reads the work and removes it, but does not finish: the next tick
    // finds the agent already empty.
    assert!(team
        .runtime
        .deadline_work_at_reach(&deadline)
        .active_prompt
        .is_some());
    team.runtime
        .stop_one_deadline_agent(&team.room, &team.parent)
        .await
        .unwrap();
    assert!(team
        .runtime
        .deadline_work(&team.room, &team.parent)
        .active_prompt
        .is_none());
    for _ in 0..2 {
        team.runtime
            .tick_agent_deadlines(crate::session::unix_epoch_ms())
            .await
            .unwrap();
    }
    let room = notices(&team, &team.parent);
    let ended = room
        .iter()
        .filter(|notice| notice.contains("time limit ended at"))
        .collect::<Vec<_>>();
    assert_eq!(ended.len(), 1, "told once: {room:?}");
    assert!(ended[0].contains("Stopped:"), "{}", ended[0]);
    assert!(!ended[0].contains("It had no running turn"), "{}", ended[0]);
}

// TL1 / TL4: a notice reaches an agent in the middle of its turn. An active turn takes only urgent
// rows, so the warning and the parent-ended notice must be the urgent front of the inbox.
#[tokio::test]
async fn time_limit_notices_are_urgent_so_an_active_turn_takes_them() {
    let team = team_with("deadline-notices-urgent", None, true).await;
    let now = crate::session::unix_epoch_ms();
    set_deadline(&team, &team.parent, now + 9 * 60_000).await;
    team.runtime.tick_agent_deadlines(now).await.unwrap();
    let store = &team.runtime.owned.durable_state_store;
    let warning = store
        .agent_urgent_delivery_front(&team.room, &team.parent)
        .unwrap()
        .expect("a warning waits for the active turn");
    assert_eq!(warning.kind, "time_limit_warning");
    assert!(
        warning.urgent,
        "the warning is delivered into the running turn"
    );

    // The parent's limit ends while its sub-agent works: the sub-agent hears of it in its turn.
    let deadline = crate::session::unix_epoch_ms();
    set_deadline(&team, &team.parent, deadline + 30).await;
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    team.runtime
        .tick_agent_deadlines(crate::session::unix_epoch_ms())
        .await
        .unwrap();
    let told = store
        .agent_inbox(&team.room, &team.child, 0)
        .unwrap()
        .into_iter()
        .find(|event| event.kind == "parent_time_limit_ended")
        .expect("the sub-agent is told");
    assert!(told.urgent, "the parent-ended notice is urgent");
    assert_eq!(
        store
            .agent_urgent_delivery_front(&team.room, &team.child)
            .unwrap()
            .map(|event| event.sequence),
        Some(told.sequence),
        "an active turn of the sub-agent takes it first"
    );
}

// TL4: three sub-agents, two mid-turn and one yielded on a wake, all go on when their parent's
// limit ends, and each is told that no agent above it runs.
#[tokio::test]
async fn a_parents_limit_leaves_three_sub_agents_running_and_each_is_told() {
    use crate::durable_state::agent_lifecycle::{AgentWake, Operation};
    let family = super::child_finished::family("deadline-three-kids", 3, None, true).await;
    let (runtime, room) = (&family.runtime, family.room.clone());
    let store = &runtime.owned.durable_state_store;
    // The second sub-agent ends its turn on a wake that is still armed.
    let parked = &family.kids[1];
    let (task, prompt) = (
        parked.task.task_id.clone(),
        parked.child_prompt.id().to_string(),
    );
    let now = crate::session::unix_epoch_ms();
    let wake: AgentWake = serde_json::from_value(serde_json::json!({
        "id": "parked-wake", "task_id": task, "room_id": room, "agent_id": parked.child,
        "registration_id": "completion-parked-wake", "kind": "timer", "label": "wait for a build",
        "state": "scheduled", "created_at_ms": now, "verified_at_ms": null,
        "next_due_ms": now + 3_600_000, "interval_ms": null, "command": [],
        "match_text": null, "matched_at_ms": null, "pid": null, "exit_code": null,
        "fire_count": 0, "missed_fires": 0, "last_fired_at_ms": null, "last_sequence": null,
        "last_delivery": null, "last_delivered_at_ms": null, "last_acknowledged_at_ms": null,
        "alerted_sequence": null
    }))
    .unwrap();
    store
        .agent_lifecycle(Operation::CreateWake {
            task: task.clone(),
            prompt: prompt.clone(),
            wake,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Yield {
            task: task.clone(),
            prompt: prompt.clone(),
            registrations: vec!["completion-parked-wake".into()],
            cursor: 0,
            deadline: Some(now + 3_900_000),
            reason: "wait for the build".into(),
            now,
        })
        .unwrap();
    store
        .agent_lifecycle(Operation::Settle {
            room: room.clone(),
            agent: parked.child.clone(),
            prompt: prompt.clone(),
            run: parked.child_run.clone(),
            has_answer: true,
            cancelled: false,
            now,
        })
        .unwrap();
    runtime
        .owned
        .complete_local_prompt_without_advance(&room, &parked.child, Some(&parked.child_run))
        .unwrap();

    let owner = runtime
        .owned
        .session_store
        .get_session(&room)
        .unwrap()
        .owner_user_id()
        .to_string();
    runtime
        .set_agent_deadline(
            &owner,
            crate::local::SetAgentDeadlineRequest {
                session_id: room.clone(),
                agent_id: family.parent.clone(),
                deadline_ms: Some(crate::session::unix_epoch_ms() + 30),
            },
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    for _ in 0..2 {
        runtime
            .tick_agent_deadlines(crate::session::unix_epoch_ms())
            .await
            .unwrap();
    }

    let session = runtime.owned.session_store.get_session(&room).unwrap();
    for (index, kid) in family.kids.iter().enumerate() {
        assert!(
            store.agent_deadline(&room, &kid.child).unwrap().is_none(),
            "kid {index} has no limit"
        );
        if index == 1 {
            assert!(
                !runtime
                    .owned
                    .unsettled_agent_wakes(&room, &kid.child)
                    .unwrap()
                    .is_empty(),
                "the parked sub-agent's wake was not cancelled"
            );
        } else {
            assert!(
                runtime
                    .owned
                    .prompt_state_owner
                    .active_prompt_for_agent(&session, &kid.child)
                    .is_some(),
                "kid {index} is still in its turn"
            );
            assert_ne!(
                runtime
                    .owned
                    .provider_store
                    .get_run(&kid.child_run)
                    .unwrap()
                    .state(),
                crate::provider::ProviderRunState::Ended
            );
        }
        let told = store
            .agent_inbox(&room, &kid.child, 0)
            .unwrap()
            .into_iter()
            .find(|event| event.kind == "parent_time_limit_ended")
            .unwrap_or_else(|| panic!("kid {index} was not told"));
        assert!(
            told.payload["message"]
                .as_str()
                .unwrap()
                .contains("You keep running"),
            "{told:?}"
        );
    }
    let text = runtime
        .agent_notices_for_test(&room, &family.parent)
        .into_iter()
        .find(|notice| notice.contains("time limit ended at"))
        .expect("the Room is told");
    for alias in ["child", "child2", "child3"] {
        assert!(text.contains(alias), "{alias} is named: {text}");
    }
}
