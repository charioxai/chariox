use super::child_finished::{family, team_with};

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
