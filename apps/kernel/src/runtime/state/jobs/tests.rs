use super::*;
use crate::provider::*;

async fn fixture() -> (
    KernelRuntimeState,
    RuntimeProviderRun,
    AgentTaskExecution,
    crate::test_support::TestWorktree,
) {
    let worktree = crate::test_support::TestWorktree::new("offload-job-entry");
    let mut app = crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let (session, agent) = crate::app::KernelSessionService::new(&mut app)
        .create_session(worktree.session_request())
        .unwrap();
    let mut request =
        LaunchProviderRequest::new(session.id(), "codex", "codex", "default", "test-model")
            .with_agent_id(agent.id())
            .with_execution_mode(AgentExecutionMode::Build)
            .with_permission_level(AgentPermissionLevel::Yolo)
            .with_client_interface(ProviderClientInterface::Chariox);
    request.write_access_mode = ProviderWriteAccessMode::Unrestricted;
    let mut run = RuntimeProviderRun::new(
        format!("offload-test-run-{}", rand::random::<u64>()),
        &request,
        ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::External,
            process_label: "offload-job-test".into(),
            pty_target: None,
            pty_program: None,
            pty_args: vec![],
            pty_env: BTreeMap::new(),
            pty_env_remove: vec![],
            working_directory: Some(worktree.path().to_owned()),
            structured_endpoint: None,
        },
    );
    let ledger::Outcome::Task(task) = app
        .durable_state_store()
        .agent_lifecycle(ledger::Operation::Begin {
            owner: "owner".into(),
            room: session.id().into(),
            agent: agent.id().into(),
            prompt: "offload-test-turn".into(),
            run: Some(run.id().into()),
            now: crate::session::unix_epoch_ms(),
        })
        .unwrap()
    else {
        panic!("task required")
    };
    run.mark_running();
    app.providers_mut().insert_run_for_test(run.clone());
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::new(tokio::sync::Mutex::new(app)),
        1,
    );
    (
        router.runtime_state().with_job_router(router),
        run,
        task,
        worktree,
    )
}

#[tokio::test]
async fn offload_job_local_completion_has_durable_result_and_log() {
    let (state, run, task, _worktree) = fixture().await;
    let result = state.dispatch_job_tool(Some(&run), "chariox.jobs.run", &serde_json::json!({"command":["/bin/sh","-c","printf 'job-done\\n'; exit 7"],"wait_s":5}), &task).await.unwrap();
    assert_eq!(result["placement"], "local");
    assert_eq!(result["result"]["exit_code"], 7);
    assert_eq!(result["result"]["outcome"], "completed");
    assert!(result["result"]["output_tail"]
        .as_str()
        .unwrap()
        .contains("job-done"));
    assert!(Path::new(result["result"]["log"].as_str().unwrap()).exists());
    let status = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.status",
            &serde_json::json!({"job_id":result["job_id"]}),
            &task,
        )
        .await
        .unwrap();
    assert_eq!(status["result"], result["result"]);
    let wake_id = state
        .job_wake_driver
        .as_ref()
        .unwrap()
        .read(result["job_id"].as_str().unwrap())
        .unwrap()
        .wake_id;
    assert_eq!(
        state
            .owned
            .durable_state_store
            .agent_wake(&wake_id)
            .unwrap()
            .unwrap()
            .state,
        "exited"
    );
}

#[tokio::test]
async fn offload_job_time_limit_stops_owned_command() {
    let (state, run, task, _worktree) = fixture().await;
    let result = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.run",
            &serde_json::json!({"command":["/bin/sleep","30"],"timeout_s":1,"wait_s":10}),
            &task,
        )
        .await
        .unwrap();
    assert_eq!(result["result"]["outcome"], "time_limit");
}

#[test]
fn offload_job_request_rejects_env_values_and_escaping_paths() {
    for value in [
        serde_json::json!({"command":["echo"],"env":["TOKEN=value"]}),
        serde_json::json!({"command":["echo"],"cwd":"../escape"}),
        serde_json::json!({"command":["echo"],"outputs":["/etc/passwd"]}),
        serde_json::json!({"command":["echo"],"wait_s":601}),
    ] {
        let request = serde_json::from_value::<RunJobRequest>(value).unwrap();
        assert!(validate(&request).is_err());
    }
}

struct Worker {
    router: crate::runtime::router::CommandRouter,
    home: PathBuf,
}
impl crate::room_exec::gateway_placement::GrantedSend for Worker {
    fn send<'a>(
        &'a self,
        id: String,
        request: &'a LocalDaemonRequest,
    ) -> crate::room_exec::gateway_placement::SendFuture<'a> {
        Box::pin(async move {
            if matches!(request, LocalDaemonRequest::StatKernelFile(crate::local::StatKernelFileRequest { path, .. }) if path == "~/")
            {
                return Ok(serde_json::to_value(LocalDaemonResponse::KernelFileStat {
                    stat: crate::local::KernelFileStat {
                        path: self.home.to_string_lossy().into_owned(),
                        exists: true,
                        kind: Some(crate::local::KernelFileKind::Directory),
                        size: None,
                        modified_ms: None,
                        revision: None,
                        mode: Some(0o755),
                        sha256: None,
                        partial_size: None,
                    },
                })
                .unwrap());
            }
            let command =
                crate::runtime::command::KernelCommand::from_local_request(id, None, None, request);
            self.router
                .dispatch(command, request.clone())
                .await
                .map(|response| serde_json::to_value(response).unwrap())
                .map_err(
                    |error| crate::runtime::kernel_gateway::GatewayError::Remote {
                        name: "worker".into(),
                        code: "job_test_failure".into(),
                        message: error.to_string(),
                        retryable: false,
                    },
                )
        })
    }
}

#[tokio::test]
async fn offload_job_remote_upload_exec_environment_artifact_and_conflict_return() {
    use crate::local::{JobCommand, JobNeeds};
    let source = crate::test_support::TestWorktree::new("offload-source");
    let home = crate::test_support::TestWorktree::new("offload-worker-home");
    std::fs::write(source.path().join("source.txt"), b"base").unwrap();
    std::fs::write(source.path().join(".env"), b"not-uploaded").unwrap();
    let app = crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    let router = crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
        Arc::new(tokio::sync::Mutex::new(app)),
        2,
    );
    let worker = Arc::new(Worker {
        router,
        home: home.path().to_owned(),
    });
    let record = Record { job_id: "job-remote-test".into(), wake_id: "wake-remote-test".into(), registration_id: "completion-wake-remote-test".into(), room_id: "home-room".into(), agent_id: "home-agent".into(), request: RunJobRequest { command: JobCommand::Argv(vec!["/bin/sh".into(), "-c".into(), "printf remote > source.txt; mkdir -p target; printf artifact > target/build.bin; printf '%s' \"$NAMED\"".into()]), cwd: None, env: vec!["NAMED".into()], needs: JobNeeds::default(), timeout_s: 30, wait_s: 0, outputs: vec!["target/*.bin".into()] }, workspace: source.path().to_owned(), cwd: source.path().to_owned(), placement: JobPlacement::Remote, started_at_ms: crate::session::unix_epoch_ms(), running_at_ms: None, state: "uploading".into(), allocation_ref: None, capability_revision: None, machine_label: Some("offload-test".into()), address: Some("2001:db8::1".into()), result: None, exec_id: None, class_id: None, rate_per_minute: None, estimate_micros: None, deleted: false, remote_available: true, remote_refusal: None, estimated_minutes: None, memory_mib: None };
    let base = crate::jobs::snapshot(source.path(), &record.request.outputs).unwrap();
    let upload = crate::jobs::snapshot(source.path(), &[]).unwrap();
    let (_cancel, cancelled) = watch::channel(false);
    let (remote, root) = remote::Remote::prepare(worker, &record, &upload, &cancelled)
        .await
        .unwrap();
    assert!(!Path::new(&root).join(".env").exists());
    let exec = remote
        .open(validate(&record.request).unwrap(), true, "test-command")
        .await
        .unwrap();
    let input = serde_json::json!({"argv":validate(&record.request).unwrap(),"env":{"NAMED":"selected-entry"},"job_id":record.job_id}).to_string() + "\n";
    let next = remote.write(&exec, 0, input.as_bytes()).await.unwrap();
    remote.close_stdin(&exec, next).await.unwrap();
    let mut cursor = 0;
    let mut bytes = vec![];
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let page = remote.read(&exec, cursor).await.unwrap();
            cursor = page.next_cursor;
            for chunk in page.chunks {
                bytes.extend(
                    base64::engine::general_purpose::STANDARD
                        .decode(chunk.data_base64)
                        .unwrap(),
                );
            }
            if let Some(outcome) = page.outcome {
                assert_eq!(outcome, crate::room_exec::ExecOutcome::Exited { code: 0 });
                break;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(bytes, b"selected-entry");
    std::fs::write(source.path().join("source.txt"), b"local edit").unwrap();
    let (files, conflicts) = remote
        .return_files(&root, &record, &base, &upload)
        .await
        .unwrap();
    assert_eq!(conflicts, ["source.txt"]);
    assert_eq!(files, ["target/build.bin"]);
    assert_eq!(
        std::fs::read(source.path().join("source.txt")).unwrap(),
        b"local edit"
    );
    assert_eq!(
        std::fs::read(
            source
                .path()
                .join(".chariox/jobs/job-remote-test/conflicts/source.txt")
        )
        .unwrap(),
        b"remote"
    );
    assert_eq!(
        std::fs::read(source.path().join("target/build.bin")).unwrap(),
        b"artifact"
    );
}
