//! MP-08/MP-10/MP-11: the real Reloaded branch must settle failed sudo starts.
use super::tests::{fixture_with_catalog_reload, popup, Fixture, PASSKEY};
use super::*;

pub(super) async fn queued_reload(
    f: &Fixture,
) -> (
    KernelSudoTurn,
    tokio::task::JoinHandle<Result<LocalDaemonResponse, DaemonError>>,
) {
    let agent = f.request.target_agent_id.as_deref().unwrap();
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    let busy = crate::app::KernelPreparedPromptSubmission {
        session_id: session.id().into(),
        prompt: PromptQueueItem::new(
            "busy-before-reload",
            &f.request.attachment_id,
            agent,
            "ordinary",
            PromptStatus::Queued,
        ),
        force_queue: false,
        refresh_projection: true,
    };
    f.state
        .owned
        .submit_local_prepared_prompt_with_queue_policy(&busy, false)
        .unwrap()
        .unwrap();
    let state = f.state.clone();
    let request = f.request.clone();
    let task = tokio::spawn(async move {
        state
            .submit_sudo_prompt(request, "local", "sudo-terminal")
            .await
    });
    let prompt = popup(&f.state).await;
    f.state
        .answer_terminal_runtime_interaction(
            &prompt.session_id,
            &prompt.interaction_id,
            "approve",
            None,
            Some("local"),
            Some(&ApprovalPasskey::new(PASSKEY)),
            None,
            Some(KernelConnectionClass::Terminal),
        )
        .await
        .unwrap();
    // Observe the hold installed while ordinary work is still busy.
    tokio::time::timeout(Duration::from_secs(5), async {
        while !f
            .state
            .owned
            .prompt_state_owner
            .sudo_work_held(&session, agent)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let window = f.state.list_sudo_turns("local").pop().unwrap();
    assert!(window.prompt_id.is_none());
    f.state
        .owned
        .prompt_state_owner
        .cancel_active_prompt_only(&session, agent)
        .unwrap();
    // Termination proves reload ran, rather than the ordinary dev-stub path.
    tokio::time::timeout(Duration::from_secs(5), async {
        while f
            .state
            .owned
            .provider_store
            .get_run_for_agent(session.id(), agent)
            .is_some()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    (window, task)
}

#[tokio::test(start_paused = true)]
async fn sudo_relaunch_waits_for_running_then_starts_the_authorized_turn() {
    let f = fixture_with_catalog_reload();
    let (window, task) = queued_reload(&f).await;
    assert!(!task.is_finished());
    // A replacement on the shared launch path supersedes the delayed task.
    let replacement = f
        .state
        .owned
        .start_provider_launch(
            crate::provider::LaunchProviderRequest::new(
                &window.session_id,
                "dev-stub",
                "dev-stub",
                "default",
                "default",
            )
            .with_agent_id(&window.agent_id),
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!task.is_finished());
    assert!(f.state.list_sudo_turns("local")[0].prompt_id.is_none());
    f.state.finish_provider_launch(&replacement, None).await;
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(
        result,
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { .. },
            ..
        }
    ));
    let live = f.state.list_sudo_turns("local").pop().unwrap();
    assert_eq!(live.provider_run_id.as_deref(), Some(replacement.run.id()));
    assert_eq!(live.prompt_id.as_deref(), Some(window.entry_id.as_str()));
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
        .unwrap();
    // Let the original delayed task observe the newer run and exit.
    tokio::time::sleep(Duration::from_secs(13)).await;
}

#[tokio::test(start_paused = true)]
async fn sudo_relaunch_bounds_starting_and_parked_replacements() {
    for parked in [false, true] {
        let f = fixture_with_catalog_reload();
        let (window, task) = queued_reload(&f).await;
        let mut replacement = f
            .state
            .owned
            .start_provider_launch(
                crate::provider::LaunchProviderRequest::new(
                    &window.session_id,
                    "dev-stub",
                    "dev-stub",
                    "default",
                    "default",
                )
                .with_agent_id(&window.agent_id),
            )
            .unwrap()
            .run;
        if parked {
            replacement.mark_parked();
            f.state
                .owned
                .provider_store
                .write()
                .insert_run_for_test(replacement);
        }
        let result = tokio::time::timeout(Duration::from_secs(65), task)
            .await
            .expect("Starting or Parked must not extend the relaunch deadline")
            .unwrap();
        assert!(
            matches!(result, Err(DaemonError::LocalTransport { operation: "kernel access", ref message })
            if message.contains("provider relaunch failed")),
            "{result:?}"
        );
        assert!(f.state.list_sudo_turns("local").is_empty());
        let session = f
            .state
            .owned
            .session_store
            .get_session(&window.session_id)
            .unwrap();
        assert!(!f
            .state
            .owned
            .prompt_state_owner
            .sudo_work_held(&session, &window.agent_id));
    }
}
