use super::explicit_completion_tests::provider_run;
use super::*;
use crate::config::DaemonConfig;
use crate::provider::ProviderClientInterface;

#[test]
fn mp08_mp10_native_completion_retains_only_a_running_worker_binding() {
    for (interface, ended, retained, cancel_failed) in [
        (ProviderClientInterface::NativeTui, false, true, false),
        (ProviderClientInterface::NativeTui, true, false, false),
        (ProviderClientInterface::Chariox, false, false, false),
        (ProviderClientInterface::NativeTui, true, false, true),
    ] {
        let mut app = crate::app::DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new("home", "home"))
            .unwrap();
        app.agents
            .bind_remote_execution(
                agent.id(),
                crate::agent::RemoteAgentBinding {
                    worker_kernel_id: "worker-kernel-1".into(),
                    worker_machine_id: "worker-machine".into(),
                    execution_lease_id: "lease".into(),
                    leased_agent_id: "leased".into(),
                    active_worker_provider_run_id: Some("run".into()),
                    relay_url: None,
                    relay_token: None,
                    relay_peer_protocol_version: Some(
                        crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                    ),
                },
            )
            .unwrap();
        crate::app::KernelSessionService::new(&mut app)
            .attach(crate::attachment::AttachRequest::new(
                session.id(),
                "native-client",
                crate::attachment::ClientCapabilityLevel::FullTerminal,
            ))
            .unwrap();
        let mut run = provider_run(
            "run",
            "worker-session",
            "worker-agent",
            "claude",
            "claude",
            interface,
        );
        run.mark_running();
        let project =
            |app: &mut crate::app::DaemonApp, run_id: &str, snapshot, prompts, completions| {
                RemoteLeaseRuntime::new(app)
                    .project_remote_runtime_projection(
                        crate::runtime::relay_peer_authority::test_projection_authority(
                            "worker-kernel-1",
                        ),
                        RelayPeerEvent::LeasedRuntimeProjection {
                            home_session_id: session.id().into(),
                            home_agent_id: agent.id().into(),
                            provider_run_id: run_id.into(),
                            provider_run: snapshot,
                            prompts,
                            output_chunks: vec![],
                            notices: vec![],
                            completions,
                        },
                    )
                    .unwrap()
            };
        assert!(
            project(
                &mut app,
                "run",
                Some(run.clone()),
                vec![RelayProjectedPrompt {
                    prompt_id: "native-A".into(),
                    text: "research".into()
                }],
                vec![]
            )
            .accepted
        );
        if ended {
            run.mark_ended();
        }
        if cancel_failed {
            app.prompt_owner_begin_cancelling_active_prompt(session.id(), agent.id())
                .unwrap();
        }
        let completion = project(
            &mut app,
            "run",
            Some(run),
            vec![],
            vec![RelayProjectedCompletion {
                message_id: "complete-A".into(),
                completed_at_ms: crate::session::unix_epoch_ms(),
                home_prompt_id: Some("native-A".into()),
                provider_termination: cancel_failed.then(|| {
                    crate::provider::ProviderRunTermination::runtime_failure(
                        "Claude stopped responding to cancel; the run was ended.",
                        crate::session::unix_epoch_ms(),
                    )
                }),
            }],
        );
        assert_eq!(completion.completions.len(), 1);
        if cancel_failed {
            let settled = app
                .completed_git_turn_snapshot_store()
                .latest_projection_for_agent(session.id(), agent.id())
                .unwrap();
            assert_eq!(
                settled.settlement_status,
                crate::git_observer::CompletedTurnSettlementStatus::Failed
            );
            assert!(settled
                .provider_termination
                .unwrap()
                .reason
                .contains("stopped responding to cancel"));
        }
        let binding = app
            .agents
            .get_agent(agent.id())
            .unwrap()
            .remote_execution()
            .unwrap()
            .clone();
        assert_eq!(
            binding.active_worker_provider_run_id.as_deref(),
            retained.then_some("run"),
            "only the still-running native provider must retain its admitted binding"
        );
        if !retained {
            continue;
        }
        assert!(
            !project(
                &mut app,
                "stale-run",
                None,
                vec![RelayProjectedPrompt {
                    prompt_id: "foreign-B".into(),
                    text: "foreign".into()
                }],
                vec![]
            )
            .accepted
        );
        assert!(
            project(
                &mut app,
                "run",
                None,
                vec![RelayProjectedPrompt {
                    prompt_id: "native-B".into(),
                    text: "continuation".into()
                }],
                vec![]
            )
            .accepted,
            "unchanged native run sends no new provider snapshot"
        );
        let active = app
            .prompt_owner_active_prompt_for_agent(session.id(), agent.id())
            .unwrap()
            .unwrap();
        assert_eq!(active.id(), "native-B");
        assert_eq!(
            active.durable_delivery_phase(),
            Some(crate::session::DurablePromptDeliveryPhase::Delivered)
        );
        assert_eq!(active.durable_delivery_provider_run_id(), Some("run"));
        app.prompt_owner_begin_cancelling_active_prompt(session.id(), agent.id())
            .unwrap();
        let cancelled = project(
            &mut app,
            "run",
            None,
            vec![],
            vec![RelayProjectedCompletion {
                message_id: "cancel-B".into(),
                completed_at_ms: crate::session::unix_epoch_ms(),
                home_prompt_id: Some("native-B".into()),
                provider_termination: None,
            }],
        );
        assert_eq!(
            cancelled.completions[0].completed.status(),
            PromptStatus::Cancelled
        );
        assert_eq!(
            app.agents
                .get_agent(agent.id())
                .unwrap()
                .remote_execution()
                .unwrap()
                .active_worker_provider_run_id
                .as_deref(),
            Some("run")
        );
        // MP-11: an explicit profile/run retirement still closes admission.
        app.agents
            .set_remote_execution_active_worker_provider_run_id(agent.id(), None)
            .unwrap();
        assert!(!project(&mut app, "run", None, vec![], vec![]).accepted);
    }
}

#[test]
fn mp11_worker_completion_preserves_unrelated_local_turn_activity() {
    let mut app = crate::app::DaemonApp::bootstrap(DaemonConfig::for_tests()).unwrap();
    let (local, local_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new("local", "local"))
        .unwrap();
    let local_attachment = crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            local.id(),
            "local-tui",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let mut local_run = provider_run(
        "run",
        local.id(),
        local_agent.id(),
        "managed-dev-stub",
        "managed-dev-stub",
        ProviderClientInterface::Chariox,
    );
    local_run.mark_running();
    app.providers_mut().insert_run_for_test(local_run);
    app.prompt_owner_submit_prepared_prompt(
        local.id(),
        PromptQueueItem::new(
            "local-active",
            local_attachment.id(),
            local_agent.id(),
            "local research",
            PromptStatus::Queued,
        ),
        false,
    )
    .unwrap();
    crate::transport::flow_control::note_prompt_started(&mut app, "run");
    let local_active_turn = app
        .active_turns
        .get("run")
        .expect("real admitted local prompt must own its active turn");

    let (home, home_agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(crate::session::CreateSessionRequest::new(
            "remote", "remote",
        ))
        .unwrap();
    app.agents
        .bind_remote_execution(
            home_agent.id(),
            crate::agent::RemoteAgentBinding {
                worker_kernel_id: "worker-kernel-1".into(),
                worker_machine_id: "worker-machine".into(),
                execution_lease_id: "lease".into(),
                leased_agent_id: "leased".into(),
                active_worker_provider_run_id: Some("run".into()),
                relay_url: None,
                relay_token: None,
                relay_peer_protocol_version: Some(
                    crate::transport::relay_peer::RELAY_PEER_PROTOCOL_VERSION,
                ),
            },
        )
        .unwrap();
    crate::app::KernelSessionService::new(&mut app)
        .attach(crate::attachment::AttachRequest::new(
            home.id(),
            "native-tui",
            crate::attachment::ClientCapabilityLevel::FullTerminal,
        ))
        .unwrap();
    let mut run = provider_run(
        "run",
        "worker-session",
        "worker-agent",
        "claude",
        "claude",
        ProviderClientInterface::NativeTui,
    );
    run.mark_running();
    let project = |app: &mut crate::app::DaemonApp, snapshot, prompts, completions| {
        RemoteLeaseRuntime::new(app)
            .project_remote_runtime_projection(
                crate::runtime::relay_peer_authority::test_projection_authority("worker-kernel-1"),
                RelayPeerEvent::LeasedRuntimeProjection {
                    home_session_id: home.id().into(),
                    home_agent_id: home_agent.id().into(),
                    provider_run_id: "run".into(),
                    provider_run: snapshot,
                    prompts,
                    output_chunks: vec![],
                    notices: vec![],
                    completions,
                },
            )
            .unwrap()
    };
    assert!(
        project(
            &mut app,
            Some(run),
            vec![RelayProjectedPrompt {
                prompt_id: "native-A".into(),
                text: "remote research".into()
            }],
            vec![]
        )
        .accepted
    );
    let projected_id = crate::provider::projected_leased_provider_run_id("leased", "run");
    assert!(app.prompt_activity.read().contains_key(&projected_id));
    let completion = project(
        &mut app,
        None,
        vec![],
        vec![RelayProjectedCompletion {
            message_id: "complete-A".into(),
            completed_at_ms: crate::session::unix_epoch_ms(),
            home_prompt_id: Some("native-A".into()),
            provider_termination: None,
        }],
    );
    assert_eq!(completion.completions.len(), 1);
    assert_eq!(
        app.active_turns.get("run"),
        Some(local_active_turn),
        "a worker's raw run ID cannot retire a local turn"
    );
    assert!(app.prompt_activity.read().contains_key("run"));
    assert!(
        !app.prompt_activity.read().contains_key(&projected_id),
        "the completed native projection must clear its own home activity"
    );
}
