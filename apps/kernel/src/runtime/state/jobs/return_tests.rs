//! What a finished remote command sends back: file modes, and files that cannot be fetched yet.
use super::tests::Worker;
use super::*;
use crate::local::{JobCommand, JobNeeds};
use crate::room_exec::gateway_placement::GrantedSend;
use std::os::unix::fs::PermissionsExt;

pub(super) fn worker_kernel(home: &Path) -> Arc<Worker> {
    let app = crate::DaemonApp::bootstrap(crate::config::DaemonConfig::for_tests()).unwrap();
    Arc::new(Worker {
        router: crate::runtime::router::CommandRouter::with_interactive_capacity_from_app(
            Arc::new(tokio::sync::Mutex::new(app)),
            2,
        ),
        home: home.to_owned(),
    })
}

pub(super) fn remote_record(source: &Path, script: &str, outputs: &[&str]) -> Record {
    Record {
        job_id: "job-return-test".into(),
        wake_id: "wake-return-test".into(),
        registration_id: "completion-wake-return-test".into(),
        room_id: "home-room".into(),
        agent_id: "home-agent".into(),
        request: RunJobRequest {
            command: JobCommand::Argv(vec!["/bin/sh".into(), "-c".into(), script.into()]),
            cwd: None,
            env: vec![],
            needs: JobNeeds::default(),
            timeout_s: 30,
            wait_s: 0,
            outputs: outputs.iter().map(|pattern| (*pattern).into()).collect(),
            machine: None,
            machine_only: false,
        },
        workspace: source.to_owned(),
        cwd: source.to_owned(),
        placement: JobPlacement::Remote,
        started_at_ms: crate::session::unix_epoch_ms(),
        running_at_ms: None,
        state: "uploading".into(),
        allocation_ref: None,
        capability_revision: None,
        machine_label: Some("offload-test".into()),
        held_kernel_id: None,
        address: Some("2001:db8::1".into()),
        result: None,
        exec_id: None,
        class_id: None,
        rate_per_minute: None,
        estimate_micros: None,
        deleted: false,
        remote_available: true,
        remote_refusal: None,
        estimated_minutes: None,
        memory_mib: None,
        remote: RemoteStage::Idle,
        cancel_cause: None,
    }
}

pub(super) struct FinishedCommand {
    pub(super) remote: remote::Remote,
    pub(super) root: String,
    pub(super) record: Record,
    pub(super) base: crate::jobs::Manifest,
    pub(super) upload: crate::jobs::Manifest,
}

/// Uploads `source`, runs `script` on the worker and waits for it to exit 0.
pub(super) async fn finish_on_worker(
    source: &Path,
    worker: Arc<dyn GrantedSend>,
    script: &str,
    outputs: &[&str],
) -> FinishedCommand {
    let record = remote_record(source, script, outputs);
    let workspace = record.transfer_workspace().unwrap();
    let base = workspace.snapshot(&record.request.outputs).unwrap();
    let upload = workspace.snapshot(&[]).unwrap();
    let (_cancel, cancelled) = watch::channel(false);
    let located = remote::Remote::locate(worker.clone(), &record.job_id)
        .await
        .unwrap();
    let remote = remote::Remote::prepare(worker, &record, &located, &upload, &cancelled)
        .await
        .unwrap();
    let root = located.root;
    let argv = validate(&record.request).unwrap();
    let exec = remote
        .open(argv.clone(), Some(JobNeeds::default()), "test-command")
        .await
        .unwrap();
    let input = serde_json::json!({"argv":argv,"env":{},"job_id":record.job_id}).to_string() + "\n";
    let next = remote.write(&exec, 0, input.as_bytes()).await.unwrap();
    remote.close_stdin(&exec, next).await.unwrap();
    let mut cursor = 0;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let page = remote.read(&exec, cursor).await.unwrap();
            cursor = page.next_cursor;
            if let Some(outcome) = page.outcome {
                assert_eq!(outcome, crate::room_exec::ExecOutcome::Exited { code: 0 });
                break;
            }
        }
    })
    .await
    .unwrap();
    FinishedCommand {
        remote,
        root,
        record,
        base,
        upload,
    }
}

fn executable(path: &Path) -> bool {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o111 != 0
}

#[tokio::test]
async fn offload_job_return_applies_a_mode_only_change_and_checks_it_for_conflicts() {
    let source = crate::test_support::TestWorktree::new("offload-mode-source");
    let home = crate::test_support::TestWorktree::new("offload-mode-home");
    let scripts = source.path().join("scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    for (name, mode, body) in [
        ("build.sh", 0o644, "#!/bin/sh\n"),
        ("run.sh", 0o755, "#!/bin/sh\n"),
        ("tool.sh", 0o644, "a"),
    ] {
        std::fs::write(scripts.join(name), body).unwrap();
        std::fs::set_permissions(scripts.join(name), std::fs::Permissions::from_mode(mode))
            .unwrap();
    }
    let finished = finish_on_worker(
        source.path(),
        worker_kernel(home.path()),
        "chmod +x scripts/build.sh; chmod -x scripts/run.sh; printf b > scripts/tool.sh",
        &[],
    )
    .await;
    // Another local edit of the same file's permissions while the job ran.
    std::fs::set_permissions(
        scripts.join("tool.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let (mut files, conflicts) = finished
        .remote
        .return_files(
            &finished.root,
            &finished.record,
            &finished.base,
            &finished.upload,
        )
        .await
        .unwrap();
    files.sort();
    assert_eq!(files, ["scripts/build.sh", "scripts/run.sh"]);
    assert_eq!(conflicts, ["scripts/tool.sh"]);
    assert!(
        executable(&scripts.join("build.sh")),
        "chmod +x on the worker must reach the home copy"
    );
    assert!(
        !executable(&scripts.join("run.sh")),
        "a removed executable bit must reach the home copy"
    );
    assert_eq!(std::fs::read(scripts.join("tool.sh")).unwrap(), b"a");
    assert!(executable(&scripts.join("tool.sh")));
    assert_eq!(
        std::fs::read(
            source
                .path()
                .join(".chariox/jobs/job-return-test/conflicts/scripts/tool.sh")
        )
        .unwrap(),
        b"b"
    );
}

#[tokio::test]
async fn offload_job_return_brings_back_files_below_nested_ignored_folders_of_a_recursive_output() {
    let source = crate::test_support::TestWorktree::new("offload-nested-source");
    let home = crate::test_support::TestWorktree::new("offload-nested-home");
    let finished = finish_on_worker(
        source.path(),
        worker_kernel(home.path()),
        "mkdir -p target/release/deps; printf app > target/release/app; printf lib > target/release/deps/lib.rlib",
        &["target/**"],
    )
    .await;
    let (mut files, conflicts) = finished
        .remote
        .return_files(
            &finished.root,
            &finished.record,
            &finished.base,
            &finished.upload,
        )
        .await
        .unwrap();
    files.sort();
    assert_eq!(
        files,
        ["target/release/app", "target/release/deps/lib.rlib"]
    );
    assert!(conflicts.is_empty());
    assert_eq!(
        std::fs::read(source.path().join("target/release/app")).unwrap(),
        b"app"
    );
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[tokio::test]
async fn offload_job_return_keeps_the_home_copys_read_and_write_bits() {
    let source = crate::test_support::TestWorktree::new("offload-bits-source");
    let home = crate::test_support::TestWorktree::new("offload-bits-home");
    let scripts = source.path().join("scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    for (name, home_mode, body) in [
        ("note.txt", 0o644, "n"),
        ("build.sh", 0o644, "#!/bin/sh\n"),
        ("shared.sh", 0o664, "#!/bin/sh\n"),
        ("run.sh", 0o755, "#!/bin/sh\n"),
        ("both.sh", 0o640, "a"),
    ] {
        std::fs::write(scripts.join(name), body).unwrap();
        std::fs::set_permissions(
            scripts.join(name),
            std::fs::Permissions::from_mode(home_mode),
        )
        .unwrap();
    }
    let finished = finish_on_worker(
        source.path(),
        worker_kernel(home.path()),
        "printf m > scripts/note.txt; chmod +x scripts/build.sh scripts/shared.sh; chmod -x scripts/run.sh; chmod +x scripts/both.sh; printf b > scripts/both.sh",
        &[],
    )
    .await;
    let (mut files, conflicts) = finished
        .remote
        .return_files(
            &finished.root,
            &finished.record,
            &finished.base,
            &finished.upload,
        )
        .await
        .unwrap();
    files.sort();
    assert_eq!(
        files,
        [
            "scripts/both.sh",
            "scripts/build.sh",
            "scripts/note.txt",
            "scripts/run.sh",
            "scripts/shared.sh"
        ]
    );
    assert!(conflicts.is_empty());
    assert_eq!(std::fs::read(scripts.join("note.txt")).unwrap(), b"m");
    for (name, expected) in [
        ("note.txt", 0o644),
        ("build.sh", 0o755),
        ("shared.sh", 0o775),
        ("run.sh", 0o644),
        ("both.sh", 0o750),
    ] {
        assert_eq!(mode(&scripts.join(name)), expected, "{name}");
    }
}

/// Fails one artifact read as a relay outage would, until it is disarmed.
pub(super) struct Outage {
    worker: Arc<Worker>,
    path: &'static str,
    pub(super) cut: std::sync::atomic::AtomicBool,
    /// Every request fails, as when the relay or the worker is gone.
    pub(super) down: std::sync::atomic::AtomicBool,
    /// Opening the command fails, as when the worker refuses it.
    pub(super) open_down: std::sync::atomic::AtomicBool,
}
impl GrantedSend for Outage {
    fn send<'a>(
        &'a self,
        id: String,
        request: &'a LocalDaemonRequest,
    ) -> crate::room_exec::gateway_placement::SendFuture<'a> {
        if self.open_down.load(std::sync::atomic::Ordering::SeqCst)
            && matches!(request, LocalDaemonRequest::RunCommand(room) if matches!(room.operation, crate::local::RunCommandOperation::ExecOpenJob { .. }))
        {
            return Box::pin(async {
                Err(crate::runtime::kernel_gateway::GatewayError::Remote {
                    name: "worker".into(),
                    code: "open_refused".into(),
                    message: "the worker refused the command".into(),
                    retryable: false,
                })
            });
        }
        if self.down.load(std::sync::atomic::Ordering::SeqCst)
            || (self.cut.load(std::sync::atomic::Ordering::SeqCst)
                && matches!(request, LocalDaemonRequest::ReadKernelFile(read) if read.path.ends_with(self.path)))
        {
            return Box::pin(async {
                Err(crate::runtime::kernel_gateway::GatewayError::Remote {
                    name: "worker".into(),
                    code: "relay_outage".into(),
                    message: "relay outage".into(),
                    retryable: false,
                })
            });
        }
        self.worker.send(id, request)
    }
}

pub(super) struct HeldWorker {
    pub(super) state: KernelRuntimeState,
    pub(super) run: crate::provider::RuntimeProviderRun,
    pub(super) task: AgentTaskExecution,
    pub(super) workspace: crate::test_support::TestWorktree,
    pub(super) _home: crate::test_support::TestWorktree,
    pub(super) outage: Arc<Outage>,
    pub(super) job_id: String,
}

/// A job record of a remote command whose worker is the test's own second kernel.
pub(super) struct Prepared {
    pub(super) state: KernelRuntimeState,
    pub(super) run: crate::provider::RuntimeProviderRun,
    pub(super) task: AgentTaskExecution,
    pub(super) workspace: crate::test_support::TestWorktree,
    pub(super) home: crate::test_support::TestWorktree,
    pub(super) outage: Arc<Outage>,
    pub(super) record: Record,
    pub(super) machine: super::super::job_compute::JobMachine,
}

pub(super) fn test_machine(grant: Arc<Outage>) -> super::super::job_compute::JobMachine {
    super::super::job_compute::JobMachine {
        machine_label: "offload-test".into(),
        address: None,
        kernel_id: "worker".into(),
        grant,
        class: Some(super::super::job_compute::JobClass {
            allocation_ref: "cleanup-test".into(),
            capability_revision: "v1".into(),
            class_id: "test-class".into(),
            rate_per_minute: crate::local::JobMachineRate {
                amount_micros: 1,
                currency: "usd".into(),
            },
        }),
    }
}

pub(super) async fn prepared(script: &str, outputs: &[&str]) -> Prepared {
    let (state, run, task, workspace) = super::tests::fixture().await;
    let home = crate::test_support::TestWorktree::new("offload-outage-home");
    let outage = Arc::new(Outage {
        worker: worker_kernel(home.path()),
        path: "target/second.bin",
        cut: true.into(),
        down: false.into(),
        open_down: false.into(),
    });
    let started = state
        .dispatch_job_tool(
            Some(&run),
            "chariox.jobs.run",
            &serde_json::json!({"command":["/bin/echo","placeholder"],"wait_s":5}),
            &task,
        )
        .await
        .unwrap();
    let driver = state.job_wake_driver.as_ref().unwrap();
    let mut record = driver.read(started["job_id"].as_str().unwrap()).unwrap();
    record.request = remote_record(workspace.path(), script, outputs).request;
    record.placement = JobPlacement::Remote;
    record.state = "uploading".into();
    record.result = None;
    record.allocation_ref = Some("cleanup-test".into());
    record.capability_revision = Some("v1".into());
    record.deleted = false;
    let machine = test_machine(outage.clone());
    Prepared {
        state,
        run,
        task,
        workspace,
        home,
        outage,
        record,
        machine,
    }
}

impl Prepared {
    /// Runs the command and settles its job the way the driver does.
    pub(super) async fn run(&mut self) -> Result<JobResult, DaemonError> {
        let mut output = super::execute::Output::new(&self.state, &self.record).unwrap();
        let (_cancel, mut cancelled) = watch::channel(false);
        let result = self
            .state
            .run_remote_job(
                &mut self.record,
                &self.machine,
                &BTreeMap::new(),
                &mut output,
                &mut cancelled,
            )
            .await;
        self.state
            .complete_remote_job(&mut self.record, &mut output, result, false)
            .await
    }
}

/// A command that made two artifacts finished on the worker while the read of the second failed.
pub(super) async fn worker_held_after_failed_return() -> HeldWorker {
    let mut job = prepared(
        "mkdir -p target; printf one > target/first.bin; printf two > target/second.bin",
        &["target/*.bin"],
    )
    .await;
    // The worker is told its job started; any later request is the release.
    let release = super::cleanup_tests::service(
        &job.state,
        vec![
            ("job_started", 200, super::cleanup_tests::machine("ready")),
            ("get", 200, super::cleanup_tests::machine("deleted")),
        ],
    )
    .await;
    job.run().await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !release.is_finished(),
        "the worker was released while a file it produced had not come home"
    );
    release.abort();
    job.record.state = "finished".into();
    let driver = job.state.job_wake_driver.as_ref().unwrap();
    driver.save(&job.record).unwrap();
    let saved = driver.read(&job.record.job_id).unwrap();
    assert!(saved.remote.holds_worker() && !saved.deleted);
    assert!(saved
        .result
        .unwrap()
        .output_tail
        .contains("files could not all be returned"));
    assert_eq!(
        std::fs::read(job.workspace.path().join("target/first.bin")).unwrap(),
        b"one"
    );
    assert!(!job.workspace.path().join("target/second.bin").exists());
    HeldWorker {
        state: job.state,
        run: job.run,
        task: job.task,
        workspace: job.workspace,
        _home: job.home,
        outage: job.outage,
        job_id: job.record.job_id,
    }
}

pub(super) async fn release_service(state: &KernelRuntimeState) -> tokio::task::JoinHandle<()> {
    super::cleanup_tests::service(
        state,
        vec![("get", 200, super::cleanup_tests::machine("deleted"))],
    )
    .await
}

impl HeldWorker {
    pub(super) async fn job_tool(&self, name: &str) -> serde_json::Value {
        tokio::time::timeout(
            super::cleanup_tests::SCRIPTED_REQUEST_DEADLINE,
            self.state.dispatch_job_tool(
                Some(&self.run),
                name,
                &serde_json::json!({"job_id":self.job_id}),
                &self.task,
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("{name} never answered"))
        .unwrap()
    }
}

#[tokio::test]
async fn offload_job_keeps_the_worker_until_files_that_could_not_be_returned_are_collected() {
    let held = worker_held_after_failed_return().await;
    held.outage
        .cut
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let release = release_service(&held.state).await;
    let status = held.job_tool("chariox.jobs.status").await;
    release.await.unwrap();
    assert_eq!(
        std::fs::read(held.workspace.path().join("target/second.bin")).unwrap(),
        b"two"
    );
    assert_eq!(status["deleted"], true);
    assert_eq!(status["return_pending"], false);
    assert_eq!(status["result"]["exit_code"], 0);
    assert_eq!(status["result"]["outcome"], "completed");
    assert_eq!(
        status["result"]["files_back"],
        serde_json::json!(["target/first.bin", "target/second.bin"])
    );
}

#[tokio::test]
async fn offload_job_status_keeps_the_worker_while_its_files_still_cannot_be_returned() {
    let held = worker_held_after_failed_return().await;
    let status = held.job_tool("chariox.jobs.status").await;
    assert_eq!(status["deleted"], false);
    assert_eq!(status["return_pending"], true);
    assert!(status["return_error"]
        .as_str()
        .unwrap()
        .contains("relay outage"));
    assert!(!held.workspace.path().join("target/second.bin").exists());
}

#[tokio::test]
async fn offload_job_cancel_gives_up_the_files_and_deletes_the_worker() {
    let held = worker_held_after_failed_return().await;
    let release = release_service(&held.state).await;
    let status = held.job_tool("chariox.jobs.cancel").await;
    release.await.unwrap();
    assert_eq!(status["deleted"], true);
    assert_eq!(status["return_pending"], false);
    assert!(!held.workspace.path().join("target/second.bin").exists());
}

pub(super) fn job_state_path(state: &KernelRuntimeState, relative: &str) -> PathBuf {
    state
        .owned
        .config_projection
        .snapshot()
        .private_runtime_state_root()
        .join("jobs")
        .join(relative)
}

#[tokio::test]
async fn offload_job_has_its_return_plan_and_worker_hold_saved_before_the_command_runs() {
    let probe = crate::test_support::TestWorktree::new("offload-plan-probe");
    let marker = probe.path().join("marker");
    let mut job = prepared("true", &[]).await;
    let id = job.record.job_id.clone();
    let record = job_state_path(&job.state, &format!("{id}.json"));
    let plan = job_state_path(&job.state, &format!("{id}/return.json"));
    job.record.request.command = JobCommand::Argv(vec![
        "/bin/sh".into(),
        "-c".into(),
        format!(
            "grep -Eq '\"remote\":(\"Planned\"|[{{]\"Running\")' {record:?} && test -f {plan:?} && printf kept > {marker:?}"
        ),
    ]);
    job.outage
        .cut
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let release = super::cleanup_tests::service(
        &job.state,
        vec![
            ("job_started", 200, super::cleanup_tests::machine("ready")),
            ("get", 200, super::cleanup_tests::machine("deleted")),
        ],
    )
    .await;
    job.run().await.unwrap();
    release.await.unwrap();
    assert_eq!(
        std::fs::read(&marker).unwrap_or_default(),
        b"kept",
        "the command ran before the job's file return was recoverable"
    );
}

#[tokio::test]
async fn offload_job_does_not_run_its_command_when_its_return_plan_cannot_be_kept() {
    let probe = crate::test_support::TestWorktree::new("offload-plan-refused");
    let marker = probe.path().join("marker");
    let mut job = prepared("true", &[]).await;
    job.record.request.command = JobCommand::Argv(vec![
        "/bin/sh".into(),
        "-c".into(),
        format!("printf ran > {marker:?}"),
    ]);
    // A directory where the plan file goes: its write cannot succeed.
    std::fs::create_dir_all(job_state_path(
        &job.state,
        &format!("{}/return.json/occupied", job.record.job_id),
    ))
    .unwrap();
    let release = super::cleanup_tests::service(
        &job.state,
        vec![("get", 200, super::cleanup_tests::machine("deleted"))],
    )
    .await;
    let result = job.run().await.unwrap();
    release.await.unwrap();
    assert!(
        !marker.exists(),
        "the command ran although its outputs could not be tracked"
    );
    assert!(result.output_tail.contains("return plan"));
    assert!(job.record.deleted && !job.record.remote.holds_worker());
}

#[tokio::test]
async fn offload_job_after_a_restart_reconciles_the_kept_worker_with_the_service_before_collecting()
{
    let held = worker_held_after_failed_return().await;
    // A restarted kernel has no live grant and its gateway does not know the private worker.
    held.state
        .job_wake_driver
        .as_ref()
        .unwrap()
        .return_grants
        .lock()
        .unwrap()
        .remove(&held.job_id);
    // The service names the allocation but has no reachable target for it.
    let service = super::cleanup_tests::service(
        &held.state,
        vec![("get", 200, super::cleanup_tests::machine("ready"))],
    )
    .await;
    let status = held.job_tool("chariox.jobs.status").await;
    tokio::time::timeout(Duration::from_secs(5), service)
        .await
        .expect("the saved allocation was never reconciled through the control plane")
        .unwrap();
    assert_eq!(status["return_pending"], true);
    assert_eq!(status["deleted"], false);
    assert!(status["return_error"]
        .as_str()
        .unwrap()
        .contains("no reachable"));
}

/// A home kernel that died while the command still ran on its worker: the record is what the
/// last save left, the worker kept running, and the grant of the dead process is gone with it.
pub(super) struct Crashed {
    pub(super) held: HeldWorker,
    pub(super) service: tokio::task::JoinHandle<()>,
}

pub(super) async fn crashed_while_running(
    script: &str,
    responses: Vec<(&'static str, u16, serde_json::Value)>,
) -> Crashed {
    let job = prepared(script, &["target/*.bin"]).await;
    job.outage
        .cut
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let driver = job.state.job_wake_driver.as_ref().unwrap();
    driver.save(&job.record).unwrap();
    let service = super::cleanup_tests::service(&job.state, responses).await;
    let home = {
        let state = job.state.clone();
        let mut record = driver.read(&job.record.job_id).unwrap();
        let machine = test_machine(job.outage.clone());
        tokio::spawn(async move {
            let mut output = super::execute::Output::new(&state, &record).unwrap();
            let (_cancel, mut cancelled) = watch::channel(false);
            let _ = state
                .run_remote_job(
                    &mut record,
                    &machine,
                    &BTreeMap::new(),
                    &mut output,
                    &mut cancelled,
                )
                .await;
        })
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        while driver.read(&job.record.job_id).unwrap().state != "running" {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the command never started");
    home.abort();
    let _ = home.await;
    Crashed {
        held: HeldWorker {
            state: job.state,
            run: job.run,
            task: job.task,
            workspace: job.workspace,
            _home: job.home,
            outage: job.outage,
            job_id: job.record.job_id,
        },
        service,
    }
}

impl Crashed {
    /// The restarted kernel recovers the job's wake the way startup does.
    pub(super) fn restart(&self) {
        recover_job(&self.held.state, &self.held.job_id);
    }

    pub(super) async fn finished(&self) -> serde_json::Value {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let status = self.held.job_tool("chariox.jobs.status").await;
                if status["state"] == "finished" {
                    return status;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("the job never finished")
    }
}

/// The restarted kernel recovers the job's wake the way startup does.
pub(super) fn recover_job(state: &KernelRuntimeState, job_id: &str) {
    let driver = state.job_wake_driver.as_ref().unwrap();
    let wake_id = driver.read(job_id).unwrap().wake_id;
    let wake = state
        .owned
        .durable_state_store
        .agent_wake(&wake_id)
        .unwrap()
        .unwrap();
    assert!(matches!(driver.recover(&wake), Recovery::Reattached));
}

pub(super) const TWO_ARTIFACTS: &str =
    "mkdir -p target; printf one > target/first.bin; sleep 3; printf two > target/second.bin";

#[tokio::test]
async fn offload_job_after_a_restart_waits_for_its_running_command_before_collecting_or_releasing()
{
    let crashed = crashed_while_running(
        TWO_ARTIFACTS,
        vec![
            ("job_started", 200, super::cleanup_tests::machine("ready")),
            ("get", 200, super::cleanup_tests::machine("deleted")),
        ],
    )
    .await;
    crashed.restart();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let early = crashed.held.job_tool("chariox.jobs.status").await;
    assert_ne!(early["state"], "finished");
    assert_eq!(early["deleted"], false);
    assert!(
        !crashed
            .held
            .workspace
            .path()
            .join("target/first.bin")
            .exists(),
        "a file was accepted while its command still ran"
    );
    assert!(
        !crashed.service.is_finished(),
        "the worker was released while its command still ran"
    );
    let status = crashed.finished().await;
    crashed.service.await.unwrap();
    assert_eq!(status["deleted"], true);
    assert_eq!(
        status["result"]["outcome"], "completed",
        "{}",
        status["result"]["output_tail"]
    );
    assert_eq!(
        status["result"]["files_back"],
        serde_json::json!(["target/first.bin", "target/second.bin"])
    );
    for (name, body) in [("first", "one"), ("second", "two")] {
        assert_eq!(
            std::fs::read(
                crashed
                    .held
                    .workspace
                    .path()
                    .join(format!("target/{name}.bin"))
            )
            .unwrap(),
            body.as_bytes()
        );
    }
}
