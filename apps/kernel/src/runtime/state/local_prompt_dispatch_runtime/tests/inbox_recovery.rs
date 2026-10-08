//! MP-08/MP-10/MP-11: send-boundary correlation survives an authorized replacement.
use super::*;
use crate::durable_state::agent_lifecycle::{occurrence, Operation, Outcome};

#[tokio::test]
async fn a02_r8_structured_receipt_follows_actual_replacement_run() {
    for app_reaper in [false, true] {
        let (_worktree, runtime, room, agent, _, original, mut dispatch) =
            runtime_with_admitted_prompt().await;
        let store = &runtime.owned.durable_state_store;
        let Outcome::Event(event) = store
            .agent_lifecycle(Operation::Occur(occurrence(
                &room,
                &agent,
                "peer",
                "replacement-message",
                "message",
                serde_json::json!({"message":"Review"}),
            )))
            .unwrap()
        else {
            panic!()
        };
        store
            .agent_lifecycle(Operation::Attempt {
                room: room.clone(),
                agent: agent.clone(),
                sequence: event.sequence,
                prompt: dispatch.prompt_id.clone(),
                target: None,
                run: Some(original.clone()),
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap();
        store
            .agent_lifecycle(Operation::BindSubmission {
                room: room.clone(),
                agent: agent.clone(),
                sequence: event.sequence,
                prompt: dispatch.prompt_id.clone(),
                target: None,
                now: crate::session::unix_epoch_ms(),
                run: original.clone(),
                submit_epoch: runtime.owned.provider_store.structured_submit_epoch(),
            })
            .unwrap();
        let request =
            LaunchProviderRequest::new(&room, "dev-stub", "slow-structured", "default", "model")
                .with_agent_id(&agent);
        let mut replacement = crate::provider::RuntimeProviderRun::new(
            "authorized-replacement",
            &request,
            crate::provider::ProviderLaunchResult {
                endpoint_mode: crate::provider::AgentEndpointMode::Managed,
                process_label: "am2-r8-submit".into(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: std::collections::BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: Some("test".into()),
            },
        );
        replacement.mark_running();
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(replacement.clone());
        dispatch.provider_run_id = replacement.id().into();
        assert!(runtime
            .enqueue_prompt_dispatch_after_liveness_with_acceptance(&dispatch, &runtime.owned)
            .await
            .unwrap());
        let finished = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(job) = runtime
                    .owned
                    .provider_store
                    .drain_finished_structured_prompt_submit_jobs()
                    .into_iter()
                    .next()
                {
                    break job;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(finished.provider_run_id, replacement.id());
        assert!(finished.result.is_ok());
        reap_finished_submit(&runtime, app_reaper, finished).await;
        let delivered = store
            .agent_event_for_prompt(&room, &agent, &dispatch.prompt_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            delivered.state, "accepted",
            "MP-11: replacement acceptance must reconcile the inbox"
        );
        assert_eq!(delivered.provider_run_id.as_deref(), Some(replacement.id()));
        assert_eq!(
            runtime
                .owned
                .provider_store
                .drain_finished_structured_prompt_submit_jobs()
                .len(),
            0,
            "MP-11: mismatched receipts must not poison the reaper queue"
        );
        let stale = crate::provider::FinishedProviderPromptSubmitJob {
            session_id: room.clone(),
            agent_id: agent.clone(),
            prompt_id: dispatch.prompt_id.clone(),
            provider_run_id: original,
            result: Err(crate::durable_state::agent_lifecycle::error(
                "old run failed",
            )),
            settlement_retry_attempt: 0,
        };
        reap_finished_submit(&runtime, app_reaper, stale).await;
        assert_eq!(
            store
                .agent_event_for_prompt(&room, &agent, &dispatch.prompt_id)
                .unwrap()
                .unwrap(),
            delivered
        );
        assert!(runtime
            .owned
            .provider_store
            .drain_finished_structured_prompt_submit_jobs()
            .is_empty());
    }
}
