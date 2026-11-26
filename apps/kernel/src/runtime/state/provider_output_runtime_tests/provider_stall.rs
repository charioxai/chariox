use super::child_finished::{family, team_with};
use crate::runtime::state::ChildDecisionStatus;

#[tokio::test]
async fn provider_stall_reaches_the_parent_once_with_recovery_actions() {
    let team = family("provider-stall", 2, None, true).await;
    let now = crate::session::unix_epoch_ms();
    let stalled_at = now + 10 * 60 * 1000 + 5_000;
    team.runtime
        .sweep_agent_lifecycle_at_for_test(now)
        .await
        .unwrap();
    assert!(team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap()
        .active_interactions()
        .is_empty());

    for now in [stalled_at, stalled_at + 1_000] {
        team.runtime
            .sweep_agent_lifecycle_at_for_test(now)
            .await
            .unwrap();
    }
    let room = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap();
    let inbox = team
        .runtime
        .owned
        .durable_state_store
        .agent_inbox(&team.room, &team.parent, 0)
        .unwrap();
    assert_eq!(
        inbox
            .iter()
            .filter(|event| event.kind == "child_decision")
            .count(),
        2
    );
    for kid in team.kids {
        let cards = stall_cards(&room, &kid.child);
        assert_eq!(cards.len(), 1);
        let card = cards[0];
        assert!(card.message().contains("child"));
        assert!(card.message().contains("10 minutes"));
        assert!(card.message().contains("provider"));
        for choice in ["resume", "cancel", "start_fresh"] {
            assert!(card.choice(choice).is_some(), "missing {choice}");
        }
        let event = inbox
            .iter()
            .find(|event| event.payload["interaction_id"] == card.id())
            .unwrap();
        assert_eq!(event.payload["child_agent_id"], kid.child);
        assert_eq!(event.payload["detail"], card.message());
    }
}

#[tokio::test]
async fn provider_stall_clears_when_tool_output_returns() {
    let team = team_with("provider-stall-progress", None, true).await;
    let now = crate::session::unix_epoch_ms();
    let stalled_at = now + 10 * 60 * 1000 + 100;
    team.runtime
        .sweep_agent_lifecycle_at_for_test(stalled_at)
        .await
        .unwrap();
    let before = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap();
    assert_eq!(stall_cards(&before, &team.child).len(), 1);

    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    team.runtime
        .apply_owned_structured_output_batch(
            &team.room,
            &team.child_run,
            Vec::new(),
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderTool,
                    merge_key: Some("long-running-tool".into()),
                    bytes:
                        br#"{"id":"long-running-tool","status":"running","output":"first part"}"#
                            .to_vec(),
                }],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    // Time has passed the original quiet deadline. The tool update is the
    // only reason the notice clears even though the turn is still running.
    for at in [stalled_at, stalled_at + 100] {
        team.runtime
            .sweep_agent_lifecycle_at_for_test(at)
            .await
            .unwrap();
    }
    let after = team.runtime.owned.session_snapshot(&team.room).unwrap();
    assert!(stall_cards(&after, &team.child).is_empty());
    let activity = team.runtime.agent_activity_for_session(&after);
    assert!(!activity[&team.child].liveness.stalled);
    assert!(activity[&team.child].busy);
    let inbox = team
        .runtime
        .owned
        .durable_state_store
        .agent_inbox(&team.room, &team.parent, 0)
        .unwrap();
    assert_eq!(
        inbox
            .iter()
            .filter(|event| event.kind == "child_decision")
            .count(),
        1
    );
}

#[tokio::test]
async fn provider_stall_cancel_settles_the_exact_turn() {
    let team = team_with("provider-stall-cancel", None, true).await;
    let stalled_at = crate::session::unix_epoch_ms() + 10 * 60 * 1000 + 5_000;
    team.runtime
        .sweep_agent_lifecycle_at_for_test(stalled_at)
        .await
        .unwrap();
    let room = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap();
    let cards = stall_cards(&room, &team.child);
    assert_eq!(cards.len(), 1);
    let card = cards[0];
    team.runtime
        .resolve_terminal_runtime_interaction(
            &team.room,
            card.id(),
            "cancel",
            None,
            Some(room.owner_user_id()),
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let revision = team
                .runtime
                .session_projection_session_change_sequence(&team.room);
            let room = team.runtime.owned.session_snapshot(&team.room).unwrap();
            if room.active_prompt_for_agent(&team.child).is_none() {
                break;
            }
            team.runtime
                .wait_for_session_projection_session_change_after(&team.room, revision)
                .await;
        }
    })
    .await
    .expect("stall Cancel settles the stopped turn");
    let room = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap();
    assert!(stall_cards(&room, &team.child).is_empty());
}

/// The provider reports a tool row for the child's turn, as a real dispatch sees it.
async fn child_tool(team: &super::child_finished::Team, id: &str, tool: &str, status: &str) {
    if !team
        .runtime
        .owned
        .prompt_activity
        .read()
        .contains_key(&team.child_run)
    {
        // What a real dispatch records when the provider takes the prompt.
        team.runtime.owned.note_prompt_started(&team.child_run);
    }
    team.runtime
        .apply_owned_structured_output_batch(
            &team.room,
            &team.child_run,
            Vec::new(),
            crate::provider::ProviderPromptSignalBatch {
                chunks: vec![crate::provider::ProviderPromptChunk {
                    kind: crate::terminal::TerminalOutputKind::ProviderTool,
                    merge_key: Some(id.into()),
                    bytes: serde_json::to_vec(&serde_json::json!({
                        "id": id, "tool": tool, "status": status, "input": {"command": "sleep 1200"},
                    }))
                    .unwrap(),
                }],
                ..Default::default()
            },
        )
        .await
        .unwrap();
}

/// Row 36 (g): a 20-minute tool call is work. It raises no stall card, tells the parent nothing,
/// and the agent's row says it is running that tool since the tool started.
#[tokio::test]
async fn row36_g_a_long_tool_call_raises_no_card_and_shows_running_its_tool() {
    let team = team_with("row36-g-long-tool", None, true).await;
    let before = crate::session::unix_epoch_ms();
    child_tool(&team, "long-build", "Bash", "running").await;
    let after = crate::session::unix_epoch_ms();
    for at in [after + 20 * 60 * 1000, after + 20 * 60 * 1000 + 1_000] {
        team.runtime
            .sweep_agent_lifecycle_at_for_test(at)
            .await
            .unwrap();
    }
    let room = team.runtime.owned.session_snapshot(&team.room).unwrap();
    assert!(
        stall_cards(&room, &team.child).is_empty(),
        "a running tool call raised a stall card"
    );
    let inbox = team
        .runtime
        .owned
        .durable_state_store
        .agent_inbox(&team.room, &team.parent, 0)
        .unwrap();
    assert!(inbox.iter().all(|event| event.kind != "child_decision"));
    let activity = team.runtime.agent_activity_for_session(&room);
    let liveness = &activity[&team.child].liveness;
    assert!(!liveness.stalled);
    let on = liveness
        .waiting_on
        .as_ref()
        .expect("the row names what runs");
    assert_eq!(serde_json::to_value(on.kind).unwrap(), "tool");
    assert_eq!(on.detail, "Bash");
    assert!(
        (before..=after).contains(&on.since_ms),
        "the row carries the tool's start time"
    );
    assert_eq!(liveness.wait_line().as_deref(), Some("running Bash"));
}

/// Row 36 (h): once the tool has ended, a provider that stays silent is a real hang: the card
/// is raised and the parent is told once.
#[tokio::test]
async fn row36_h_a_provider_hang_after_its_tool_ended_still_raises_the_card() {
    let team = team_with("row36-h-hang", None, true).await;
    child_tool(&team, "short-tool", "Bash", "running").await;
    child_tool(&team, "short-tool", "Bash", "completed").await;
    let stalled_at = crate::session::unix_epoch_ms() + 10 * 60 * 1000 + 5_000;
    for at in [stalled_at, stalled_at + 1_000] {
        team.runtime
            .sweep_agent_lifecycle_at_for_test(at)
            .await
            .unwrap();
    }
    let room = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap();
    assert_eq!(stall_cards(&room, &team.child).len(), 1);
    let inbox = team
        .runtime
        .owned
        .durable_state_store
        .agent_inbox(&team.room, &team.parent, 0)
        .unwrap();
    assert_eq!(
        inbox
            .iter()
            .filter(|event| event.kind == "child_decision")
            .count(),
        1
    );
}

/// Row 36 (i): the parent holds its child's stall card and answers it with the owner's
/// authority; a sibling that does not hold it cannot.
#[tokio::test]
async fn row36_i_the_parent_answers_its_childs_stall_card_and_a_sibling_cannot() {
    let family = super::child_finished::family("row36-i-parent", 2, None, true).await;
    let stalled_at = crate::session::unix_epoch_ms() + 10 * 60 * 1000 + 5_000;
    family
        .runtime
        .sweep_agent_lifecycle_at_for_test(stalled_at)
        .await
        .unwrap();
    let room = family
        .runtime
        .owned
        .session_store
        .get_session(&family.room)
        .unwrap();
    let (first, second) = (&family.kids[0].child, &family.kids[1].child);
    let card = stall_cards(&room, first)[0].id().to_owned();
    let refused = family
        .runtime
        .answer_child_decision(&family.room, second, &card, "cancel")
        .await;
    assert!(
        !matches!(refused, Ok(ChildDecisionStatus::Answered)),
        "a sibling answered a card it does not hold: {refused:?}"
    );
    let status = family
        .runtime
        .answer_child_decision(&family.room, &family.parent, &card, "cancel")
        .await
        .unwrap();
    assert_eq!(status, ChildDecisionStatus::Answered);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let revision = family
                .runtime
                .session_projection_session_change_sequence(&family.room);
            let room = family.runtime.owned.session_snapshot(&family.room).unwrap();
            if room.active_prompt_for_agent(first).is_none() {
                break;
            }
            family
                .runtime
                .wait_for_session_projection_session_change_after(&family.room, revision)
                .await;
        }
    })
    .await
    .expect("the parent's Cancel stops the stalled turn");
    let audit = family
        .runtime
        .owned
        .durable_state_store
        .load_subject_events_by_kind(&card, "interaction.answered", 10)
        .unwrap();
    assert_eq!(audit[0].payload["answered_by_agent"], family.parent);
}

/// Row 36 (j): Resume from the parent stops the stalled turn and sends its request again in the
/// same agent.
#[tokio::test]
async fn row36_j_the_parents_resume_restarts_the_stalled_turn() {
    let team = team_with("row36-j-resume", None, true).await;
    let stalled_at = crate::session::unix_epoch_ms() + 10 * 60 * 1000 + 5_000;
    team.runtime
        .sweep_agent_lifecycle_at_for_test(stalled_at)
        .await
        .unwrap();
    let room = team
        .runtime
        .owned
        .session_store
        .get_session(&team.room)
        .unwrap();
    let card = stall_cards(&room, &team.child)[0].id().to_owned();
    let status = team
        .runtime
        .answer_child_decision(&team.room, &team.parent, &card, "resume")
        .await
        .unwrap();
    assert_eq!(status, ChildDecisionStatus::Answered);
    let original = team.child_prompt.id().to_owned();
    let retried = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let revision = team
                .runtime
                .session_projection_session_change_sequence(&team.room);
            let room = team.runtime.owned.session_snapshot(&team.room).unwrap();
            let next = room
                .active_prompt_for_agent(&team.child)
                .into_iter()
                .chain(
                    room.queued_prompts_for_agent(&team.child)
                        .into_iter()
                        .flatten(),
                )
                .find(|prompt| prompt.id() != original)
                .cloned();
            if let Some(next) = next {
                break next;
            }
            team.runtime
                .wait_for_session_projection_session_change_after(&team.room, revision)
                .await;
        }
    })
    .await
    .expect("the parent's Resume sent the stalled request again");
    assert_eq!(retried.prompt(), team.child_prompt.prompt());
    assert_eq!(retried.target_agent_id(), team.child);
}

fn stall_cards<'a>(
    room: &'a crate::session::RuntimeSession,
    agent: &str,
) -> Vec<&'a crate::session::RuntimeInteraction> {
    room.active_interactions()
        .iter()
        .filter(|card| {
            card.title() == Some("Agent stalled") && card.related_agent_id() == Some(agent)
        })
        .collect()
}
