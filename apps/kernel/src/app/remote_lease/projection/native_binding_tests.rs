use super::explicit_completion_tests::provider_run;
use super::*;
use crate::config::DaemonConfig;
use crate::provider::ProviderClientInterface;

#[test]
fn mp08_mp10_native_completion_retains_only_a_running_worker_binding() {
    for (interface, ended, retained) in [
        (ProviderClientInterface::NativeTui, false, true),
        (ProviderClientInterface::NativeTui, true, false),
        (ProviderClientInterface::Chariox, false, false),
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
        let completion = project(
            &mut app,
            "run",
            Some(run),
            vec![],
            vec![RelayProjectedCompletion {
                message_id: "complete-A".into(),
                completed_at_ms: crate::session::unix_epoch_ms(),
                home_prompt_id: Some("native-A".into()),
                provider_termination: None,
            }],
        );
        assert_eq!(completion.completions.len(), 1);
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
