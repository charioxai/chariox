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
        return_pending: false,
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
    let base = crate::jobs::snapshot(source, &record.request.outputs).unwrap();
    let upload = crate::jobs::snapshot(source, &[]).unwrap();
    let (_cancel, cancelled) = watch::channel(false);
    let (remote, root) = remote::Remote::prepare(worker, &record, &upload, &cancelled)
        .await
        .unwrap();
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

/// Fails one artifact read as a relay outage would, until it is disarmed.
struct Outage {
    worker: Arc<Worker>,
    path: &'static str,
    cut: std::sync::atomic::AtomicBool,
}
impl GrantedSend for Outage {
    fn send<'a>(
        &'a self,
        id: String,
        request: &'a LocalDaemonRequest,
    ) -> crate::room_exec::gateway_placement::SendFuture<'a> {
        if self.cut.load(std::sync::atomic::Ordering::SeqCst)
            && matches!(request, LocalDaemonRequest::ReadKernelFile(read) if read.path.ends_with(self.path))
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

struct HeldWorker {
    state: KernelRuntimeState,
    run: crate::provider::RuntimeProviderRun,
    task: AgentTaskExecution,
    workspace: crate::test_support::TestWorktree,
    _home: crate::test_support::TestWorktree,
    outage: Arc<Outage>,
    job_id: String,
}

/// A command that made two artifacts finished on the worker while the read of the second failed.
async fn worker_held_after_failed_return() -> HeldWorker {
    let (state, run, task, workspace) = super::tests::fixture().await;
    let home = crate::test_support::TestWorktree::new("offload-outage-home");
    let outage = Arc::new(Outage {
        worker: worker_kernel(home.path()),
        path: "target/second.bin",
        cut: true.into(),
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
    record.request = remote_record(
        workspace.path(),
        "mkdir -p target; printf one > target/first.bin; printf two > target/second.bin",
        &["target/*.bin"],
    )
    .request;
    record.placement = JobPlacement::Remote;
    record.state = "uploading".into();
    record.result = None;
    record.allocation_ref = Some("cleanup-test".into());
    record.capability_revision = Some("v1".into());
    record.deleted = false;
    let machine = super::super::job_compute::JobMachine {
        allocation_ref: "cleanup-test".into(),
        machine_label: "offload-test".into(),
        address: None,
        capability_revision: "v1".into(),
        kernel_id: "worker".into(),
        grant: outage.clone(),
        class_id: "test-class".into(),
        rate_per_minute: crate::local::JobMachineRate {
            amount_micros: 1,
            currency: "usd".into(),
        },
    };
    let mut output = super::execute::Output::new(&state, &record).unwrap();
    let (_cancel, mut cancelled) = watch::channel(false);
    // The worker is told its job started; any later request is the release.
    let release = super::cleanup_tests::service(
        &state,
        vec![
            ("job_started", 200, super::cleanup_tests::machine("ready")),
            ("get", 200, super::cleanup_tests::machine("deleted")),
        ],
    )
    .await;
    let result = state
        .run_remote_job(
            &mut record,
            &machine,
            &BTreeMap::new(),
            &mut output,
            &mut cancelled,
        )
        .await;
    state
        .complete_remote_job(&mut record, &mut output, result, false)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !release.is_finished(),
        "the worker was released while a file it produced had not come home"
    );
    release.abort();
    record.state = "finished".into();
    driver.save(&record).unwrap();
    let saved = driver.read(&record.job_id).unwrap();
    assert!(saved.return_pending && !saved.deleted);
    assert!(saved
        .result
        .unwrap()
        .output_tail
        .contains("files could not all be returned"));
    assert_eq!(
        std::fs::read(workspace.path().join("target/first.bin")).unwrap(),
        b"one"
    );
    assert!(!workspace.path().join("target/second.bin").exists());
    HeldWorker {
        state,
        run,
        task,
        workspace,
        _home: home,
        outage,
        job_id: record.job_id,
    }
}

async fn release_service(state: &KernelRuntimeState) -> tokio::task::JoinHandle<()> {
    super::cleanup_tests::service(
        state,
        vec![("get", 200, super::cleanup_tests::machine("deleted"))],
    )
    .await
}

impl HeldWorker {
    async fn job_tool(&self, name: &str) -> serde_json::Value {
        self.state
            .dispatch_job_tool(
                Some(&self.run),
                name,
                &serde_json::json!({"job_id":self.job_id}),
                &self.task,
            )
            .await
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
