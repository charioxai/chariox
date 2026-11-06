use super::*;

// MP-08 / MP-09 / MP-10 / MP-11: each finished batch owns exactly one prompt
// identity. A proposal must not consume the registration of an ordinary poll.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_plan_batch_ordinary_first_app() {
    check_plan_batch_order(false, "ordinary-first").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_plan_batch_proposal_first_app() {
    check_plan_batch_order(false, "proposal-first").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_plan_batch_no_poll_app() {
    check_plan_batch_order(false, "no-poll").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_plan_batch_two_proposals_app() {
    check_plan_batch_order(false, "two-proposals").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_plan_batch_ordinary_first_owned() {
    check_plan_batch_order(true, "ordinary-first").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_plan_batch_proposal_first_owned() {
    check_plan_batch_order(true, "proposal-first").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_plan_batch_no_poll_owned() {
    check_plan_batch_order(true, "no-poll").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_plan_batch_two_proposals_owned() {
    check_plan_batch_order(true, "two-proposals").await;
}

async fn check_plan_batch_order(owned_pump: bool, ordering: &str) {
    use crate::provider::{AgentEndpointMode, AgentExecutionMode, ProviderPromptSignalBatch};
    let worktree = crate::test_support::TestWorktree::new("claude-plan-prompt-fence");
    let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    if ordering != "ordinary-first" {
        app.providers_mut().use_manual_output_polls_for_tests();
    }
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            session.id(),
            "plan-client",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let prompt = crate::session::PromptQueueItem::new(
        app.sessions_mut().reserve_prompt_id(),
        attachment.id(),
        agent.id(),
        "Make a plan",
        crate::session::PromptStatus::Queued,
    );
    let crate::session::PromptSubmissionOutcome::Started { prompt } = app
        .prompt_owner_submit_prepared_prompt(session.id(), prompt, false)
        .unwrap()
    else {
        panic!("prompt must start")
    };
    let request = crate::provider::LaunchProviderRequest::new(
        session.id(),
        "claude",
        "claude",
        "default",
        "sonnet",
    )
    .with_agent_id(agent.id())
    .with_execution_mode(AgentExecutionMode::Plan);
    let mut run = crate::provider::RuntimeProviderRun::new("claude-plan-fence", &request,
        crate::provider::ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "claude-plan-fence".into(), pty_target: None,
            pty_program: Some("/usr/bin/python3".into()),
            pty_args: vec!["-c".into(), r#"
import json, sys, time
sys.stdin.readline()
print(json.dumps({'type':'assistant','message':{'content':[{'type':'tool_use','id':'proposal','name':'ExitPlanMode','input':{'plan':'Implement after review.'}}]}}), flush=True)
while True: time.sleep(1)
"#.into()], pty_env: Default::default(), pty_env_remove: Vec::new(),
            working_directory: Some(worktree.path().to_path_buf()),
            structured_endpoint: Some("stdio://claude".into()),
        });
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    app.update_provider_run_projection(run.clone());
    app.mark_active_prompt_delivery(
        session.id(),
        agent.id(),
        prompt.id(),
        crate::session::DurablePromptDeliveryPhase::Delivered,
        Some(run.id().into()),
        None,
    )
    .unwrap();
    crate::transport::flow_control::note_prompt_started(&mut app, run.id());
    let mut binding = crate::provider::initialize_claude_runtime(&run).unwrap();
    crate::provider::submit_claude_prompt(
        &run,
        &mut binding.state,
        &crate::prompt_assembly::PromptEnvelope::new(
            "Make a plan",
            "",
            Vec::new(),
            crate::prompt_assembly::PromptManifest::current(),
        ),
    )
    .unwrap();
    app.providers()
        .apply_runtime_binding(
            run.id(),
            crate::provider::ProviderRuntimeBinding::Claude(binding),
        )
        .unwrap();
    let store = app.structured_output_record_store();
    // An older ordinary poll can still be pending after a prompt changes.
    let ordinary_prompt_id = if ordering == "proposal-first" {
        "previous-prompt"
    } else {
        prompt.id()
    };
    if ordering != "no-poll" {
        assert!(store
            .enqueue_poll(run.id(), Some(ordinary_prompt_id.into()), || {
                if ordering == "ordinary-first" {
                    app.providers_mut().enqueue_structured_output_poll(run.id())
                } else {
                    Ok(true)
                }
            })
            .unwrap());
    }
    app.providers()
        .publish_claude_plan(
            run.clone(),
            prompt.id().into(),
            Some("proposal".into()),
            "Implement after review.".into(),
        )
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    app.providers_mut().use_manual_output_polls_for_tests();
    if ordering == "two-proposals" {
        assert!(app
            .providers()
            .publish_claude_plan(
                run.clone(),
                prompt.id().into(),
                Some("proposal".into()),
                "Implement after review.".into()
            )
            .unwrap()
            .await
            .unwrap()
            .is_err());
    }
    if ordering == "two-proposals" {
        app.providers_mut()
            .push_finished_structured_output_poll_for_test(
                run.id().into(),
                Ok(Some(ProviderPromptSignalBatch::default())),
            );
    }
    let app = Arc::new(Mutex::new(app));
    let runtime = owned_runtime_state(&app).await;
    let mut records = if owned_pump {
        runtime
            .pump_owned_provider_output(session.id(), run.id(), vec![attachment.id().into()], true)
            .await
            .unwrap()
    } else {
        crate::app::provider_output::ProviderOutputPump::new(&mut *app.lock().await)
            .pump_provider_output(crate::app::provider_output::ProviderOutputPumpRequest {
                session_id: session.id(),
                provider_run_id: run.id(),
                recipient_attachment_ids: vec![attachment.id().into()],
                initial_liveness_already_checked: true,
            })
            .unwrap()
    };
    // Keep the ordinary completion pending until the proposal is consumed.
    // Its registration must still block another poll at this point.
    let preserved_poll = if ordering == "proposal-first" {
        let identity = store.take_in_flight_prompt_id(run.id());
        let preserved = identity.as_deref() == Some(ordinary_prompt_id);
        store.mark_poll_enqueued(run.id(), identity);
        preserved
    } else {
        true
    };
    if ordering == "proposal-first" {
        app.lock()
            .await
            .providers_mut()
            .push_finished_structured_output_poll_for_test(
                run.id().into(),
                Ok(Some(ProviderPromptSignalBatch::default())),
            );
        if owned_pump {
            records.extend(
                runtime
                    .pump_owned_provider_output(
                        session.id(),
                        run.id(),
                        vec![attachment.id().into()],
                        true,
                    )
                    .await
                    .unwrap(),
            );
        } else {
            records.extend(
                crate::app::provider_output::ProviderOutputPump::new(&mut *app.lock().await)
                    .pump_provider_output(crate::app::provider_output::ProviderOutputPumpRequest {
                        session_id: session.id(),
                        provider_run_id: run.id(),
                        recipient_attachment_ids: vec![attachment.id().into()],
                        initial_liveness_already_checked: true,
                    })
                    .unwrap(),
            );
        }
    }
    // Structured completion settles on the next pump after its normal quiet
    // window; exercising that pass also detects prompts stuck after plan loss.
    tokio::time::sleep(std::time::Duration::from_millis(75)).await;
    if owned_pump {
        records.extend(
            runtime
                .pump_owned_provider_output(
                    session.id(),
                    run.id(),
                    vec![attachment.id().into()],
                    true,
                )
                .await
                .unwrap(),
        );
    } else {
        records.extend(
            crate::app::provider_output::ProviderOutputPump::new(&mut *app.lock().await)
                .pump_provider_output(crate::app::provider_output::ProviderOutputPumpRequest {
                    session_id: session.id(),
                    provider_run_id: run.id(),
                    recipient_attachment_ids: vec![attachment.id().into()],
                    initial_liveness_already_checked: true,
                })
                .unwrap(),
        );
    }
    let app = app.lock().await;
    app.providers().clear_runtime(run.id());
    assert!(
        preserved_poll,
        "proposal consumed the pending ordinary poll registration"
    );
    let completed = |bytes: &[u8]| {
        let value: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_default();
        value["tool"] == "ExitPlanMode"
            && value["status"] == "completed"
            && value["input"]["plan"] == "Implement after review."
    };
    assert_eq!(
        records.iter().filter(|r| completed(&r.bytes)).count(),
        1,
        "one plan must reach the real pump: owned={owned_pump}, order={ordering}"
    );
    assert!(
        app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent.id())
            .unwrap()
            .is_none(),
        "the planning prompt must settle: {ordering}"
    );
    let history = app
        .load_session_history_entries(&session, Some(agent.id()))
        .unwrap();
    assert_eq!(
        history
            .iter()
            .filter(|e| completed(e.text.as_bytes()))
            .count(),
        1,
        "one plan must reach history: {ordering}"
    );
}
