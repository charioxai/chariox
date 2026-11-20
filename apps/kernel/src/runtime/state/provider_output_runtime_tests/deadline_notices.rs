//! TL1 / TL2 / TL4: a time limit is announced before and after it ends, and what it does not
//! stop is said: a sub-agent keeps working and is told who hears it now.
use super::child_finished::team_with;
use super::*;

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
