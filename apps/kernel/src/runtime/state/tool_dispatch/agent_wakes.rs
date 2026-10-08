//! MP-08 / MP-09 / MP-10 / MP-11 A03: agent timers and watched processes.
use super::*;
use crate::durable_state::agent_lifecycle::{
    self as ledger, AgentTaskExecution, AgentWake, Operation,
};

const MAX_DELAY_MS: u64 = 7 * 86_400_000;

fn label(args: &serde_json::Value) -> Result<String, DaemonError> {
    let label = args["label"].as_str().unwrap_or_default().trim();
    if label.is_empty() || label.chars().count() > 200 {
        return Err(ledger::error("label of 1-200 characters required"));
    }
    let label = metadata(label);
    if label.trim().is_empty() {
        return Err(ledger::error("visible label required after sanitization"));
    }
    Ok(label)
}

fn metadata(text: &str) -> String {
    super::super::agent_process_output::sanitize(text.as_bytes())
        .chars()
        .filter(|c| !c.is_control())
        .collect()
}

fn new_wake(task: &AgentTaskExecution, kind: &str, label: String, now: u64) -> AgentWake {
    let id = format!("wake-{:032x}", rand::random::<u128>());
    AgentWake {
        registration_id: format!("completion-{id}"),
        id,
        task_id: task.task_id.clone(),
        room_id: task.room_id.clone(),
        agent_id: task.agent_id.clone(),
        kind: kind.into(),
        label,
        state: String::new(),
        created_at_ms: now,
        verified_at_ms: None,
        next_due_ms: None,
        interval_ms: None,
        command: vec![],
        match_text: None,
        matched_at_ms: None,
        pid: None,
        exit_code: None,
        fire_count: 0,
        missed_fires: 0,
        last_fired_at_ms: None,
        last_sequence: None,
        last_delivery: None,
        last_delivered_at_ms: None,
        last_acknowledged_at_ms: None,
        alerted_sequence: None,
    }
}

impl KernelRuntimeState {
    pub(super) async fn dispatch_agent_wake_tool(
        &self,
        run: &crate::provider::RuntimeProviderRun,
        name: &str,
        args: &serde_json::Value,
        task: &AgentTaskExecution,
    ) -> Result<serde_json::Value, DaemonError> {
        let store = &self.owned.durable_state_store;
        let now = crate::session::unix_epoch_ms();
        Ok(match name {
            "chariox.events.wakes" => {
                let wakes = super::agent_wake_scheduler::visible_wakes(
                    store.agent_wakes(Some(&task.room_id), Some(&task.agent_id))?,
                    now,
                );
                let receipts =
                    store.agent_wake_receipts(Some(&task.room_id), Some(&task.agent_id))?;
                serde_json::json!({"now_ms":now,"scheduler":self.wake_scheduler_health(now),"wakes":wakes,"receipts":receipts})
            }
            "chariox.events.timer" => {
                let delay = args["delay_ms"]
                    .as_u64()
                    .filter(|d| (1_000..=MAX_DELAY_MS).contains(d))
                    .ok_or_else(|| ledger::error("delay_ms between 1000 and 7 days required"))?;
                let interval = match args.get("interval_ms").filter(|v| !v.is_null()) {
                    None => None,
                    Some(v) => Some(
                        v.as_u64()
                            .filter(|i| (60_000..=MAX_DELAY_MS).contains(i))
                            .ok_or_else(|| {
                                ledger::error("interval_ms between 60000 and 7 days required")
                            })?,
                    ),
                };
                let mut wake = new_wake(task, "timer", label(args)?, now);
                wake.next_due_ms = Some(now + delay);
                wake.interval_ms = interval;
                store.agent_lifecycle(Operation::CreateWake {
                    task: task.task_id.clone(),
                    prompt: task.prompt_id.clone(),
                    wake: wake.clone(),
                })?;
                let verified = self.await_wake_verification(&wake.id).await;
                if verified.is_none() {
                    self.wake_notice(&wake, format!("Wake alert: '{}' was created but the wake scheduler did not confirm it within 5 s; the kernel sweep fires it when due", wake.label));
                } else {
                    self.refresh_wake_projection(&task.room_id);
                }
                serde_json::json!({"wake_id":wake.id,"registration_id":wake.registration_id,"first_fire_at_ms":wake.next_due_ms,"interval_ms":interval,"scheduler_confirmed":verified.is_some(),"verified_at_ms":verified})
            }
            "chariox.events.process" => {
                if !cfg!(target_os = "linux") {
                    return Err(ledger::error("watched processes require Linux process-group ownership verification on this release"));
                }
                // Only an agent that may already run any command unattended,
                // locally, can ask the kernel to run one for it.
                let agent = self.owned.agent_store.get_agent(&task.agent_id)?;
                if run.execution_mode() != crate::provider::AgentExecutionMode::Build
                    || run.permission_level() != crate::provider::AgentPermissionLevel::Yolo
                    || run.write_access_mode()
                        != crate::provider::ProviderWriteAccessMode::Unrestricted
                    || agent.remote_execution().is_some()
                {
                    return Err(ledger::error("watched processes require a local build agent with unattended command permission; run the command in your own shell instead"));
                }
                let root = run
                    .working_directory()
                    .and_then(|p| p.canonicalize().ok())
                    .ok_or_else(|| ledger::error("agent workspace unavailable"))?;
                let cwd = match args["cwd"].as_str().filter(|c| !c.is_empty()) {
                    None => root.clone(),
                    Some(rel) => root
                        .join(rel)
                        .canonicalize()
                        .ok()
                        .filter(|p| p.starts_with(&root) && p.is_dir())
                        .ok_or_else(|| {
                            ledger::error("cwd must be a directory inside the agent workspace")
                        })?,
                };
                let argv: Vec<String> = serde_json::from_value(args["argv"].clone())
                    .ok()
                    .filter(|a: &Vec<String>| {
                        !a.is_empty()
                            && a.len() <= 64
                            && !a[0].is_empty()
                            && a.iter().all(|s| s.len() <= 4_096 && !s.chars().any(char::is_control))
                    })
                    .ok_or_else(|| ledger::error("argv of 1-64 arguments required (no shell; control characters are refused)"))?;
                let match_text = match args["match_text"].as_str() {
                    Some(m) if m.is_empty() || m.len() > 200 => {
                        return Err(ledger::error("match_text of 1-200 bytes required"))
                    }
                    other => other.map(metadata),
                };
                if match_text.as_ref().is_some_and(|m| m.is_empty()) {
                    return Err(ledger::error(
                        "visible match_text required after sanitization",
                    ));
                }
                let mut wake = new_wake(task, "process", label(args)?, now);
                wake.command = argv.clone();
                wake.match_text = match_text;
                self.approve_agent_process(run, &wake, &cwd, &task.prompt_id)
                    .await?;
                self.authorize_current_external_command()?;
                let wake = self
                    .start_agent_process(wake, argv, cwd, &task.task_id, &task.prompt_id, run)
                    .await?;
                self.refresh_wake_projection(&task.room_id);
                serde_json::json!({"wake_id":wake.id,"registration_id":wake.registration_id,"pid":wake.pid,"verified_at_ms":wake.verified_at_ms})
            }
            "chariox.events.cancel_wake" => {
                let id = args["wake_id"]
                    .as_str()
                    .ok_or_else(|| ledger::error("wake_id required"))?;
                store.agent_lifecycle(Operation::CancelWake {
                    id: id.into(),
                    task: task.task_id.clone(),
                    prompt: Some(task.prompt_id.clone()),
                })?;
                let terminated = self.owned.agent_wakes.processes.terminate(id);
                self.refresh_wake_projection(&task.room_id);
                let wake = store
                    .agent_wakes(Some(&task.room_id), Some(&task.agent_id))?
                    .into_iter()
                    .find(|w| w.id == id);
                serde_json::json!({"cancel_requested":id,"process_signalled":terminated,"settled":wake.is_some_and(|w| w.state == "cancelled")})
            }
            _ => return Err(ledger::error("unknown wake tool")),
        })
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn security_f13_wake_label_strips_terminal_controls() {
        let result =
            label(&serde_json::json!({"label":"safe\u{1b}[2J\u{1b}]0;forged-title\u{7}\u{85}end"}))
                .unwrap();
        assert!(
            !result.chars().any(char::is_control),
            "unsafe label retained: {result:?}"
        );
        assert!(
            !result.contains("forged-title"),
            "OSC payload must not become visible metadata"
        );
    }

    async fn fixture(
        provider: &str,
        native: bool,
    ) -> (
        KernelRuntimeState,
        crate::provider::RuntimeProviderRun,
        AgentTaskExecution,
        crate::test_support::TestWorktree,
    ) {
        use crate::provider::*;
        let worktree = crate::test_support::TestWorktree::new("security-wake-admission");
        let mut app =
            crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
        let (session, agent) = crate::app::KernelSessionService::new(&mut app)
            .create_session(worktree.session_request())
            .unwrap();
        let mut request =
            LaunchProviderRequest::new(session.id(), provider, provider, "default", "test-model")
                .with_agent_id(agent.id())
                .with_execution_mode(AgentExecutionMode::Build)
                .with_permission_level(AgentPermissionLevel::Yolo)
                .with_client_interface(if native {
                    ProviderClientInterface::NativeTui
                } else {
                    ProviderClientInterface::Chariox
                });
        request.write_access_mode = ProviderWriteAccessMode::Unrestricted;
        let run = RuntimeProviderRun::new(
            "security-run",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::External,
                process_label: "security-wake".into(),
                pty_target: None,
                pty_program: None,
                pty_args: vec![],
                pty_env: BTreeMap::new(),
                pty_env_remove: vec![],
                working_directory: Some(worktree.path().to_path_buf()),
                structured_endpoint: None,
            },
        );
        let ledger::Outcome::Task(task) = app
            .durable_state_store()
            .agent_lifecycle(Operation::Begin {
                owner: "owner".into(),
                room: session.id().into(),
                agent: agent.id().into(),
                prompt: "security-turn".into(),
                run: Some(run.id().into()),
                now: crate::session::unix_epoch_ms(),
            })
            .unwrap()
        else {
            panic!("task required")
        };
        let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            Arc::new(tokio::sync::Mutex::new(app)),
            1,
        );
        (router.runtime_state(), run, task, worktree)
    }

    #[tokio::test]
    async fn security_f10_claude_native_yolo_cannot_launch_without_current_user_approval() {
        let (state, run, task, _worktree) = fixture("claude", true).await;
        let result = state
            .dispatch_agent_wake_tool(
                &run,
                "chariox.events.process",
                &serde_json::json!({"label":"approval-test", "argv":["/bin/sleep","0.1"]}),
                &task,
            )
            .await;
        state.owned.agent_wakes.processes.shutdown();
        assert!(
            result.is_err(),
            "launch-time bypass is insufficient without live approval: {result:?}"
        );
        assert!(
            state
                .owned
                .durable_state_store
                .agent_wakes(Some(&task.room_id), Some(&task.agent_id))
                .unwrap()
                .is_empty(),
            "denied calls must not create a launch intent"
        );
    }
    #[tokio::test]
    async fn review_r1_control_arguments_are_refused_before_launch() {
        let (state, run, task, _worktree) = fixture("codex", false).await;
        for argument in ["safe\u{1b}[31mready", "line\nnext", "safe\u{85}"] {
            let result = state
                .dispatch_agent_wake_tool(
                    &run,
                    "chariox.events.process",
                    &serde_json::json!({"label":"safe", "argv":["/bin/echo", argument]}),
                    &task,
                )
                .await;
            state.owned.agent_wakes.processes.shutdown();
            assert!(
                result.is_err(),
                "MP-11 R1: differing display/execution must be refused: {result:?}"
            );
        }
        assert!(state
            .owned
            .durable_state_store
            .agent_wakes(Some(&task.room_id), Some(&task.agent_id))
            .unwrap()
            .is_empty());
    }
    #[tokio::test]
    async fn review_r1_changed_argv_cannot_reuse_an_approval() {
        let (state, run, task, worktree) = fixture("codex", false).await;
        let mut wake = new_wake(
            &task,
            "process",
            "approved echo".into(),
            crate::session::unix_epoch_ms(),
        );
        wake.command = vec!["/bin/echo".into(), "two words".into(), "".into()];
        let result = state
            .start_agent_process(
                wake,
                vec!["/bin/echo".into(), "different".into()],
                worktree.path().to_path_buf(),
                &task.task_id,
                &task.prompt_id,
                &run,
            )
            .await;
        state.owned.agent_wakes.processes.shutdown();
        assert!(
            result.is_err(),
            "MP-11 R1: a changed vector requires fresh approval"
        );
        assert!(state
            .owned
            .durable_state_store
            .agent_wakes(Some(&task.room_id), Some(&task.agent_id))
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn review_r1_safe_argument_boundaries_survive_metadata_retention() {
        let (state, run, task, _worktree) = fixture("codex", false).await;
        let argv = vec!["/bin/echo", "two words", "", "quote\"and\\slash"];
        let result = state.dispatch_agent_wake_tool(&run, "chariox.events.process",
            &serde_json::json!({"label":"safe\u{1b}[2Jlabel", "argv":argv, "match_text":"\u{1b}[31mready\u{85}"}), &task).await;
        state.owned.agent_wakes.processes.shutdown();
        result.unwrap();
        let wake = state
            .owned
            .durable_state_store
            .agent_wakes(Some(&task.room_id), Some(&task.agent_id))
            .unwrap()
            .remove(0);
        assert_eq!(wake.command, argv);
        assert!(!wake.label.chars().any(char::is_control));
        assert_eq!(wake.match_text.as_deref(), Some("ready"));
        let shown = serde_json::to_string(&wake.command).unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&shown).unwrap(),
            wake.command
        );
    }
}
