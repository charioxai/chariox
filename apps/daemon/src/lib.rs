pub mod agent;
pub mod app;
pub mod attachment;
pub mod capability;
pub mod config;
pub(crate) mod env_lock;
pub mod error;
pub mod execution_lease;
pub mod history;
pub mod kernel_transport;
pub mod local;
pub mod logging;
pub mod prompt_transcript;
pub mod provider;
pub mod pty;
pub mod scheduler;
pub mod session;
pub mod session_history_page;
pub mod terminal;
pub mod transport;

pub use app::DaemonApp;
pub use config::DaemonConfig;
pub use error::DaemonError;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::thread;
    use std::time::Duration;

    use super::agent::CreateAgentRequest;
    use super::attachment::{AttachRequest, ClientCapabilityLevel};
    use super::provider::{LaunchProviderRequest, ProviderResumeState};
    use super::session::{CreateSessionRequest, PromptSubmissionOutcome};
    use super::terminal::TerminalOutputKind;
    use super::transport::relay_peer::{RelayProjectedCompletion, RelayProjectedOutputChunk};
    use super::{DaemonApp, DaemonConfig, DaemonError};

    #[test]
    fn daemon_app_bootstrap_wires_runtime_services() {
        let config = DaemonConfig::for_tests();
        let app = DaemonApp::bootstrap(config.clone()).expect("bootstrap should succeed");

        assert_eq!(app.config(), &config);
        assert_eq!(app.sessions().active_session_count(), 0);
        assert!(app.attachments().list_events().is_empty());
        assert!(app.providers().registry().registered_adapter_count() >= 1);
        assert!(app.terminal().input_records().is_empty());
        assert!(app.terminal().output_records().is_empty());
        assert!(app.terminal().notice_records().is_empty());
        assert_eq!(
            app.startup_message(),
            format!(
                "arroba daemon daemon-test ready on machine machine-test ({})",
                config.kernel_websocket_url()
            )
        );
    }

    #[test]
    fn daemon_config_rejects_empty_identifiers() {
        let error = match DaemonApp::bootstrap(DaemonConfig::new("", "machine-local", "miguel")) {
            Ok(_) => panic!("empty daemon id should be rejected"),
            Err(error) => error,
        };

        match error {
            DaemonError::InvalidConfig { field, .. } => assert_eq!(field, "daemon_id"),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn ending_session_via_app_removes_runtime_attachments() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");

        let attachment = app
            .attach(AttachRequest::new(
                session.id(),
                "client-a",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");

        let ended = app
            .end_session(session.id())
            .expect("session should end cleanly through the app");

        assert_eq!(ended.id(), session.id());
        assert!(app.attachments().get_attachment(attachment.id()).is_err());
    }

    #[test]
    fn shutdown_cleanup_ends_live_sessions_and_clears_managed_processes() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let attachment = app
            .attach(AttachRequest::new(
                session.id(),
                "client-a",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let run = app
            .launch_provider(LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            ))
            .expect("provider run should launch");

        assert_eq!(app.sessions().active_session_count(), 1);
        assert!(app
            .tracked_provider_processes
            .values()
            .any(|process| process.owner_provider_run_ids == vec![run.id().to_string()]));

        app.shutdown_cleanup()
            .expect("shutdown cleanup should end sessions");

        assert_eq!(app.sessions().active_session_count(), 0);
        assert!(app.tracked_provider_processes.is_empty());
        assert!(app.tracked_provider_run_processes.is_empty());
        assert!(app.attachments().get_attachment(attachment.id()).is_err());
    }

    #[test]
    fn execution_leases_require_opt_in_and_can_be_destroyed() {
        let mut disabled = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let error = disabled
            .create_execution_lease("home-kernel", "session-1", "agent-1")
            .expect_err("remote leases should require opt-in");
        match error {
            DaemonError::RemoteLeasesDisabled { .. } => {}
            other => panic!("unexpected error: {other}"),
        }

        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app =
            DaemonApp::bootstrap(config.clone()).expect("daemon bootstrap should succeed");
        let lease = app
            .create_execution_lease("home-kernel", "session-1", "agent-1")
            .expect("execution lease should be created");
        assert_eq!(lease.worker_kernel_id, config.daemon_id);
        assert_eq!(lease.machine_id, config.host_machine_id);
        assert_eq!(app.execution_lease_count(), 1);

        let removed = app
            .destroy_execution_lease(&lease.id)
            .expect("execution lease should be removed");
        assert_eq!(removed.id, lease.id);
        assert_eq!(app.execution_lease_count(), 0);
    }

    #[test]
    fn leased_agents_require_existing_lease_and_can_be_destroyed() {
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let lease = app
            .create_execution_lease("home-kernel", "session-1", "agent-home-1")
            .expect("execution lease should be created");
        let leased_agent = app
            .create_leased_agent(&lease.id, "opencode", Some("kimi2.5".to_string()), None)
            .expect("leased agent should be created");
        assert_eq!(leased_agent.lease_id, lease.id);
        assert_eq!(leased_agent.home_agent_id, "agent-home-1");
        assert_eq!(leased_agent.provider, "opencode");
        assert_eq!(app.leased_agent_count(), 1);

        let removed = app
            .destroy_leased_agent(&leased_agent.id)
            .expect("leased agent should be removed");
        assert_eq!(removed.id, leased_agent.id);
        assert_eq!(app.leased_agent_count(), 0);
    }
    #[test]
    fn leased_agents_can_submit_and_complete_prompts_through_backing_session() {
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app = DaemonApp::bootstrap(config).expect("daemon bootstrap should succeed");
        let lease = app
            .create_execution_lease("home-kernel", "session-1", "agent-home-1")
            .expect("execution lease should be created");
        let leased_agent = app
            .create_leased_agent(&lease.id, "dev-stub", Some("sonnet".to_string()), None)
            .expect("leased agent should be created");

        let hidden_backing_session = app
            .sessions()
            .get_session(&leased_agent.backing_session_id)
            .expect("backing session should exist");
        assert!(hidden_backing_session.is_hidden());
        assert!(app
            .sessions()
            .list_sessions()
            .into_iter()
            .all(|session| session.id() != leased_agent.backing_session_id));

        let (provider_run_id, outcome) = app
            .submit_leased_prompt(&leased_agent.id, "remote leased prompt\n", Vec::new())
            .expect("leased prompt should submit");
        match outcome {
            PromptSubmissionOutcome::Started { .. } => {}
            other => panic!("unexpected prompt submission outcome: {other:?}"),
        }

        let provider_run = app
            .providers()
            .get_run(&provider_run_id)
            .expect("provider run should exist");
        assert_eq!(provider_run.session_id(), leased_agent.backing_session_id);
        assert_eq!(
            provider_run.agent_instance_id(),
            Some(leased_agent.backing_agent_id.as_str())
        );

        let completion = app
            .complete_leased_prompt(&leased_agent.id)
            .expect("leased prompt should complete");
        assert_eq!(
            completion.completed.target_agent_id(),
            leased_agent.backing_agent_id
        );
    }

    #[test]
    fn remote_runtime_projection_records_output_and_completion_on_home_session() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let attachment = app
            .attach(AttachRequest::new(
                session.id(),
                "client-a",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("attachment should attach");

        app.project_remote_runtime_projection(
            session.id(),
            agent.id(),
            "remote:worker:provider-run-1",
            vec![RelayProjectedOutputChunk {
                kind: TerminalOutputKind::ProviderOutput,
                merge_key: Some("assistant-1".to_string()),
                bytes: b"remote output".to_vec(),
            }],
            vec!["remote notice".to_string()],
            vec![RelayProjectedCompletion {
                message_id: "assistant-msg-1".to_string(),
                completed_at_ms: 1234,
            }],
        )
        .expect("projection should succeed");

        let outputs = app
            .terminal_mut()
            .drain_output_records(session.id(), attachment.id());
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].agent_id.as_deref(), Some(agent.id()));
        assert_eq!(outputs[0].bytes, b"remote output".to_vec());

        let notices = app
            .terminal_mut()
            .drain_notice_records(session.id(), attachment.id());
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].agent_id.as_deref(), Some(agent.id()));
        assert_eq!(notices[0].message, "remote notice");

        let completions = app
            .terminal_mut()
            .drain_completion_records(session.id(), attachment.id());
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].agent_id.as_deref(), Some(agent.id()));
        assert_eq!(completions[0].message_id, "assistant-msg-1");
    }

    #[test]
    fn launching_provider_via_app_marks_session_active() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");

        let run = app
            .launch_provider(LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            ))
            .expect("provider run should launch");

        let session = app
            .sessions()
            .get_session(session.id())
            .expect("session should still exist");

        assert_eq!(session.active_provider_run_id(), Some(run.id()));
    }

    #[test]
    fn workflow_runs_flush_participating_agent_provider_runs_by_default() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");

        let initial_run = app
            .launch_provider(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id()),
            )
            .expect("provider run should launch");

        let workflow = app
            .sessions_mut()
            .create_workflow(session.id(), Some("review".to_string()))
            .expect("workflow should be created");
        let node = app
            .sessions_mut()
            .add_workflow_node(session.id(), workflow.id(), agent.id())
            .expect("workflow node should be added");
        let _endpoint = app
            .sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                workflow.id(),
                node.id(),
                Some("entry".to_string()),
            )
            .expect("workflow endpoint should be created");

        let workflow = app
            .sessions()
            .resolve_workflow_ref(session.id(), workflow.id())
            .expect("workflow should resolve with nodes");

        app.flush_workflow_agent_context_if_needed(session.id(), &workflow)
            .expect("workflow flush should succeed");

        let flushed_run = app
            .providers()
            .get_run(initial_run.id())
            .expect("initial provider run should still exist");
        assert_eq!(
            flushed_run.state(),
            super::provider::ProviderRunState::Ended
        );
    }

    #[test]
    fn detaching_last_attachment_parks_and_reattaching_resumes_same_provider_run() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");

        let attachment = app
            .attach(AttachRequest::new(
                session.id(),
                "client-a",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");

        let run = app
            .launch_provider(LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            ))
            .expect("provider run should launch");

        let _detached = app
            .detach(attachment.id())
            .expect("last attachment should detach cleanly");

        let parked_session = app
            .sessions()
            .get_session(session.id())
            .expect("session should still exist after detach");
        let parked_run = app
            .providers()
            .get_run(run.id())
            .expect("provider run should still exist after detach");

        assert_eq!(parked_session.active_provider_run_id(), None);
        assert_eq!(
            parked_run.state(),
            super::provider::ProviderRunState::Parked
        );

        let reattached = app
            .attach(AttachRequest::new(
                session.id(),
                "client-b",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("reattach should resume the parked provider run");

        let resumed_session = app
            .sessions()
            .get_session(session.id())
            .expect("session should still exist after reattach");
        let resumed_run = app
            .providers()
            .get_run(run.id())
            .expect("provider run should still exist after reattach");

        assert_eq!(reattached.session_id(), session.id());
        assert_eq!(resumed_session.active_provider_run_id(), Some(run.id()));
        assert_eq!(
            resumed_run.state(),
            super::provider::ProviderRunState::Running
        );
    }

    #[test]
    fn launching_a_provider_run_persists_resume_state_back_to_the_agent() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = app
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");

        let run = app
            .launch_provider(
                LaunchProviderRequest::new(
                    session.id(),
                    "dev-stub",
                    "claude-code",
                    "default",
                    "sonnet",
                )
                .with_agent_id(agent.id())
                .with_resume_state(ProviderResumeState::from_codex_thread_id("thread-1")),
            )
            .expect("provider run should launch");

        let stored_agent = app
            .agents()
            .get_agent(agent.id())
            .expect("agent should still exist");

        assert_eq!(run.resume_state().codex_thread_id(), Some("thread-1"));
        assert_eq!(
            stored_agent.provider_resume_state().codex_thread_id(),
            Some("thread-1")
        );
    }

    #[test]
    fn prompt_submission_queues_and_notifies_other_attachments() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");

        let first = app
            .attach(AttachRequest::new(
                session.id(),
                "client-a",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("first attachment should attach");
        let second = app
            .attach(AttachRequest::new(
                session.id(),
                "client-b",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("second attachment should attach");

        let _run = app
            .launch_provider(LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            ))
            .expect("provider run should launch");

        let first_outcome = crate::transport::TransportService::schedule_direct_prompt(
            &mut app,
            session.id(),
            first.id(),
            "first prompt\n",
            Vec::new(),
        )
        .expect("first prompt should start");
        let second_outcome = crate::transport::TransportService::schedule_direct_prompt(
            &mut app,
            session.id(),
            second.id(),
            "second prompt\n",
            Vec::new(),
        )
        .expect("second prompt should queue");

        match first_outcome {
            PromptSubmissionOutcome::Started { .. } => {}
            _ => panic!("expected first prompt to start"),
        }
        match second_outcome {
            PromptSubmissionOutcome::Queued { .. } => {}
            _ => panic!("expected second prompt to queue"),
        }

        assert_eq!(app.terminal().notice_records().len(), 1);
        assert!(app.terminal().notice_records()[0]
            .recipient_attachment_ids
            .contains(&first.id().to_string()));
    }

    #[test]
    fn spawning_a_seventh_agent_in_one_session_succeeds() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");

        for index in 0..6 {
            let agent = app
                .spawn_agent(
                    CreateAgentRequest::new(session.id(), "opencode")
                        .with_alias(format!("agent-{index}"))
                        .with_worktree("worktree-1"),
                )
                .expect("agent spawn should succeed");
            assert_eq!(agent.session_id(), session.id());
        }

        assert_eq!(app.list_session_agents(session.id()).len(), 7);
    }

    #[test]
    fn ended_sessions_reopen_on_attach_and_preserve_history() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, _agent) = app
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let attachment = app
            .attach(AttachRequest::new(
                session.id(),
                "client-a",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");

        let _run = app
            .launch_provider(LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            ))
            .expect("provider run should launch");

        let _ = crate::transport::TransportService::schedule_direct_prompt(
            &mut app,
            session.id(),
            attachment.id(),
            "restore me\n",
            Vec::new(),
        )
        .expect("prompt should submit");
        let _ = app.end_session(session.id()).expect("session should end");

        let reopened = app
            .attach(AttachRequest::new(
                session.id(),
                "client-b",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("ended session should reopen on attach");
        let reopened_session = app
            .sessions()
            .get_session(session.id())
            .expect("session should exist after reopen");
        let history = app
            .session_history(session.id())
            .expect("history should still load");

        assert_eq!(reopened.session_id(), session.id());
        assert_eq!(
            reopened_session.status(),
            super::session::SessionStatus::Parked
        );
        assert_eq!(reopened_session.attachment_ids().len(), 1);
        assert!(history
            .iter()
            .any(|entry| entry.text.contains("restore me")));
    }

    #[test]
    fn deleted_sessions_cannot_be_reattached() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = app
            .sessions_mut()
            .create_session(
                CreateSessionRequest::new("workspace-1", "worktree-1").with_alias("main"),
            )
            .expect("session should be created");

        let deleted = app
            .delete_session_ref("main", Some("workspace-1"))
            .expect("session should delete");

        assert_eq!(deleted.id(), session.id());
        assert!(matches!(
            app.attach(AttachRequest::new(
                session.id(),
                "client-a",
                ClientCapabilityLevel::FullTerminal,
            )),
            Err(crate::error::DaemonError::SessionNotFound { .. })
        ));
    }

    #[test]
    fn terminal_flow_writes_input_resizes_and_fans_out_output() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");

        let source = app
            .attach(AttachRequest::new(
                session.id(),
                "client-a",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("source attachment should attach");
        let observer = app
            .attach(AttachRequest::new(
                session.id(),
                "client-b",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("observer should attach");

        let run = app
            .launch_provider(LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            ))
            .expect("provider run should launch");

        app.resize_terminal(session.id(), 90, 24)
            .expect("terminal resize should succeed");
        app.send_terminal_input(session.id(), source.id(), b"fanout test\n")
            .expect("attachment input should reach provider PTY");

        let records = wait_for_terminal_output(&mut app, session.id(), source.id());

        assert!(!records.is_empty());
        assert_eq!(app.terminal().input_records().len(), 1);
        assert!(records
            .iter()
            .all(|record| record.provider_run_id == run.id()));
        assert!(records.iter().all(|record| {
            record
                .recipient_attachment_ids
                .contains(&source.id().to_string())
                && record
                    .recipient_attachment_ids
                    .contains(&observer.id().to_string())
        }));
        let combined = records
            .into_iter()
            .flat_map(|record| record.bytes)
            .collect::<Vec<u8>>();
        let combined = String::from_utf8_lossy(&combined);
        assert!(combined.contains("fanout test"));
    }

    #[test]
    fn config_updates_are_versioned_and_notified() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let first = app
            .attach(AttachRequest::new(
                session.id(),
                "client-a",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");
        let second = app
            .attach(AttachRequest::new(
                session.id(),
                "client-b",
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("attachment should attach");

        let config = app
            .update_session_config(
                session.id(),
                first.id(),
                BTreeMap::from([("theme".to_string(), "compact".to_string())]),
                false,
            )
            .expect("config update should succeed");

        assert_eq!(config.version(), 1);
        assert_eq!(
            config.values().get("theme").map(String::as_str),
            Some("compact")
        );
        assert_eq!(app.terminal().notice_records().len(), 1);
        assert!(app.terminal().notice_records()[0]
            .recipient_attachment_ids
            .contains(&second.id().to_string()));
    }

    #[test]
    fn failed_provider_switch_resumes_previous_run_and_records_notice() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new("workspace-1", "worktree-1"))
            .expect("session should be created");
        let _attachment = app
            .attach(AttachRequest::new(
                session.id(),
                "client-a",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");

        let first_run = app
            .launch_provider(LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "claude-code",
                "default",
                "sonnet",
            ))
            .expect("first provider run should launch");

        let error = app
            .launch_provider(LaunchProviderRequest::new(
                session.id(),
                "dev-invalid-pty",
                "claude-code",
                "default",
                "opus",
            ))
            .expect_err("invalid PTY adapter should fail during launch");

        match error {
            DaemonError::PtySpawn { .. } => {}
            other => panic!("unexpected error: {other}"),
        }

        let session = app
            .sessions()
            .get_session(session.id())
            .expect("session should still exist");
        let resumed_run = app
            .providers()
            .get_run(first_run.id())
            .expect("original run should still exist");

        assert_eq!(session.active_provider_run_id(), Some(first_run.id()));
        assert_eq!(
            resumed_run.state(),
            super::provider::ProviderRunState::Running
        );
        assert_eq!(app.terminal().notice_records().len(), 1);
        assert!(app.terminal().notice_records()[0]
            .message
            .contains("resumed the previous provider run"));
    }

    #[test]
    fn shell_command_capability_runs_through_daemon_app() {
        let worktree_root = std::env::temp_dir().join("arroba-shell-app-test");
        std::fs::create_dir_all(&worktree_root).expect("worktree dir should exist");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new(
                "workspace-1",
                worktree_root.display().to_string(),
            ))
            .expect("session should be created");
        let attachment = app
            .attach(AttachRequest::new(
                session.id(),
                "client-shell",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");

        let result = app
            .run_shell_command(crate::capability::RunShellCommandRequest::new(
                session.id(),
                attachment.id(),
                "/bin/sh",
                vec!["-lc".to_string(), "printf shell-app".to_string()],
                worktree_root,
                None,
            ))
            .expect("shell capability should succeed");

        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "shell-app");
    }

    #[test]
    fn directory_tree_file_and_git_capabilities_run_through_daemon_app() {
        let worktree_root = std::env::temp_dir().join("arroba-daemon-app-capability-test");
        let _ = std::fs::remove_dir_all(&worktree_root);
        std::fs::create_dir_all(worktree_root.join("src")).expect("worktree dir should exist");
        std::fs::write(worktree_root.join("README.md"), "hello").expect("file should exist");
        std::fs::write(worktree_root.join("src/lib.rs"), "before").expect("file should exist");
        std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&worktree_root)
            .output()
            .expect("git init should work");

        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new(
                "workspace-1",
                worktree_root.display().to_string(),
            ))
            .expect("session should be created");
        let attachment = app
            .attach(AttachRequest::new(
                session.id(),
                "client-capability",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");

        let tree = app
            .read_directory_tree(session.id(), attachment.id(), None, 2)
            .expect("tree read should succeed");
        let file = app
            .read_file(
                session.id(),
                attachment.id(),
                worktree_root.join("src/lib.rs"),
            )
            .expect("file read should succeed");
        let edit = app
            .edit_file(
                session.id(),
                attachment.id(),
                worktree_root.join("src/lib.rs"),
                "after".to_string(),
            )
            .expect("file edit should succeed");
        let git = app
            .inspect_git(session.id(), attachment.id(), None)
            .expect("git inspect should succeed");

        assert!(tree
            .entries
            .iter()
            .any(|entry| entry.relative_path == "README.md"));
        assert_eq!(file.contents, "before");
        assert_eq!(edit.bytes_written, 5);
        assert_eq!(edit.old_size, 6);
        assert_eq!(edit.new_size, 5);
        assert!(edit.changed);
        assert_eq!(
            std::fs::read_to_string(worktree_root.join("src/lib.rs")).expect("file readable"),
            "after"
        );
        assert!(git.status.contains("main"));
    }

    #[test]
    fn screenshot_capability_returns_structured_unavailable_result() {
        let _guard = crate::env_lock::lock();
        std::env::set_var("ARROBA_SCREENSHOT_DISABLE", "1");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new(
                "workspace-1",
                std::env::temp_dir().display().to_string(),
            ))
            .expect("session should be created");
        let attachment = app
            .attach(AttachRequest::new(
                session.id(),
                "client-screenshot",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");

        let result = app
            .capture_screenshot(session.id(), attachment.id())
            .expect("screenshot request should return structured result");
        std::env::remove_var("ARROBA_SCREENSHOT_DISABLE");

        assert_eq!(
            result.status,
            crate::capability::ScreenshotStatus::Unavailable
        );
        assert!(result.artifact_path.is_none());
    }

    #[test]
    fn transfer_capability_stores_artifact_under_session_root() {
        let worktree_root = std::env::temp_dir().join("arroba-transfer-app-test");
        let _ = std::fs::remove_dir_all(&worktree_root);
        std::fs::create_dir_all(&worktree_root).expect("worktree should exist");
        let source = worktree_root.join("artifact.txt");
        std::fs::write(&source, "artifact").expect("source should exist");
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let session = app
            .sessions_mut()
            .create_session(CreateSessionRequest::new(
                "workspace-1",
                worktree_root.display().to_string(),
            ))
            .expect("session should be created");
        let attachment = app
            .attach(AttachRequest::new(
                session.id(),
                "client-transfer",
                ClientCapabilityLevel::FullTerminal,
            ))
            .expect("attachment should attach");

        let result = app
            .store_transferred_file(session.id(), attachment.id(), source, None)
            .expect("transfer should succeed");

        assert!(result
            .stored_path
            .to_string_lossy()
            .contains("arroba-session-artifacts"));
        assert_eq!(result.bytes, 8);
    }

    fn wait_for_terminal_output(
        app: &mut DaemonApp,
        session_id: &str,
        attachment_id: &str,
    ) -> Vec<super::terminal::TerminalOutputRecord> {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);

        loop {
            let records = app
                .pump_terminal_output(session_id, attachment_id)
                .expect("terminal output should fan out");
            if !records.is_empty() {
                return records;
            }

            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for terminal output"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
