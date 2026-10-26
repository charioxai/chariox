// MP-08 / MP-10 / MP-11: a completed commentary item is not a successful provider turn.
use super::*;

#[test]
fn leased_deferred_commentary_replay_preserves_later_provider_failure() {
    let (mut app, leased, run) = super::explicit_completion_tests::reused_explicit_completion_run();
    let active = app
        .prompt_owner_active_prompt_for_agent_snapshot(
            &leased.backing_session_id,
            &leased.backing_agent_id,
        )
        .unwrap()
        .unwrap();
    // This projection fixture has no remote receipt. Use its actual backing
    // prompt id, so the termination is scoped to this turn, not the prior one.
    app.leased_agents
        .get_mut(&leased.id)
        .unwrap()
        .active_home_prompt_id = Some(active.id().to_string());
    app.terminal_mut().fan_out_output(
        &leased.backing_session_id,
        &run,
        Some(&leased.backing_agent_id),
        TerminalOutputKind::ProviderOutput,
        Some("partial-review".into()),
        vec![leased.backing_attachment_id.clone()],
        b"I will inspect the repository before completing the review.",
    );
    app.terminal_mut().record_assistant_message_completion(
        &leased.backing_session_id,
        &run,
        Some(&leased.backing_agent_id),
        vec![leased.backing_attachment_id.clone()],
        "partial-commentary-complete",
        crate::session::unix_epoch_ms().saturating_add(1),
    );
    let first = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection(&leased.id, &run, false)
        .unwrap()
        .unwrap();
    let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = first.1;
    assert!(
        completions.is_empty(),
        "commentary must wait for the native turn"
    );
    let cached = app
        .leased_agents
        .get(&leased.id)
        .unwrap()
        .replayable_completion
        .as_ref()
        .unwrap();
    assert_eq!(cached.message_id, "partial-commentary-complete");
    assert!(cached.provider_termination.is_none());

    let failure =
        crate::provider::ProviderRunTermination::signal("Killed", crate::session::unix_epoch_ms());
    app.fail_active_prompt_with_termination(
        &leased.backing_session_id,
        &leased.backing_agent_id,
        Some(&run),
        Some(failure.clone()),
        None,
    )
    .unwrap();
    app.providers_mut()
        .mark_run_ended_provider_only(&leased.backing_session_id, &run)
        .unwrap();
    let settled = app
        .completed_git_turn_snapshot_store()
        .latest_projection_for_agent(&leased.backing_session_id, &leased.backing_agent_id)
        .unwrap();
    assert_eq!(settled.prompt_id, active.id());
    assert_eq!(settled.provider_termination.as_ref(), Some(&failure));
    let failed = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection_with_recovery(&leased.id, &run, false, true)
        .unwrap()
        .unwrap();
    let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = failed.1;
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].message_id, "partial-commentary-complete");
    assert_eq!(
        completions[0].provider_termination.as_ref(),
        Some(&failure),
        "deferred commentary completion lost provider termination"
    );
    let retained = app
        .leased_agents
        .get(&leased.id)
        .unwrap()
        .replayable_completion
        .as_ref()
        .unwrap();
    assert_eq!(retained.provider_termination.as_ref(), Some(&failure));
    if let Some((_, RelayPeerEvent::LeasedRuntimeProjection { completions, .. })) =
        RemoteLeaseRuntime::new(&mut app)
            .drain_leased_runtime_projection_with_recovery(&leased.id, &run, false, true)
            .unwrap()
    {
        assert!(
            completions
                .iter()
                .all(|c| c.provider_termination.as_ref() == Some(&failure)),
            "a recovery replay must retain the same failure"
        );
    }
}

// EA2-75: a failed turn is not necessarily evidence that its process crashed.
#[test]
fn leased_failure_without_termination_keeps_its_native_cause() {
    let (mut app, leased, run) = super::explicit_completion_tests::reused_explicit_completion_run();
    let active = app
        .prompt_owner_active_prompt_for_agent_snapshot(
            &leased.backing_session_id,
            &leased.backing_agent_id,
        )
        .unwrap()
        .unwrap();
    app.leased_agents
        .get_mut(&leased.id)
        .unwrap()
        .active_home_prompt_id = Some(active.id().into());
    let message = format!(
        "{}\n\nYou've hit your usage limit.\nTry again later.",
        "Diagnostic context. ".repeat(20)
    );
    assert!(message.len() > 256);
    let expected = crate::provider::ProviderFailure::classify("codex", &message);
    let providers = app.providers.clone();
    let active_turns = app.active_turns.clone();
    let activity = app.prompt_activity.clone();
    let projection = app.agent_runtime_projection_store();
    crate::app::provider_output_prompt_settlement::ProviderOutputPromptSettlement::new(
        &mut app,
        providers,
        active_turns,
        activity,
        projection,
    )
    .fail_for_terminal_failure_if_matches(
        &leased.backing_session_id,
        &run,
        Some(active.id()),
        None,
        &message,
    )
    .unwrap();
    let settled = app
        .completed_git_turn_snapshot_store()
        .latest_projection_for_agent(&leased.backing_session_id, &leased.backing_agent_id)
        .unwrap();
    assert_eq!(settled.provider_termination, None);
    assert_eq!(settled.failure, Some(expected.clone()));
    assert!(
        app.providers
            .get_run(&run)
            .unwrap()
            .terminal_diagnostic()
            .unwrap()
            .len()
            <= 256
    );
    let event = RemoteLeaseRuntime::new(&mut app)
        .drain_leased_runtime_projection_with_recovery(&leased.id, &run, false, true)
        .unwrap()
        .unwrap()
        .1;
    let RelayPeerEvent::LeasedRuntimeProjection { completions, .. } = event;
    assert_eq!(completions.len(), 1);
    let serialized = serde_json::to_value(&completions[0]).unwrap();
    assert_eq!(
        serialized.get("failure"),
        Some(&serde_json::json!(expected))
    );
    assert!(serialized.get("provider_termination").is_none());
}
