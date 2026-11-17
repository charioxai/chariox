use super::*;

impl KernelRuntimeState {
    pub(crate) async fn agent_utility_provider_run(
        &self,
        session_id: &str,
        agent_id: &str,
        operation: &'static str,
    ) -> Result<
        (
            crate::agent::AgentInstance,
            crate::provider::RuntimeProviderRun,
        ),
        DaemonError,
    > {
        let session = self.owned.session_store.get_session(session_id)?;
        let agent = self
            .owned
            .agent_store
            .get_session_agents(session_id)
            .iter()
            .find(|agent| agent.id() == agent_id)
            .cloned()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation,
                message: format!("agent `{agent_id}` does not belong to session `{session_id}`"),
            })?;
        if agent.remote_execution().is_some() {
            return Err(DaemonError::LocalTransport {
                operation,
                message: format!(
                    "agent `{agent_id}` is remote-backed; hidden utilities must run on its worker kernel"
                ),
            });
        }
        if self
            .owned
            .prompt_state_owner
            .active_prompt_for_agent(&session, agent_id)
            .is_some()
        {
            return Err(DaemonError::LocalTransport {
                operation,
                message: format!("agent `{agent_id}` is busy"),
            });
        }
        let provider_run = if let Some(provider_run) = self
            .owned
            .provider_store
            .get_run_for_agent(session_id, agent_id)
        {
            provider_run
        } else {
            let session_id = session_id.to_string();
            let agent_id = agent_id.to_string();
            let provider_run_id = self
                .with_app_side_effect(move |app| {
                    app.ensure_prompt_provider_run_for_agent(&session_id, &agent_id)
                })
                .await?;
            self.owned.provider_store.get_run(&provider_run_id)?
        };
        Ok((agent, provider_run))
    }

    /// The bridge that shows a provider's permission questions to every client of the Room.
    pub(crate) fn provider_native_interaction_bridge(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::provider::ProviderNativeInteractionBridge>> {
        self.owned.provider_store.native_interaction_bridge()
    }

    /// A setup turn on a Claude run with no structured I/O (headless, native terminal), with the
    /// run's own account credentials.
    pub(crate) async fn run_room_commands_on_native_claude(
        &self,
        provider_run: crate::provider::RuntimeProviderRun,
        visible_user_prompt: String,
        hidden_system_context: String,
        timeout: tokio::time::Duration,
    ) -> Result<String, DaemonError> {
        const OPERATION: &str = "run room commands on native Claude";
        let credentials = self
            .resolve_provider_account_credentials_for_run_with_vault(&provider_run, OPERATION)
            .await?;
        tokio::task::spawn_blocking(move || {
            crate::provider::run_room_commands_on_fresh_claude(
                &provider_run,
                &credentials,
                &visible_user_prompt,
                &hidden_system_context,
                timeout,
            )
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: OPERATION,
            message: format!("provider utility prompt task failed: {error}"),
        })?
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_structured_provider_utility_prompt(
        &self,
        provider_run: crate::provider::RuntimeProviderRun,
        visible_user_prompt: String,
        hidden_system_context: String,
        timeout: tokio::time::Duration,
        policy: crate::provider::ProviderUtilityExecutionPolicy,
        admission: Option<crate::provider::ProviderUtilityAdmission>,
        stop: Option<crate::provider::UtilityStop>,
    ) -> Result<String, DaemonError> {
        let provider_store = self.owned.provider_store.clone();
        tokio::task::spawn_blocking(move || {
            provider_store.run_structured_utility_prompt_with_admission(
                &provider_run,
                &visible_user_prompt,
                &hidden_system_context,
                timeout,
                policy,
                admission,
                stop,
            )
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "run structured provider utility prompt",
            message: format!("provider utility prompt task failed: {error}"),
        })?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    async fn owned_runtime_state(app: &Arc<Mutex<DaemonApp>>) -> KernelRuntimeState {
        let (
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        ) = {
            let app_locked = app.lock().await;
            (
                app_locked.config_projection_store(),
                app_locked.session_state_store(),
                app_locked.agents().clone(),
                app_locked.attachments().clone(),
                app_locked.providers().clone(),
                app_locked.provider_process_tracking_store(),
                app_locked.slices(),
                app_locked.session_state_projection_store(),
                app_locked.provider_run_projection_store(),
                app_locked.operational_history_store(),
                app_locked.durable_state_store(),
                app_locked.prompt_state_owner(),
                app_locked.active_turn_store(),
                app_locked.prompt_activity_store(),
                app_locked.prompt_workspace_claim_store(),
                app_locked.structured_output_record_store(),
                app_locked.terminal_stream_store(),
                app_locked.workflow_design_event_store(),
                app_locked.metaagent_event_store(),
                app_locked.workspace_coordinator(),
            )
        };
        KernelRuntimeState::new_with_owned_state(
            Arc::clone(app),
            config_projection,
            session_store,
            agent_store,
            attachment_store,
            provider_store,
            provider_process_tracking,
            slice_store,
            session_projection,
            provider_run_projection,
            operational_history_store,
            durable_state_store,
            prompt_state_owner,
            active_turns,
            prompt_activity,
            prompt_workspace_claims,
            structured_output_records,
            terminal_stream,
            workflow_design_events,
            metaagent_events,
            workspace_coordinator,
        )
    }

    #[tokio::test]
    async fn utility_provider_run_rejects_prompt_owner_active_prompt_when_mirror_is_stale() {
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests())
            .expect("daemon bootstrap should succeed");
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-utility-owner-busy",
                "worktree-utility-owner-busy",
            ))
            .expect("session should be created");
        let external_prompt = crate::session::PromptQueueItem::external_observed_running(
            "codex",
            "codex-thread-utility-owner-busy",
            "codex-turn-utility-owner-busy",
            agent.id(),
            "external prompt still running",
        );
        app.prompt_owner_sync_external_active_prompt(
            session.id(),
            agent.id(),
            Some(external_prompt),
        )
        .expect("external active prompt should sync");
        app.sessions_mut()
            .mirror_agent_prompt_state(
                session.id(),
                agent.id(),
                None,
                std::collections::VecDeque::new(),
            )
            .expect("test drift should clear stale session prompt mirror");
        assert!(
            app.sessions()
                .get_session(session.id())
                .expect("session should load")
                .active_prompt_for_agent(agent.id())
                .is_none(),
            "session mirror should not expose the active prompt"
        );

        let runtime = owned_runtime_state(&Arc::new(Mutex::new(app))).await;
        let error = runtime
            .agent_utility_provider_run(session.id(), agent.id(), "test utility")
            .await
            .expect_err("utility should be rejected while prompt owner has active prompt");

        assert!(
            error.to_string().contains("is busy"),
            "expected busy rejection, got {error}"
        );
    }
    #[tokio::test]
    async fn brief_utility_obeys_the_remaining_deadline() {
        use crate::provider::{
            AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult,
            ProviderUtilityExecutionPolicy, RuntimeProviderRun,
        };
        let app = Arc::new(Mutex::new(
            DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap(),
        ));
        let runtime = owned_runtime_state(&app).await;
        let request = LaunchProviderRequest::new("s", "claude", "claude", "default", "sonnet");
        let mut run = RuntimeProviderRun::new("short-utility", &request, ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed, process_label: "deadline-fixture".into(),
            pty_target: None, pty_program: Some("/bin/sh".into()),
            pty_args: vec!["-c".into(), r#"while IFS= read -r line; do sleep 1; printf '%s\n' '{"type":"assistant","message":{"content":[{"type":"text","text":"brief"}]}}' '{"type":"result","subtype":"success"}'; done"#.into()],
            pty_env: Default::default(), pty_env_remove: vec![], working_directory: None, structured_endpoint: None,
        });
        run.mark_running();
        runtime
            .owned
            .provider_store
            .write()
            .insert_run_for_test(run.clone());
        runtime
            .owned
            .provider_store
            .initialize_runtime(&run)
            .unwrap();
        let started = std::time::Instant::now();
        let result = crate::runtime::agent_utility_executor::run_provider_utility_prompt(
            &runtime,
            run.clone(),
            crate::runtime::agent_utility_executor::AgentUtilityPromptParts {
                visible_user_prompt: "brief".into(),
                hidden_system_context: String::new(),
            },
            "test remaining deadline",
            ProviderUtilityExecutionPolicy::MetadataOnlyDiscovery,
            std::time::Duration::from_millis(100),
        )
        .await;
        runtime
            .owned
            .provider_store
            .terminate_run_provider_only(run.session_id(), run.id())
            .unwrap();
        assert!(
            matches!(
                &result,
                Err(crate::error::DaemonError::ProviderProtocol {
                    operation: "claude_utility_timeout",
                    ..
                })
            ),
            "the remaining deadline must reach the Claude utility runner: {result:?}"
        );
        assert!(started.elapsed() < std::time::Duration::from_millis(900));
    }

    #[tokio::test]
    async fn headless_claude_runs_a_setup_turn_on_a_fresh_print_child() {
        use crate::provider::{
            AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult,
            ProviderUtilityExecutionPolicy, RuntimeProviderRun,
        };
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                "workspace-headless-setup",
                "worktree-headless-setup",
            ))
            .unwrap();
        let runtime = owned_runtime_state(&Arc::new(Mutex::new(app))).await;
        let request = LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude-headless",
            "default",
            "claude-headless/sonnet",
        );
        let root = std::env::temp_dir().join(format!("headless-setup-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let claude = root.join("claude");
        // Answers only when started as `claude -p` with the git and gh allowance.
        std::fs::write(
            &claude,
            r#"#!/usr/bin/python3
import sys, json
args = " ".join(sys.argv[1:])
if "-p " not in args or "Bash(git:*) Bash(gh:*)" not in args or "native-hooks.json" in args:
    sys.stderr.write("unexpected arguments: " + args + "\n")
    sys.exit(9)
for line in sys.stdin:
    print(json.dumps({"type": "assistant", "message": {"id": "m", "content": [{"type": "text", "text": "PUBLISHED https://github.com/me/x"}]}}), flush=True)
    print(json.dumps({"type": "result", "subtype": "success", "user_message_uuids": [json.loads(line)["uuid"]]}), flush=True)
"#,
        )
        .unwrap();
        std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let mut run = RuntimeProviderRun::new(
            "headless-setup",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "claude:headless".into(),
                pty_target: None,
                pty_program: Some(claude.display().to_string()),
                // The headless terminal's own arguments: the setup turn must not reuse them.
                pty_args: vec!["--settings".into(), "native-hooks.json".into()],
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        assert_eq!(agent.session_id(), session.id());
        let result = crate::runtime::agent_utility_executor::run_provider_utility_prompt(
            &runtime,
            run,
            crate::runtime::agent_utility_executor::AgentUtilityPromptParts {
                visible_user_prompt: "Publish".into(),
                hidden_system_context: "You are setting up a Room.".into(),
            },
            "test headless setup turn",
            ProviderUtilityExecutionPolicy::RoomCommands,
            std::time::Duration::from_secs(10),
        )
        .await;
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            result.expect("a headless Claude setup turn answers"),
            "PUBLISHED https://github.com/me/x"
        );
    }

    /// A Room in a fresh app with a Claude run whose provider program is `program`.
    async fn claude_setup_run(
        label: &str,
        program: &std::path::Path,
        args: Vec<String>,
    ) -> (KernelRuntimeState, crate::provider::RuntimeProviderRun) {
        use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};
        let mut app = DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, _agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(crate::session::CreateSessionRequest::new(
                format!("workspace-{label}"),
                format!("worktree-{label}"),
            ))
            .unwrap();
        let runtime = owned_runtime_state(&Arc::new(Mutex::new(app))).await;
        let request = LaunchProviderRequest::new(
            session.id(),
            "claude",
            "claude-headless",
            "default",
            "claude-headless/sonnet",
        );
        let mut run = crate::provider::RuntimeProviderRun::new(
            label,
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "claude:headless".into(),
                pty_target: None,
                pty_program: Some(program.display().to_string()),
                pty_args: args,
                pty_env: Default::default(),
                pty_env_remove: vec![],
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.mark_running();
        (runtime, run)
    }

    async fn setup_turn(
        runtime: &KernelRuntimeState,
        run: crate::provider::RuntimeProviderRun,
    ) -> Result<String, crate::error::DaemonError> {
        crate::runtime::agent_utility_executor::run_provider_utility_prompt(
            runtime,
            run,
            crate::runtime::agent_utility_executor::AgentUtilityPromptParts {
                visible_user_prompt: "Publish".into(),
                hidden_system_context: "You are setting up a Room.".into(),
            },
            "test setup turn",
            crate::provider::ProviderUtilityExecutionPolicy::RoomCommands,
            std::time::Duration::from_secs(10),
        )
        .await
    }

    fn write_executable(path: &std::path::Path, text: &str) {
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
    }

    #[tokio::test]
    async fn a_managed_setup_turn_runs_in_the_runs_own_folder() {
        let root = std::env::temp_dir().join(format!("managed-setup-{}", std::process::id()));
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        // Stands in for bubblewrap: enters the `--chdir` folder and starts the program after `--`.
        let sandbox = root.join("sandbox");
        write_executable(
            &sandbox,
            "#!/bin/sh\ndir=/\nwhile [ \"$1\" != \"--\" ]; do\n  [ \"$1\" = --chdir ] && dir=$2\n  shift\ndone\nshift\ncd \"$dir\" && exec \"$@\"\n",
        );
        let claude = root.join("claude");
        write_executable(
            &claude,
            &format!(
                r#"#!/usr/bin/python3
import sys, json, os
if os.getcwd() != {project:?}:
    sys.stderr.write("started in " + os.getcwd() + "\n")
    sys.exit(9)
for line in sys.stdin:
    print(json.dumps({{"type": "assistant", "message": {{"id": "m", "content": [{{"type": "text", "text": "PUBLISHED https://github.com/me/x"}}]}}}}), flush=True)
    print(json.dumps({{"type": "result", "subtype": "success", "user_message_uuids": [json.loads(line)["uuid"]]}}), flush=True)
"#,
                project = project.display().to_string()
            ),
        );
        let marker = "CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE";
        let (runtime, run) = claude_setup_run(
            "managed-setup",
            &sandbox,
            [
                "--setenv",
                marker,
                "1",
                "--chdir",
                &project.display().to_string(),
                "--",
                &claude.display().to_string(),
            ]
            .map(String::from)
            .into(),
        )
        .await;
        let result = setup_turn(&runtime, run).await;
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            result.expect("the setup turn answers from the run's folder"),
            "PUBLISHED https://github.com/me/x"
        );
    }

    #[tokio::test]
    async fn a_provider_limit_reaches_the_failed_publish_step() {
        let root = std::env::temp_dir().join(format!("limited-setup-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let claude = root.join("claude");
        write_executable(
            &claude,
            r#"#!/usr/bin/python3
import sys, json
for line in sys.stdin:
    print(json.dumps({"type": "result", "subtype": "success", "is_error": True, "result": "You've hit your weekly limit · resets Nov 18, 9pm (UTC)", "user_message_uuids": [json.loads(line)["uuid"]]}), flush=True)
"#,
        );
        let (runtime, run) = claude_setup_run("limited-setup", &claude, Vec::new()).await;
        let error = setup_turn(&runtime, run)
            .await
            .expect_err("a run at its weekly limit does not answer");
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            super::super::room_setup_publish::publish_failure_reason("claude", &error),
            "The agent could not publish: You've hit your weekly limit · resets Nov 18, 9pm (UTC)"
        );
    }
}
