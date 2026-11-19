// A Claude headless start that stops at a screen nothing in Chariox answers fails the turn,
// naming that screen, instead of waiting for a cancel. The agent has no provider session yet,
// as on a fresh kernel.
use super::*;

#[tokio::test]
async fn a_claude_screen_nothing_answers_fails_the_start_naming_it() {
    crate::test_support::isolated_env_test!();
    TEST_PROMPT_TIMER_MS.store(5000, std::sync::atomic::Ordering::Relaxed);
    let (_worktree, runtime, session_id, agent_id, _source_id, run, dispatch) =
        runtime_with_claude_headless_cli(
            "#!/bin/sh\nprintf 'Welcome to Claude Code\\nChoose the text style that looks best\\n'\nwhile IFS= read -r line; do :; done\n",
            None,
        )
        .await;
    runtime
        .with_app_side_effect(|app| {
            crate::app::ProviderLaunchProcessRuntime::new(app).spawn_for_launch(&run)
        })
        .await
        .unwrap();
    runtime.spawn_prompt_dispatch(dispatch, runtime.provider_runtime_lanes.clone());
    let reason = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            runtime.pump_ready_provider_output_run(run.id()).await;
            let history = runtime
                .owned
                .operational_history_store
                .load_session_events(&session_id, Some(&agent_id))
                .unwrap();
            if let Some(text) = history
                .iter()
                .filter_map(|event| event.content.as_deref())
                .find(|text| text.contains("acknowledge Claude headless prompt"))
            {
                break text.to_string();
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await;
    let settled = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let session = runtime
                .owned
                .session_store
                .get_session(&session_id)
                .unwrap();
            if runtime
                .owned
                .prompt_state_owner
                .active_prompt_for_agent(&session, &agent_id)
                .is_none()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await;
    TEST_PROMPT_TIMER_MS.store(0, std::sync::atomic::Ordering::Relaxed);
    runtime
        .retire_owned_provider_run_after_terminal_failure(&session_id, run.id())
        .await;
    settled.expect("the failed start must end the turn, not leave its prompt active");
    let reason = reason.expect("the start must fail once its screen stops changing");
    assert!(
        reason.contains("Welcome to Claude Code"),
        "the failure must name the screen: {reason}"
    );
    assert!(
        reason.contains("the agent's terminal"),
        "the failure must say what to do: {reason}"
    );
}

#[tokio::test]
async fn a_workspace_trust_card_left_open_keeps_the_start_alive_past_the_start_budget() {
    crate::test_support::isolated_env_test!();
    TEST_PROMPT_TIMER_MS.store(5000, std::sync::atomic::Ordering::Relaxed);
    let (_worktree, runtime, session_id, agent_id, _source_id, run, dispatch) =
        runtime_with_claude_headless_cli(
            "#!/bin/sh\nprintf 'Quick safety check\\nDo you trust this folder?\\n'\nwhile IFS= read -r line; do :; done\n",
            None,
        )
        .await;
    runtime
        .with_app_side_effect(|app| {
            crate::app::ProviderLaunchProcessRuntime::new(app).spawn_for_launch(&run)
        })
        .await
        .unwrap();
    let prompt_id = dispatch.prompt_id.clone();
    let marker = std::path::Path::new(&run.pty_env()["CHARIOX_CLAUDE_NATIVE_CONTEXT"])
        .with_file_name("active-prompt-id");
    // The state the output pump leaves while the owner's trust card is open; the card's own
    // 300 s deadline governs from here.
    std::fs::write(
        &marker,
        "startup-workspace-trust:claude-workspace-review-test",
    )
    .unwrap();
    // The card as the output pump raises it: open in the session until the owner answers.
    let card = crate::session::RuntimeInteraction::new(
        "claude-workspace-review-test",
        agent_id.clone(),
        crate::session::RuntimeInteractionKind::Permission,
        crate::session::RuntimeInteractionLevel::Warning,
        Some("Trust this Claude Code workspace?".to_string()),
        "trust the folder",
        vec![crate::session::RuntimeInteractionChoice::new(
            "allow_once",
            "Trust workspace",
            "allow",
            None,
        )],
        None,
        Some(300),
        Some("deny".to_string()),
    );
    let card_runtime = runtime.clone();
    let card_session = session_id.clone();
    let card_task = tokio::spawn(async move {
        crate::runtime::native_interaction_bridge::request_runtime_interaction_with_timeout(
            &card_runtime,
            &card_session,
            card,
            "test_trust_card",
            None,
        )
        .await
    });
    runtime.spawn_prompt_dispatch(dispatch, runtime.provider_runtime_lanes.clone());
    // Well past the idle bound.
    let waited_until = std::time::Instant::now() + std::time::Duration::from_secs(8);
    while std::time::Instant::now() < waited_until {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let session = runtime
        .owned
        .session_store
        .get_session(&session_id)
        .unwrap();
    let still_active = runtime
        .owned
        .prompt_state_owner
        .active_prompt_for_agent(&session, &agent_id)
        .is_some();
    let failed_start = runtime
        .owned
        .operational_history_store
        .load_session_events(&session_id, Some(&agent_id))
        .unwrap()
        .iter()
        .filter_map(|event| event.content.as_deref())
        .any(|text| text.contains("acknowledge Claude headless prompt"));
    let run_alive = runtime
        .owned
        .provider_store
        .get_run(run.id())
        .map(|run| run.state() != crate::provider::ProviderRunState::Ended)
        .unwrap_or(false);
    // The owner answers: the prompt is written and the acknowledgement clock starts fresh, which
    // ends this start (the fixture Claude never acknowledges).
    std::fs::write(&marker, format!("injected:{prompt_id}")).unwrap();
    let acknowledged_late = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let failed = runtime
                .owned
                .operational_history_store
                .load_session_events(&session_id, Some(&agent_id))
                .unwrap()
                .iter()
                .filter_map(|event| event.content.as_deref())
                .any(|text| text.contains("acknowledge Claude headless prompt"));
            if failed {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await;
    TEST_PROMPT_TIMER_MS.store(0, std::sync::atomic::Ordering::Relaxed);
    runtime
        .retire_owned_provider_run_after_terminal_failure(&session_id, run.id())
        .await;
    acknowledged_late
        .expect("once the card is answered the acknowledgement clock must end the start");
    card_task.abort();
    assert!(
        !failed_start,
        "the start watch must not fail a start waiting on its card"
    );
    assert!(
        still_active,
        "the turn must stay active while its card is open"
    );
    assert!(
        run_alive,
        "the provider run must stay alive while its card is open"
    );
}

#[tokio::test]
async fn a_screen_that_never_clears_fails_the_start_at_the_budget_naming_it() {
    crate::test_support::isolated_env_test!();
    TEST_PROMPT_TIMER_MS.store(5000, std::sync::atomic::Ordering::Relaxed);
    // The bypass-permissions confirmation: the kernel answers it, this Claude never moves on.
    let (_worktree, runtime, session_id, agent_id, source_id, run, dispatch) =
        runtime_with_claude_headless_cli(
            "#!/bin/sh\nprintf 'WARNING: Claude Code running in Bypass Permissions mode\\n 1. No, exit\\n 2. Yes, I accept\\n Enter to confirm\\n'\nwhile IFS= read -r line; do :; done\n",
            None,
        )
        .await;
    runtime
        .with_app_side_effect(|app| {
            crate::app::ProviderLaunchProcessRuntime::new(app).spawn_for_launch(&run)
        })
        .await
        .unwrap();
    runtime.spawn_prompt_dispatch(dispatch, runtime.provider_runtime_lanes.clone());
    let reason = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            runtime.pump_ready_provider_output_run(run.id()).await;
            let history = runtime
                .owned
                .operational_history_store
                .load_session_events(&session_id, Some(&agent_id))
                .unwrap();
            if let Some(text) = history
                .iter()
                .filter_map(|event| event.content.as_deref())
                .find(|text| text.contains("acknowledge Claude headless prompt"))
            {
                break text.to_string();
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await;
    TEST_PROMPT_TIMER_MS.store(0, std::sync::atomic::Ordering::Relaxed);
    if reason.is_err() {
        // Without a bound the start waits for ever; the person's cancel is what ends it.
        let cancellation = runtime
            .owned
            .cancel_local_prompt(&session_id, &agent_id, &source_id)
            .unwrap()
            .unwrap();
        runtime.spawn_prompt_abort(
            cancellation.dispatch.unwrap(),
            runtime.provider_runtime_lanes.clone(),
        );
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    runtime
        .retire_owned_provider_run_after_terminal_failure(&session_id, run.id())
        .await;
    let reason =
        reason.expect("a start held by a screen that never clears must fail at the budget");
    assert!(
        reason.contains("Bypass Permissions mode confirmation"),
        "the failure must name the screen: {reason}"
    );
    assert!(
        reason.contains("the agent's terminal"),
        "the failure must say what to do: {reason}"
    );
}

#[tokio::test]
async fn a_trust_marker_whose_card_is_gone_fails_the_start_at_the_budget() {
    crate::test_support::isolated_env_test!();
    TEST_PROMPT_TIMER_MS.store(5000, std::sync::atomic::Ordering::Relaxed);
    // A hot reload or restart loses the open card but not the marker it left in the run's files.
    let (_worktree, runtime, session_id, agent_id, _source_id, run, dispatch) =
        runtime_with_claude_headless_cli(
            "#!/bin/sh\nprintf 'Quick safety check\\nDo you trust this folder?\\n'\nwhile IFS= read -r line; do :; done\n",
            None,
        )
        .await;
    runtime
        .with_app_side_effect(|app| {
            crate::app::ProviderLaunchProcessRuntime::new(app).spawn_for_launch(&run)
        })
        .await
        .unwrap();
    std::fs::write(
        std::path::Path::new(&run.pty_env()["CHARIOX_CLAUDE_NATIVE_CONTEXT"])
            .with_file_name("active-prompt-id"),
        "startup-workspace-trust:claude-workspace-review-lost",
    )
    .unwrap();
    runtime.spawn_prompt_dispatch(dispatch, runtime.provider_runtime_lanes.clone());
    let failed = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let history = runtime
                .owned
                .operational_history_store
                .load_session_events(&session_id, Some(&agent_id))
                .unwrap();
            if let Some(text) = history
                .iter()
                .filter_map(|event| event.content.as_deref())
                .find(|text| text.contains("acknowledge Claude headless prompt"))
            {
                break text.to_string();
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await;
    TEST_PROMPT_TIMER_MS.store(0, std::sync::atomic::Ordering::Relaxed);
    runtime
        .retire_owned_provider_run_after_terminal_failure(&session_id, run.id())
        .await;
    let failed = failed.expect("a start waiting on a card that is gone must fail at the budget");
    assert!(
        failed.contains("did not finish starting"),
        "the failure must say the start ran out of time: {failed}"
    );
}
