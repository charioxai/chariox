//! MP-08/MP-10/MP-11: an idle native run may still have a busy catalog lane.
use super::tests::{fixture_with_run_profile, popup, Fixture, PASSKEY};
use super::*;
use crate::provider::{
    AgentEndpointMode, ProviderClientInterface, ProviderLaunchResult, RuntimeProviderRun,
};

async fn native_fixture(room_tools: bool) -> Fixture {
    let mut f = fixture_with_run_profile(None, room_tools, "opencode", "sudo-native-fixture");
    let request = super::super::provider_reload::policy_reload_launch_request(
        &f.run,
        f.request.target_agent_id.as_deref().unwrap(),
        Default::default(),
    )
    .with_client_interface(ProviderClientInterface::NativeTui);
    let request = f
        .state
        .prepare_provider_launch_request_with_vault(request, "native catalog fixture")
        .await
        .unwrap();
    let mut run = RuntimeProviderRun::new(
        f.run.id(),
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "metadata-only".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: Default::default(),
            pty_env_remove: vec![],
            working_directory: f.run.working_directory().map(ToOwned::to_owned),
            structured_endpoint: None,
        },
    );
    run.mark_running();
    f.state
        .owned
        .provider_store
        .write()
        .insert_run_for_test(run.clone());
    f.state.owned.provider_run_projection.update(run.clone());
    f.run = run;
    f
}

async fn approve(f: &Fixture) -> tokio::task::JoinHandle<Result<LocalDaemonResponse, DaemonError>> {
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
    task
}

#[tokio::test]
async fn sudo_native_first_turn_waits_for_deferred_catalog_refresh() {
    for (busy, refreshing) in [(false, false), (true, false), (false, true)] {
        let f = native_fixture(true).await;
        let session = f
            .state
            .owned
            .session_store
            .get_session(&f.request.session_id)
            .unwrap();
        let agent = f.request.target_agent_id.as_deref().unwrap();
        let changes = f
            .state
            .owned
            .provider_run_projection
            .catalog_changes()
            .clone();
        let mut watch = changes.subscribe(f.run.id()).unwrap();
        let lane = (!refreshing).then(|| {
            f.state
                .owned
                .provider_store
                .run_operation_lanes()
                .try_acquire(f.run.id())
                .unwrap()
        });
        let refresh = refreshing.then(|| changes.begin_refresh(f.run.id()).unwrap());
        assert_eq!(
            f.state
                .refresh_native_runtime_catalog(f.run.clone())
                .await
                .unwrap(),
            super::super::provider_reload::ProviderReloadOutcome::Deferred
        );
        let initial = watch.current().desired;
        if busy {
            f.state
                .owned
                .submit_local_prepared_prompt_with_queue_policy(
                    &crate::app::KernelPreparedPromptSubmission {
                        session_id: session.id().into(),
                        prompt: PromptQueueItem::new(
                            "busy-before-native-refresh",
                            &f.request.attachment_id,
                            agent,
                            "ordinary",
                            PromptStatus::Queued,
                        ),
                        force_queue: false,
                        refresh_projection: true,
                    },
                    false,
                )
                .unwrap()
                .unwrap();
        }
        let task = approve(&f).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        if busy {
            assert!(f
                .state
                .owned
                .prompt_state_owner
                .sudo_work_held(&session, agent));
            f.state
                .owned
                .prompt_state_owner
                .cancel_active_prompt_only(&session, agent)
                .unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert!(!task.is_finished(), "idle does not imply catalog readiness");
        assert!(f.state.list_sudo_turns("local")[0].prompt_id.is_none());
        assert!(f
            .state
            .owned
            .prompt_state_owner
            .sudo_work_held(&session, agent));
        assert_eq!(watch.current().desired, initial);
        drop(lane);
        drop(refresh);
        // Supplementary fixture acknowledgement of the normal tools/list seam.
        // No official harness or user-facing acceptance is claimed here.
        tokio::time::timeout(Duration::from_secs(5), async {
            while watch.current().desired == initial {
                watch.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert!(!task.is_finished());
        changes.observed(f.run.id(), changes.revision(f.run.id()).unwrap());
        let response = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(
            response,
            LocalDaemonResponse::PromptSubmitted {
                outcome: PromptSubmissionOutcome::Started { .. },
                ..
            }
        ));
        let window = f.state.list_sudo_turns("local").pop().unwrap();
        assert_eq!(window.provider_run_id.as_deref(), Some(f.run.id()));
        f.state
            .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
            .unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn sudo_native_deferred_catalog_refresh_has_a_bounded_wait() {
    let f = native_fixture(true).await;
    let _lane = f
        .state
        .owned
        .provider_store
        .run_operation_lanes()
        .try_acquire(f.run.id())
        .unwrap();
    let task = approve(&f).await;
    let result = tokio::time::timeout(Duration::from_secs(65), task)
        .await
        .expect("catalog contention must not wait for the hour-scale sudo window")
        .unwrap();
    assert!(
        matches!(result, Err(DaemonError::LocalTransport {
        operation: "kernel access", ref message
    }) if message.contains("provider catalog refresh failed")),
        "{result:?}"
    );
    assert!(f.state.list_sudo_turns("local").is_empty());
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    assert!(!f
        .state
        .owned
        .prompt_state_owner
        .sudo_work_held(&session, f.request.target_agent_id.as_deref().unwrap()));
    assert!(f
        .state
        .owned
        .session_snapshot(session.id())
        .unwrap()
        .sudo_windows()
        .is_empty());
    assert!(f.state.owned.sudo_timers.lock().unwrap().is_empty());
    assert!(f.state.owned.sudo_scopes.lock().unwrap().is_empty());
}

// MP-08/MP-10/MP-11: ordinary work must not consume the refresh budget.
// Use the system clock through completion: native refresh owns a std deadline.
#[tokio::test]
async fn sudo_native_catalog_budget_restarts_after_ordinary_work() {
    let f = native_fixture(false).await;
    let session = f
        .state
        .owned
        .session_store
        .get_session(&f.request.session_id)
        .unwrap();
    let agent = f.request.target_agent_id.as_deref().unwrap();
    let changes = f
        .state
        .owned
        .provider_run_projection
        .catalog_changes()
        .clone();
    let mut watch = changes.subscribe(f.run.id()).unwrap();
    let initial = watch.current().desired;
    let lane = f
        .state
        .owned
        .provider_store
        .run_operation_lanes()
        .try_acquire(f.run.id())
        .unwrap();
    let task = approve(&f).await;
    // Let the idle refresh defer and start its budget before ordinary work.
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(!task.is_finished());
    assert!(!f
        .state
        .owned
        .prompt_state_owner
        .sudo_work_held(&session, agent));
    let submission = f
        .state
        .owned
        .submit_local_prepared_prompt_with_queue_policy(
            &crate::app::KernelPreparedPromptSubmission {
                session_id: session.id().into(),
                prompt: PromptQueueItem::new(
                    "busy-after-native-refresh-deferral",
                    &f.request.attachment_id,
                    agent,
                    "ordinary",
                    PromptStatus::Queued,
                ),
                force_queue: false,
                refresh_projection: true,
            },
            false,
        )
        .unwrap()
        .unwrap();
    assert!(matches!(
        submission.outcome,
        PromptSubmissionOutcome::Started { .. }
    ));
    tokio::time::sleep(Duration::from_secs(65)).await;
    assert!(
        !task.is_finished(),
        "ordinary work must not time out an unattempted catalog refresh"
    );
    assert_eq!(watch.current().desired, initial);
    assert!(f.state.list_sudo_turns("local")[0].prompt_id.is_none());
    f.state
        .owned
        .prompt_state_owner
        .complete_active_prompt_only(&session, agent)
        .unwrap();
    // More than the old budget's remaining 30 s: idle retries get a fresh 60 s.
    tokio::time::sleep(Duration::from_secs(40)).await;
    assert!(!task.is_finished());
    drop(lane);
    // Supplementary tools/list acknowledgement; no live acceptance claim.
    tokio::time::timeout(Duration::from_secs(5), async {
        while watch.current().desired == initial {
            watch.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(!task.is_finished());
    changes.observed(f.run.id(), changes.revision(f.run.id()).unwrap());
    let response = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(
        response,
        LocalDaemonResponse::PromptSubmitted {
            outcome: PromptSubmissionOutcome::Started { .. },
            ..
        }
    ));
    let window = f.state.list_sudo_turns("local").pop().unwrap();
    assert_eq!(window.provider_run_id.as_deref(), Some(f.run.id()));
    f.state
        .revoke_sudo(Some("local"), Some(&window.entry_id), "fixture_cleanup")
        .unwrap();
}
