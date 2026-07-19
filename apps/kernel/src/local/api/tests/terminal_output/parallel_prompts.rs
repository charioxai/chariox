use super::*;

#[test]
fn terminal_output_drain_streams_parallel_agent_prompts_for_same_attachment() {
    let harness = LocalRouterTestHarness::new();
    let (session, default_agent) = match harness
        .dispatch(LocalDaemonRequest::CreateSession(
            CreateSessionRequest::new("workspace-1", "worktree-1"),
        ))
        .expect("session should be created")
    {
        LocalDaemonResponse::SessionCreated { session, agent } => (session, agent),
        _ => panic!("unexpected local response"),
    };
    let attachment = match harness
        .dispatch(LocalDaemonRequest::AttachToSession(
            AttachToSessionRequest {
                session_id: session.id().to_string(),
                client_id: "client-1".to_string(),
                capability_level: ClientCapabilityLevel::FullTerminal,
            },
        ))
        .expect("attachment should attach")
    {
        LocalDaemonResponse::SessionAttached { attachment } => attachment,
        _ => panic!("unexpected local response"),
    };
    let spawned = match harness
        .dispatch(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
            session_id: session.id().to_string(),
            alias: Some("parallel".to_string()),
            provider: Some("dev-stub".to_string()),
            model: Some("claude-code".to_string()),
            effort: Some("default".to_string()),
            execution_mode: None,
            permission_level: None,
            worktree_id: None,
            kernel_ref: None,
            slice_ref: None,
            worktree_placement: None,
            metaagent: false,
        }))
        .expect("spawn should succeed")
    {
        LocalDaemonResponse::AgentSpawned { agent } => agent,
        _ => panic!("unexpected local response"),
    };

    let (default_run_id, spawned_run_id) = harness.with_app_mut(|app| {
        (
            launch_slow_structured_run(app, session.id(), default_agent.id()),
            launch_slow_structured_run(app, session.id(), spawned.id()),
        )
    });

    let mut submitted_prompt_ids = std::collections::BTreeMap::new();
    for agent_id in [default_agent.id(), spawned.id()] {
        match harness
            .dispatch(LocalDaemonRequest::SubmitPrompt(SubmitPromptRequest {
                session_id: session.id().to_string(),
                attachment_id: attachment.id().to_string(),
                target_agent_id: Some(agent_id.to_string()),
                prompt: format!("parallel prompt for {agent_id}\n"),
                attachments: Vec::new(),
            }))
            .expect("prompt should start")
        {
            LocalDaemonResponse::PromptSubmitted {
                outcome: PromptSubmissionOutcome::Started { prompt },
                ..
            } => {
                submitted_prompt_ids.insert(agent_id.to_string(), prompt.id().to_string());
            }
            _ => panic!("unexpected local response"),
        }
    }

    let delivery_deadline = Instant::now() + Duration::from_secs(2);
    let mut prompts_delivered = false;
    while Instant::now() < delivery_deadline && !prompts_delivered {
        prompts_delivered = harness.with_app_mut(|app| {
            crate::app::provider_output::reap_structured_prompt_jobs(app);
            submitted_prompt_ids.iter().all(|(agent_id, prompt_id)| {
                app.prompt_owner_active_prompt_for_agent_snapshot(session.id(), agent_id)
                    .expect("active prompt snapshot should remain available")
                    .is_some_and(|prompt| {
                        prompt.id() == prompt_id
                            && prompt.durable_delivery_phase()
                                == Some(crate::session::DurablePromptDeliveryPhase::Delivered)
                    })
            })
        });
        if !prompts_delivered {
            thread::sleep(Duration::from_millis(10));
        }
    }
    assert!(
        prompts_delivered,
        "both provider submissions should be acknowledged before output is polled"
    );

    harness.with_app_mut(|app| {
        let structured_output_records = app.structured_output_record_store();
        for (provider_run_id, agent_id) in [
            (default_run_id.clone(), default_agent.id().to_string()),
            (spawned_run_id.clone(), spawned.id().to_string()),
        ] {
            structured_output_records.mark_poll_enqueued(
                &provider_run_id,
                Some(
                    submitted_prompt_ids
                        .get(&agent_id)
                        .expect("submitted prompt should be tracked")
                        .clone(),
                ),
            );
            app.providers_mut()
                .push_finished_structured_output_poll_for_test(
                    provider_run_id,
                    Ok(Some(ProviderPromptSignalBatch {
                        chunks: vec![ProviderPromptChunk {
                            kind: TerminalOutputKind::ProviderOutput,
                            merge_key: Some(format!("parallel-{agent_id}")),
                            bytes: format!("parallel output for {agent_id}\n").into_bytes(),
                        }],
                        ..ProviderPromptSignalBatch::default()
                    })),
                );
        }
    });

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut seen_agents = std::collections::BTreeSet::new();
    while Instant::now() < deadline && seen_agents.len() < 2 {
        let records = harness.with_app_mut(|app| {
            crate::app::provider_output::pump_terminal_output_for_attachment(
                app,
                session.id(),
                attachment.id(),
            )
            .expect("terminal output should keep pumping")
        });
        for record in records {
            if let Some(agent_id) = record.agent_id {
                seen_agents.insert(agent_id);
            }
        }
        thread::sleep(Duration::from_millis(10));
    }

    assert!(
        seen_agents.contains(default_agent.id()) && seen_agents.contains(spawned.id()),
        "expected output from both active agent prompts, saw {:?}",
        seen_agents
    );
}
