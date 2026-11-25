//! A card is shown only when an action is needed that the agent cannot take itself. A usage stop
//! is status on the agent's row (paused, resets <time>) and resumes by itself at the reset; the
//! person gets a card only when they have a real choice (another account with room, no
//! orchestrator owning the switch) or have asked to decide ("Ask me", at the reset).
use super::*;

fn default_config() -> crate::config::DaemonConfig {
    // The person never set the policy: the default.
    crate::config::DaemonConfig::for_tests()
}

fn card_titles(team: &Team, lane: usize) -> Vec<String> {
    team.rt()
        .owned
        .session_store
        .get_session(&team.session_id)
        .unwrap()
        .active_interactions()
        .iter()
        .filter(|card| card.related_agent_id() == Some(team.lanes[lane].agent_id.as_str()))
        .map(|card| card.title().unwrap_or_default().to_owned())
        .collect()
}

/// What the agent's row shows of a usage stop.
fn row_stop(team: &Team, lane: usize) -> Option<crate::agent::UsageLimitStop> {
    team.agent(lane).usage_limit_wait().cloned()
}

// C1
#[tokio::test]
async fn a_usage_stop_with_no_other_account_shows_no_card_and_resumes_at_the_reset() {
    let team = team(default_config(), 1).await;
    team.hit_the_limit(0).await;

    assert!(team.cards(0).is_empty(), "{:?}", team.cards(0));
    let row = row_stop(&team, 0).expect("the row shows the agent paused");
    let reset = row.resumes_at_ms.expect("and when it resets");
    assert!(matches!(row.state, UsageLimitStopState::Armed { at_ms } if at_ms == reset));
    assert!(!team.has_turn(0));

    tick(team.rt(), reset - 1).await;
    assert!(!team.has_turn(0), "nothing before the reset");
    tick(team.rt(), reset).await;
    assert!(team.has_turn(0), "the agent goes on by itself at the reset");
    assert!(team.cards(0).is_empty());
}

// C2
#[tokio::test]
async fn a_second_account_with_room_raises_one_usage_card_and_not_now_raises_no_other() {
    let mut config = default_config();
    config.room_agent_tools = true;
    let team = team_claude(config, 1).await.with_second_account();
    let task_id = owe_work(&team, 0);
    team.hit_the_limit(0).await;

    let cards = team.cards(0);
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert!(cards[0].starts_with("usage-limit-"), "{cards:?}");
    assert_eq!(card_titles(&team, 0), ["Usage limit reached"]);
    let session = team
        .rt()
        .owned
        .session_store
        .get_session(&team.session_id)
        .unwrap();
    let card = session
        .active_interactions()
        .iter()
        .find(|card| card.id() == cards[0])
        .unwrap();
    let labels = card
        .choices()
        .iter()
        .map(|choice| choice.label().to_owned())
        .collect::<Vec<_>>();
    assert!(
        labels
            .iter()
            .any(|label| label.starts_with("Continue now on ")),
        "{labels:?}"
    );
    assert!(
        labels
            .iter()
            .all(|label| label != "Cancel" && label != "Start a fresh agent"),
        "{labels:?}"
    );

    team.answer(&cards[0], "not_now").await;
    team.until(0, |stop| stop.state == UsageLimitStopState::Declined)
        .await;
    // The task the stop blocked stays the stop's: minutes later no generic card appears.
    let task = task_of(&team, &task_id);
    team.rt().ensure_task_owner_interaction(task).await.unwrap();
    tick(team.rt(), crate::session::unix_epoch_ms() + 600_000).await;
    assert!(team.cards(0).is_empty(), "{:?}", team.cards(0));
    assert!(
        row_stop(&team, 0).is_some_and(|stop| stop.state == UsageLimitStopState::Declined),
        "the row still says paused"
    );
}

// C3
#[tokio::test]
async fn an_orchestrator_that_owns_the_switch_gets_the_stop_and_the_person_no_card() {
    let team = team_claude(default_config(), 2).await.with_second_account();
    team.rt()
        .set_agent_parent(
            OWNER,
            crate::local::SetAgentParentRequest {
                session_id: team.session_id.clone(),
                agent_id: team.lanes[1].agent_id.clone(),
                parent_agent_id: Some(team.lanes[0].agent_id.clone()),
            },
        )
        .await
        .unwrap();
    team.hit_the_limit(1).await;

    assert!(team.cards(1).is_empty(), "{:?}", team.cards(1));
    let stop = team.stop(1).unwrap();
    assert!(
        matches!(stop.state, UsageLimitStopState::Armed { .. }),
        "{stop:?}"
    );
    // The parent takes the resume back, as before; no card comes of it.
    team.rt()
        .resolve_usage_limit_stop(
            &team.session_id,
            &team.lanes[1].agent_id,
            OWNER,
            Some(&team.lanes[0].agent_id),
            crate::local::UsageLimitStopAction::Cancel,
        )
        .await
        .unwrap();
    assert_eq!(team.stop(1).unwrap().state, UsageLimitStopState::Declined);
    assert!(team.cards(1).is_empty());
}

// C7
#[tokio::test]
async fn ask_me_shows_no_card_at_the_stop_and_one_at_the_reset() {
    let team = team(config(UsageLimitResume::Ask), 1).await;
    team.hit_the_limit(0).await;
    let reset = team.stop(0).unwrap().resumes_at_ms.unwrap();

    assert!(
        team.cards(0).is_empty(),
        "nothing at the stop: {:?}",
        team.cards(0)
    );
    assert_eq!(
        row_stop(&team, 0).map(|stop| stop.state),
        Some(UsageLimitStopState::Asking),
        "the row says paused until the reset"
    );
    tick(team.rt(), reset - 1).await;
    assert!(team.cards(0).is_empty(), "nothing before the reset");
    assert!(!team.has_turn(0), "Ask me never resumes without the person");

    // The reset has come (the stop's own clock is the provider's named time).
    let now = crate::session::unix_epoch_ms();
    let late = team
        .stop(0)
        .map(|stop| stop.resumes_at_ms.unwrap())
        .unwrap()
        .max(now);
    tick(team.rt(), late).await;
    team.make_reset_due(0);
    tick(team.rt(), late).await;
    let cards = team.cards(0);
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert!(cards[0].ends_with("-back"), "{cards:?}");
    assert_eq!(card_titles(&team, 0), ["Usage is back"]);
    // Asking again does not raise a second card.
    tick(team.rt(), late + 1_000).await;
    assert_eq!(team.cards(0).len(), 1);

    team.answer(&cards[0], "not_now").await;
    team.until(0, |stop| stop.state == UsageLimitStopState::Declined)
        .await;
    tick(team.rt(), late + 3_600_000).await;
    assert!(team.cards(0).is_empty(), "Leave paused: no further card");
    assert!(
        row_stop(&team, 0).is_some_and(|stop| stop.state == UsageLimitStopState::Declined),
        "the row still says paused"
    );
    assert!(!team.has_turn(0));
}

#[tokio::test]
async fn ask_me_resume_at_the_reset_resumes_the_agent() {
    let team = team(config(UsageLimitResume::Ask), 1).await;
    team.hit_the_limit(0).await;
    team.make_reset_due(0);
    tick(team.rt(), crate::session::unix_epoch_ms()).await;
    let cards = team.cards(0);
    assert_eq!(cards.len(), 1, "{cards:?}");

    team.answer(&cards[0], "resume_when_back").await;
    team.until(0, |stop| {
        matches!(stop.state, UsageLimitStopState::Armed { .. })
    })
    .await;
    tick(team.rt(), crate::session::unix_epoch_ms() + 1_000).await;
    assert!(team.has_turn(0), "Resume goes on");
    assert!(team.cards(0).is_empty());
}

/// Union review P1: at the reset an explicit "Ask me" stop is the person's to decide. Ticking on
/// past the reset must keep the stop and its one "Usage is back" decision until they answer; the
/// reset must not apply `StopEvent::Reset`, which would drop the stop and close the decision.
#[tokio::test]
async fn ask_me_keeps_the_stop_and_its_one_decision_through_the_reset_until_answered() {
    let team = team(config(UsageLimitResume::Ask), 1).await;
    team.hit_the_limit(0).await;
    team.make_reset_due(0);
    let now = crate::session::unix_epoch_ms();
    for after_ms in [0, 1_000, 60_000, 3_600_000] {
        tick(team.rt(), now + after_ms).await;
        let stop = team
            .stop(0)
            .unwrap_or_else(|| panic!("the stop is still there {after_ms} ms past the reset"));
        assert_eq!(stop.state, UsageLimitStopState::Asking, "{stop:?}");
        let cards = team.cards(0);
        assert_eq!(cards.len(), 1, "exactly one decision is up: {cards:?}");
        assert!(cards[0].ends_with("-back"), "{cards:?}");
        assert_eq!(card_titles(&team, 0), ["Usage is back"]);
        assert!(!team.has_turn(0), "Ask me never resumes without the person");
    }
    assert!(
        row_stop(&team, 0).is_some_and(|stop| stop.state == UsageLimitStopState::Asking),
        "the row still says paused"
    );

    let cards = team.cards(0);
    team.answer(&cards[0], "resume_when_back").await;
    team.until(0, |stop| {
        matches!(stop.state, UsageLimitStopState::Armed { .. })
    })
    .await;
    tick(team.rt(), now + 3_700_000).await;
    assert!(team.has_turn(0), "choosing Resume resumes the agent");
    assert!(team.cards(0).is_empty(), "the answered decision is gone");
}

#[tokio::test]
async fn agent_resume_goes_on_after_leave_paused() {
    let team = team(config(UsageLimitResume::Ask), 1).await;
    team.hit_the_limit(0).await;
    team.make_reset_due(0);
    tick(team.rt(), crate::session::unix_epoch_ms()).await;
    team.answer(&team.cards(0)[0], "not_now").await;
    team.until(0, |stop| stop.state == UsageLimitStopState::Declined)
        .await;

    // The person, at the CLI: `agent resume`.
    team.rt()
        .resolve_usage_limit_stop(
            &team.session_id,
            &team.lanes[0].agent_id,
            OWNER,
            None,
            crate::local::UsageLimitStopAction::ResumeWhenBack,
        )
        .await
        .unwrap();
    tick(team.rt(), crate::session::unix_epoch_ms() + 1_000).await;
    assert!(team.has_turn(0));
}

#[tokio::test]
async fn ask_me_with_a_spare_account_still_shows_no_card_at_the_stop_and_one_at_the_reset() {
    let team = team_claude(config(UsageLimitResume::Ask), 1)
        .await
        .with_second_account();
    team.hit_the_limit(0).await;
    let reset = team.stop(0).unwrap().resumes_at_ms.unwrap();

    assert!(
        team.cards(0).is_empty(),
        "nothing at the stop: {:?}",
        team.cards(0)
    );
    tick(team.rt(), reset - 1).await;
    assert!(team.cards(0).is_empty(), "nothing before the reset");

    team.make_reset_due(0);
    tick(team.rt(), crate::session::unix_epoch_ms()).await;
    let cards = team.cards(0);
    assert_eq!(cards.len(), 1, "{cards:?}");
    assert!(cards[0].ends_with("-back"), "{cards:?}");
    assert_eq!(card_titles(&team, 0), ["Usage is back"]);
}
