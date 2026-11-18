// A Claude headless start that stops at a screen nothing in Chariox answers fails the turn,
// naming that screen, instead of waiting for a cancel. The agent has no provider session yet,
// as on a fresh kernel.
use super::*;

#[tokio::test]
async fn a_claude_screen_nothing_answers_fails_the_start_naming_it() {
    crate::test_support::isolated_env_test!();
    TEST_PROMPT_ACK_IDLE_MS.store(1500, std::sync::atomic::Ordering::Relaxed);
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
    let reason = tokio::time::timeout(std::time::Duration::from_secs(20), async {
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
    TEST_PROMPT_ACK_IDLE_MS.store(0, std::sync::atomic::Ordering::Relaxed);
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
        reason.contains("Open the agent's terminal"),
        "the failure must say what to do: {reason}"
    );
}
