//! MP-08 / MP-10 / MP-11: real PTY delivery, lost settlement and actual pump restart.
use super::*;
use crate::provider::{
    AgentEndpointMode, LaunchProviderRequest, ProviderClientInterface, ProviderLaunchResult,
    RuntimeProviderRun,
};

async fn pump(runtime: &KernelRuntimeState) {
    // Wait out the shared one-second floor, then use the production coordinator.
    if let Some(due) = runtime.app_control().event_pump().next_due() {
        tokio::time::sleep(due.saturating_duration_since(std::time::Instant::now())).await;
    }
    runtime.schedule_app_event_pump();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if runtime
                .app_control()
                .event_pump()
                .next_due()
                .is_none_or(|due| due > std::time::Instant::now())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("scheduled notification pass should finish");
}

// Poll before removal so the product cleanup path never needs to send a signal.
async fn close_pty(runtime: &KernelRuntimeState, run_id: &str, input: Option<&[u8]>) {
    if let Some(input) = input {
        runtime
            .with_app_side_effect(|app| {
                app.write_provider_pty_input_for_runtime(run_id, input)
                    .unwrap()
            })
            .await;
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if runtime
                .with_app_side_effect(|app| {
                    crate::app::ProviderLaunchProcessRuntime::new(app)
                        .poll_exit(run_id)
                        .unwrap()
                        .is_some()
                })
                .await
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owned fixture should exit normally");
    runtime
        .with_app_side_effect(|app| {
            crate::app::ProviderProcessTracker::new(app)
                .remove_run(run_id)
                .unwrap();
        })
        .await;
}

async fn delivered_without_settlement(adapter: &str, provider: &str) {
    for app_source in [false, true] {
        let (root, runtime, session, id) = fixture(app_source, 1).await;
        let injection = runtime
            .owned
            .prepare_notification_injection(&session, &id)
            .unwrap()
            .unwrap();
        // Replace the initial fixture's cat process, rather than reusing its PTY alias.
        close_pty(&runtime, &injection.dispatch.provider_run_id, Some(b"\x04")).await;
        let capture = root.path().join("received-input.txt");
        let script = root.path().join("notification-pty.py");
        // This credential-free harness captures bytes consumed on a real PTY.
        // It has a bounded lifetime even on test failure and never signals a process.
        std::fs::write(
            &script,
            r#"import os, select, sys, time
deadline = time.monotonic() + 20
received = b''
while time.monotonic() < deadline:
    if not select.select([sys.stdin], [], [], max(0, deadline-time.monotonic()))[0]:
        break
    chunk = os.read(0, 65536)
    if not chunk or b'WF_FIXTURE_EXIT' in chunk:
        break
    received += chunk
    with open(sys.argv[1], 'wb') as capture:
        capture.write(received)
    if sys.argv[2] == 'claude-headless' and b'NOTIFICATION_INJECT_PROOF' in received:
        break
"#,
        )
        .unwrap();
        let request = LaunchProviderRequest::new(&session, adapter, provider, "default", "default")
            .with_agent_id(&injection.dispatch.agent_id)
            .with_client_interface(if adapter == "claude" {
                ProviderClientInterface::NativeTui
            } else {
                ProviderClientInterface::Chariox
            });
        let mut env = std::collections::BTreeMap::new();
        if adapter == "claude" {
            env.insert(
                "CHARIOX_CLAUDE_NATIVE_CONTEXT".into(),
                root.path().join("hidden-context.txt").display().to_string(),
            );
        }
        let mut run = RuntimeProviderRun::new(
            &injection.dispatch.provider_run_id,
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "notification-pty-fixture".into(),
                pty_target: Some(injection.dispatch.provider_run_id.clone()),
                pty_program: Some("python3".into()),
                pty_args: vec![
                    script.display().to_string(),
                    capture.display().to_string(),
                    provider.into(),
                ],
                pty_env: env,
                pty_env_remove: vec![],
                working_directory: Some(root.path().to_path_buf()),
                structured_endpoint: None,
            },
        );
        run.mark_running();
        assert!(!runtime
            .owned
            .provider_store
            .run_uses_structured_prompt_io(&run));
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(run.clone());
        runtime
            .with_app_side_effect(|app| {
                crate::app::ProviderLaunchProcessRuntime::new(app)
                    .spawn_for_launch(&run)
                    .unwrap();
            })
            .await;
        let db = rusqlite::Connection::open(runtime.owned.durable_state_store.path()).unwrap();
        // Native/ordinary PTY accepts the write, but its durable settlement is lost.
        // Headless instead loses its hook ACK by exiting after consuming the bytes.
        db.execute_batch(
            r#"CREATE TRIGGER lose_notification_settlement
            BEFORE INSERT ON durable_state_events
            WHEN json_extract(NEW.payload_json,'$.reason')='notification_injected'
            BEGIN SELECT RAISE(FAIL,'fixture loses notification settlement'); END;"#,
        )
        .unwrap();
        pump(&runtime).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            while !std::fs::read(&capture)
                .unwrap_or_default()
                .windows(b"NOTIFICATION_INJECT_PROOF".len())
                .any(|w| w == b"NOTIFICATION_INJECT_PROOF")
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("fixture must actually consume the notification");
        db.execute_batch("DROP TRIGGER lose_notification_settlement")
            .unwrap();
        drop(db);
        let sent = runtime.owned.session_store.get_session(&session).unwrap();
        // A still-active plain PTY must not receive a second write either.
        if adapter != "claude" {
            pump(&runtime).await;
        }
        // Let the owned fixture exit normally; remove only its exact registered run.
        close_pty(
            &runtime,
            run.id(),
            if provider == "claude-headless" {
                None
            } else {
                Some(b"WF_FIXTURE_EXIT\n")
            },
        )
        .await;
        assert!(!runtime
            .owned
            .provider_process_tracking
            .read()
            .run_processes
            .contains_key(run.id()));
        end_original_notification_turn(&runtime, &session, &injection);
        let config = runtime.owned.config_projection.snapshot();
        runtime
            .with_app_side_effect(|app| app.save_durable_state_snapshot().unwrap())
            .await;
        drop(runtime);
        let restored =
            super::super::super::workflow_prompt_queue_owned_state::tests::runtime_state_from_app(
                crate::test_support::bootstrap_after_test_owner_exit(config).await,
            );
        pump(&restored).await;
        pump(&restored).await;
        let snapshot = restored.owned.session_store.get_session(&session).unwrap();
        let item = snapshot
            .workflow_queued_prompts()
            .iter()
            .find(|q| q.id() == id)
            .expect("MP-08 / MP-10: unknown native/PTY send must retain its original pending item");
        assert_eq!(
            item.status(),
            WorkflowQueuedPromptStatus::Running,
            "MP-08 / MP-10: delivered input with unknown settlement cannot create a new invocation"
        );
        assert!(item.notification_injection_pending());
        assert!(snapshot
            .workflow_runs()
            .iter()
            .all(|r| r.id() == "active-0"));
        let saved = &item.publication_invocation().unwrap().caller["notification_steer"];
        assert_eq!(
            saved,
            &sent
                .workflow_queued_prompts()
                .iter()
                .find(|q| q.id() == id)
                .unwrap()
                .publication_invocation()
                .unwrap()
                .caller["notification_steer"]
        );
        assert!(
            saved.get("submit_epoch").is_some(),
            "every local path persists the same send identity"
        );
        assert_eq!(saved["provider_run_id"], injection.dispatch.provider_run_id);
        assert_eq!(
            saved["prompt_id"],
            injection.dispatch.target_active_prompt_id.unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(&capture)
                .unwrap()
                .matches("NOTIFICATION_INJECT_PROOF")
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn notification_inject_native_pty_lost_settlement_survives_pump_restart() {
    delivered_without_settlement("claude", "claude").await;
}

#[tokio::test]
async fn notification_inject_plain_pty_lost_settlement_survives_pump_restart() {
    delivered_without_settlement("dev-stub", "claude-code").await;
}

#[tokio::test]
async fn notification_inject_headless_pty_lost_ack_survives_pump_restart() {
    delivered_without_settlement("claude", "claude-headless").await;
}
