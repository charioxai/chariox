//! A job placed on a machine the agent already holds, through `chariox.jobs.run`: the held machine's
//! own kernel runs it (a second kernel in this process stands in for it), and no machine is allocated,
//! started or released.
use super::cleanup_tests::service;
use super::return_tests::worker_kernel;
use super::*;

fn held_machine(state: &str, kernel_id: &str) -> serde_json::Value {
    serde_json::json!({"kind":"machine","revision":1,"available_actions":[],
        "allocation_ref":"held-1","state":state,
        "kernel_target":{"kernel_id":kernel_id,"machine_id":"held-machine","relay_realm_id":"realm","key_thumbprint":"a".repeat(64)},
        "access_state":"pending","access_reason":null,
        "machine_label":"held-box","address":"2001:db8::7"})
}

/// A kernel of its own that answers under `kernel_id`, as the held machine's kernel would.
fn stand_in_kernel(kernel_id: &str, home: &Path) {
    super::super::job_compute::stand_in_kernels()
        .lock()
        .unwrap()
        .insert(kernel_id.into(), worker_kernel(home));
}

/// The service ends once the kernel has made every call the test scripted for it.
async fn served(service: tokio::task::JoinHandle<()>) {
    tokio::time::timeout(Duration::from_secs(10), service)
        .await
        .expect("the kernel did not call the machine service as scripted")
        .unwrap();
}

async fn run_on_held(
    state: &KernelRuntimeState,
    run: &crate::provider::RuntimeProviderRun,
    task: &AgentTaskExecution,
    arguments: serde_json::Value,
) -> Result<serde_json::Value, DaemonError> {
    tokio::time::timeout(
        Duration::from_secs(60),
        state.dispatch_job_tool(Some(run), "chariox.jobs.run", &arguments, task),
    )
    .await
    .expect("chariox.jobs.run did not end")
}

#[tokio::test]
async fn a_job_naming_a_held_machine_runs_on_its_kernel_and_leaves_the_machine_held() {
    let (state, run, task, workspace) = super::tests::remote_fixture().await;
    let home = crate::test_support::TestWorktree::new("held-machine-home");
    stand_in_kernel("held-kernel-runs", home.path());
    // The machine is read to check it before the job exists and again when the job starts; any
    // other call (acquire, job started, release) finds no service and fails the job.
    let service = service(
        &state,
        vec![
            ("get", 200, held_machine("ready", "held-kernel-runs")),
            ("get", 200, held_machine("ready", "held-kernel-runs")),
        ],
    )
    .await;
    let value = run_on_held(
        &state,
        &run,
        &task,
        serde_json::json!({"command":["/bin/sh","-c","printf held > held.bin; pwd"],"outputs":["held.bin"],"machine":"held-1","wait_s":50}),
    )
    .await
    .unwrap();
    served(service).await;
    assert_eq!(value["placement"], "remote", "{value}");
    assert_eq!(value["machine"], "held-box");
    assert_eq!(value["result"]["outcome"], "completed", "{value}");
    assert_eq!(value["result"]["exit_code"], 0);
    assert_eq!(
        value["result"]["files_back"],
        serde_json::json!(["held.bin"])
    );
    assert_eq!(
        std::fs::read(workspace.path().join("held.bin")).unwrap(),
        b"held"
    );
    assert!(
        value["result"]["output_tail"]
            .as_str()
            .unwrap()
            .contains(home.path().to_str().unwrap()),
        "the command ran in the held machine's home: {value}"
    );
    // The machine is still held: the job released nothing and records no allocation of its own.
    assert_eq!(value["deleted"], false);
    let record = state
        .job_wake_driver
        .as_ref()
        .unwrap()
        .read(value["job_id"].as_str().unwrap())
        .unwrap();
    assert!(record.allocation_ref.is_none() && record.capability_revision.is_none());
    // The uploaded copy of the workspace does not stay on a machine the agent keeps.
    assert!(!home
        .path()
        .join("chariox-jobs")
        .join(&record.job_id)
        .exists());
}

#[tokio::test]
async fn a_job_naming_a_machine_that_is_not_held_is_refused_before_it_starts() {
    let (state, run, task, _workspace) = super::tests::remote_fixture().await;
    let service = service(
        &state,
        vec![(
            "get",
            404,
            serde_json::json!({"error":{"code":"not_found"}}),
        )],
    )
    .await;
    let error = run_on_held(
        &state,
        &run,
        &task,
        serde_json::json!({"command":["/bin/true"],"machine":"held-1","wait_s":5}),
    )
    .await
    .unwrap_err();
    served(service).await;
    assert!(
        error.to_string().contains("You hold no machine held-1")
            && error.to_string().contains("chariox.machines.acquire"),
        "{error}"
    );
}

#[tokio::test]
async fn a_job_naming_a_released_machine_is_refused_with_what_to_do() {
    let (state, run, task, _workspace) = super::tests::remote_fixture().await;
    let service = service(
        &state,
        vec![("get", 200, held_machine("deleted", "held-kernel-gone"))],
    )
    .await;
    let error = run_on_held(
        &state,
        &run,
        &task,
        serde_json::json!({"command":["/bin/true"],"machine":"held-1","wait_s":5}),
    )
    .await
    .unwrap_err();
    served(service).await;
    assert!(
        error.to_string().contains("is deleted") && error.to_string().contains("Acquire another"),
        "{error}"
    );
}

#[tokio::test]
async fn a_job_naming_a_machine_whose_kernel_does_not_answer_is_refused() {
    let (state, run, task, _workspace) = super::tests::remote_fixture().await;
    // No kernel stands in under this id, so the gateway cannot reach it.
    let service = service(
        &state,
        vec![("get", 200, held_machine("ready", "held-kernel-silent"))],
    )
    .await;
    let error = run_on_held(
        &state,
        &run,
        &task,
        serde_json::json!({"command":["/bin/true"],"machine":"held-1","wait_s":5}),
    )
    .await
    .unwrap_err();
    served(service).await;
    assert!(
        error.to_string().contains("did not answer")
            && error.to_string().contains("chariox.machines.status"),
        "{error}"
    );
}

/// Naming a machine the agent already holds is not an offload policy choice, so the kernel switch
/// for renting workers (CHARIOX_REMOTE_JOBS) does not gate it.
#[tokio::test]
async fn a_job_naming_a_held_machine_runs_with_remote_jobs_off() {
    let (state, run, task, _workspace) = super::tests::fixture().await;
    let home = crate::test_support::TestWorktree::new("held-machine-off-home");
    stand_in_kernel("held-kernel-off", home.path());
    let service = service(
        &state,
        vec![
            ("get", 200, held_machine("ready", "held-kernel-off")),
            ("get", 200, held_machine("ready", "held-kernel-off")),
        ],
    )
    .await;
    let value = run_on_held(
        &state,
        &run,
        &task,
        serde_json::json!({"command":["/bin/true"],"machine":"held-1","wait_s":50}),
    )
    .await
    .unwrap();
    served(service).await;
    assert_eq!(value["result"]["outcome"], "completed", "{value}");
}

#[test]
fn a_machine_name_that_is_not_an_allocation_ref_is_rejected() {
    for machine in ["", "two words", "line\nbreak", &"x".repeat(201)] {
        let request = serde_json::from_value::<RunJobRequest>(
            serde_json::json!({"command":["echo"],"machine":machine}),
        )
        .unwrap();
        assert!(validate(&request).is_err(), "{machine:?}");
    }
}

#[tokio::test]
async fn after_a_restart_the_held_machine_is_granted_again_only_while_it_runs_the_same_kernel() {
    let (state, _run, _task, workspace) = super::tests::remote_fixture().await;
    let home = crate::test_support::TestWorktree::new("held-machine-reattach-home");
    stand_in_kernel("held-kernel-same", home.path());
    let mut record = super::return_tests::remote_record(workspace.path(), "true", &[]);
    record.request.machine = Some("held-1".into());
    let service = service(
        &state,
        vec![
            ("get", 200, held_machine("ready", "held-kernel-same")),
            ("get", 200, held_machine("ready", "held-kernel-same")),
        ],
    )
    .await;
    assert!(state
        .held_machine_reattach(&record, "held-kernel-same")
        .await
        .is_ok());
    let error = state
        .held_machine_reattach(&record, "held-kernel-before-restart")
        .await
        .err()
        .expect("another kernel is refused");
    served(service).await;
    assert!(error.to_string().contains("another kernel"), "{error}");
}

/// What goes wrong on the way to a held machine, at the nth chunk of the upload or at the cleanup.
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    /// The nth upload chunk fails as a relay outage would.
    FailUpload(usize),
    /// Every reply from the nth upload chunk onward is lost.
    LoseUploadReplies(usize),
    /// The nth upload chunk never gets its reply: the home kernel dies waiting for it.
    HangUpload(usize),
    /// Every cleanup request fails.
    CleanupDown,
    /// The workspace removal runs on the machine but its reply never comes back.
    LoseCleanupReply,
    /// The machine's kernel refuses to open the removal command, once. Its gateway ledger keeps
    /// that answer under the command id.
    TargetRefusesRemoval,
}

/// The held machine's kernel with the faults of a real machine and relay.
struct Machine {
    kernel: Arc<super::tests::Worker>,
    /// The machine's gateway ledger: a state-changing request is answered from it by command id.
    ledger: crate::runtime::kernel_gateway::ledger::GatewayLedger,
    _ledger_dir: crate::test_support::TestWorktree,
    fault: std::sync::Mutex<Fault>,
    chunks: std::sync::atomic::AtomicUsize,
    /// Told when a fault was reached.
    reached: tokio::sync::Notify,
}

impl Machine {
    fn stand_in(kernel_id: &str, home: &Path, fault: Fault) -> Arc<Self> {
        let ledger_dir = crate::test_support::TestWorktree::new("held-machine-ledger");
        let machine = Arc::new(Self {
            kernel: worker_kernel(home),
            ledger: crate::runtime::kernel_gateway::ledger::GatewayLedger::open(
                ledger_dir.path().join("ledger.jsonl"),
            ),
            _ledger_dir: ledger_dir,
            fault: fault.into(),
            chunks: 0.into(),
            reached: Default::default(),
        });
        super::super::job_compute::stand_in_kernels()
            .lock()
            .unwrap()
            .insert(kernel_id.into(), machine.clone());
        machine
    }

    fn set(&self, fault: Fault) {
        *self.fault.lock().unwrap() = fault;
    }

    async fn reached(&self) {
        tokio::time::timeout(Duration::from_secs(30), self.reached.notified())
            .await
            .expect("the machine never reached the fault");
    }
}

impl Machine {
    /// What the machine's gateway does with a state-changing request: run it once under its
    /// command id and answer every later request with that id from the ledger.
    async fn through_ledger(
        &self,
        id: String,
        request: &LocalDaemonRequest,
        refuse: bool,
    ) -> Result<serde_json::Value, crate::runtime::kernel_gateway::GatewayError> {
        use crate::room_exec::gateway_placement::GrantedSend;
        use crate::runtime::kernel_gateway::{ledger::Failure, ledger::Settled, GatewayError};
        let command = crate::runtime::command::KernelCommand::from_local_request_with_caller(
            id.clone(),
            crate::runtime::command::KernelCommandSource::RelayClient,
            crate::runtime::command::KernelCaller::for_relay_request(None),
            None,
            None,
            request,
        );
        let fingerprint =
            crate::runtime_transport::command_cache::CommandFingerprint::for_app_control(
                &command, request,
            );
        let run = async {
            if refuse {
                self.set(Fault::None);
                return Err(Failure {
                    code: "open_refused".into(),
                    message: "the machine refused to open the command".into(),
                    retryable: false,
                });
            }
            self.kernel
                .send(id.clone(), request)
                .await
                .map_err(|error| Failure {
                    code: error.code().into(),
                    message: error.transport_error().message,
                    retryable: false,
                })
        };
        match self.ledger.run(&id, fingerprint, run).await {
            Settled::Done(Ok(value)) => Ok(value),
            Settled::Done(Err(failure)) => Err(GatewayError::Remote {
                name: "held".into(),
                code: failure.code,
                message: failure.message,
                retryable: failure.retryable,
            }),
            _ => Err(outage()),
        }
    }
}

fn outage() -> crate::runtime::kernel_gateway::GatewayError {
    crate::runtime::kernel_gateway::GatewayError::Remote {
        name: "held".into(),
        code: "relay_outage".into(),
        message: "relay outage".into(),
        retryable: false,
    }
}

impl crate::room_exec::gateway_placement::GrantedSend for Machine {
    fn send<'a>(
        &'a self,
        id: String,
        request: &'a LocalDaemonRequest,
    ) -> crate::room_exec::gateway_placement::SendFuture<'a> {
        use std::sync::atomic::Ordering::SeqCst;
        Box::pin(async move {
            let fault = *self.fault.lock().unwrap();
            if matches!(request, LocalDaemonRequest::WriteKernelFile(_)) {
                let chunk = self.chunks.fetch_add(1, SeqCst);
                match fault {
                    Fault::FailUpload(at) if at == chunk => return Err(outage()),
                    Fault::LoseUploadReplies(at) if chunk >= at => {
                        return Err(
                            crate::runtime::kernel_gateway::GatewayError::ConnectionLost {
                                name: "held worker".into(),
                            },
                        );
                    }
                    Fault::HangUpload(at) if at == chunk => {
                        self.reached.notify_one();
                        std::future::pending::<()>().await
                    }
                    _ => {}
                }
            }
            let cleanup = id.contains(":cleanup");
            if cleanup && fault == Fault::CleanupDown {
                return Err(outage());
            }
            let removal = matches!(request, LocalDaemonRequest::RoomTerminal(room)
                if matches!(&room.operation, crate::local::RoomTerminalOperation::ExecOpen { command } if command.first().map(String::as_str) == Some("/bin/rm")));
            let reply = if crate::runtime::kernel_gateway::ledger::is_repeatable(request) {
                self.kernel.send(id, request).await
            } else {
                self.through_ledger(id, request, removal && fault == Fault::TargetRefusesRemoval)
                    .await
            };
            if removal && fault == Fault::LoseCleanupReply {
                self.set(Fault::None);
                self.reached.notify_one();
                std::future::pending::<()>().await
            }
            reply
        })
    }
}

/// A workspace of three small files, so an upload has several chunks to be cut in.
fn three_files(workspace: &Path) {
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(workspace.join(name), name).unwrap();
    }
}

fn uploaded_copy(home: &Path, job_id: &str) -> PathBuf {
    home.join("chariox-jobs").join(job_id)
}

async fn job_status(
    state: &KernelRuntimeState,
    run: &crate::provider::RuntimeProviderRun,
    task: &AgentTaskExecution,
    name: &str,
    job_id: &str,
) -> serde_json::Value {
    state
        .dispatch_job_tool(Some(run), name, &serde_json::json!({"job_id":job_id}), task)
        .await
        .unwrap()
}

async fn finished(
    state: &KernelRuntimeState,
    run: &crate::provider::RuntimeProviderRun,
    task: &AgentTaskExecution,
    job_id: &str,
) -> serde_json::Value {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let status = job_status(state, run, task, "chariox.jobs.status", job_id).await;
            if status["state"] == "finished" {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("the job never finished")
}

fn saved(state: &KernelRuntimeState, job_id: &str) -> Record {
    state
        .job_wake_driver
        .as_ref()
        .unwrap()
        .read(job_id)
        .unwrap()
}

fn plan_saved(state: &KernelRuntimeState, job_id: &str) -> bool {
    super::return_tests::job_state_path(state, &format!("{job_id}/return.json")).exists()
}

/// The home kernel restarted: its grants are gone, the saved records are what it recovers from.
fn restart(state: &KernelRuntimeState, job_id: &str, recover_wake: bool) {
    state
        .job_wake_driver
        .as_ref()
        .unwrap()
        .return_grants
        .lock()
        .unwrap()
        .remove(job_id);
    if recover_wake {
        super::return_tests::recover_job(state, job_id);
    }
}

fn two_gets(kernel: &str) -> Vec<(&'static str, u16, serde_json::Value)> {
    vec![
        ("get", 200, held_machine("ready", kernel)),
        ("get", 200, held_machine("ready", kernel)),
    ]
}

#[tokio::test]
async fn cancelling_a_held_machine_job_during_its_upload_removes_what_was_uploaded() {
    let (state, run, task, workspace) = super::tests::remote_fixture().await;
    three_files(workspace.path());
    let home = crate::test_support::TestWorktree::new("held-cancel-upload-home");
    let machine = Machine::stand_in("held-kernel-cancel", home.path(), Fault::HangUpload(1));
    let service = service(&state, two_gets("held-kernel-cancel")).await;
    let started = run_on_held(
        &state,
        &run,
        &task,
        serde_json::json!({"command":["/bin/true"],"machine":"held-1","wait_s":1}),
    )
    .await
    .unwrap();
    let id = started["job_id"].as_str().unwrap().to_owned();
    machine.reached().await;
    assert!(
        uploaded_copy(home.path(), &id).exists(),
        "the first file was uploaded"
    );
    assert!(
        plan_saved(&state, &id),
        "where the upload goes is saved before it starts"
    );
    assert_eq!(saved(&state, &id).remote, RemoteStage::Planned);
    tokio::time::timeout(
        Duration::from_secs(10),
        job_status(&state, &run, &task, "chariox.jobs.cancel", &id),
    )
    .await
    .expect("a stalled upload must respond to cancellation");
    let status = finished(&state, &run, &task, &id).await;
    served(service).await;
    assert_eq!(status["result"]["outcome"], "cancelled", "{status}");
    assert!(
        !uploaded_copy(home.path(), &id).exists(),
        "the cancelled upload stays on the held machine"
    );
    assert!(!plan_saved(&state, &id));
    assert_eq!(status["deleted"], false);
    assert_eq!(saved(&state, &id).remote, RemoteStage::Settled);
    let output = status["result"]["output_tail"].as_str().unwrap();
    assert!(
        !output.contains("may still run"),
        "no command was opened: {output}"
    );
}

#[tokio::test]
async fn a_held_machine_job_whose_upload_fails_removes_what_was_uploaded() {
    let (state, run, task, workspace) = super::tests::remote_fixture().await;
    three_files(workspace.path());
    let home = crate::test_support::TestWorktree::new("held-failed-upload-home");
    Machine::stand_in("held-kernel-upload", home.path(), Fault::FailUpload(1));
    let service = service(&state, two_gets("held-kernel-upload")).await;
    let value = run_on_held(
        &state,
        &run,
        &task,
        serde_json::json!({"command":["/bin/true"],"machine":"held-1","wait_s":50}),
    )
    .await
    .unwrap();
    served(service).await;
    let id = value["job_id"].as_str().unwrap();
    assert_ne!(value["result"]["outcome"], "completed", "{value}");
    assert!(
        !uploaded_copy(home.path(), id).exists(),
        "the failed upload stays on the held machine"
    );
    assert!(!plan_saved(&state, id));
    assert_eq!(value["deleted"], false);
}

#[tokio::test]
async fn a_kernel_restart_during_a_held_machine_upload_removes_what_was_uploaded() {
    // Remote jobs are off: the held machine's job is recovered anyway.
    let (state, run, task, workspace) = super::tests::fixture().await;
    three_files(workspace.path());
    let home = crate::test_support::TestWorktree::new("held-restart-upload-home");
    let machine = Machine::stand_in("held-kernel-restart-up", home.path(), Fault::HangUpload(1));
    let service = service(
        &state,
        vec![
            ("get", 200, held_machine("ready", "held-kernel-restart-up")),
            ("get", 200, held_machine("ready", "held-kernel-restart-up")),
            ("get", 200, held_machine("ready", "held-kernel-restart-up")),
        ],
    )
    .await;
    let started = run_on_held(
        &state,
        &run,
        &task,
        serde_json::json!({"command":["/bin/true"],"machine":"held-1","wait_s":1}),
    )
    .await
    .unwrap();
    let id = started["job_id"].as_str().unwrap().to_owned();
    machine.reached().await;
    assert!(uploaded_copy(home.path(), &id).exists());
    assert!(plan_saved(&state, &id));
    machine.set(Fault::None);
    restart(&state, &id, true);
    let status = finished(&state, &run, &task, &id).await;
    served(service).await;
    assert!(
        !uploaded_copy(home.path(), &id).exists(),
        "recovery left the half-uploaded workspace on the held machine"
    );
    assert!(!plan_saved(&state, &id));
    assert_eq!(status["deleted"], false);
    assert_eq!(saved(&state, &id).remote, RemoteStage::Settled);
}

#[tokio::test]
async fn a_kernel_restart_while_a_held_machine_cleans_up_keeps_the_finished_result() {
    let (state, run, task, _workspace) = super::tests::remote_fixture().await;
    let home = crate::test_support::TestWorktree::new("held-restart-clean-home");
    let machine = Machine::stand_in(
        "held-kernel-restart-clean",
        home.path(),
        Fault::LoseCleanupReply,
    );
    let service = service(
        &state,
        vec![
            (
                "get",
                200,
                held_machine("ready", "held-kernel-restart-clean"),
            ),
            (
                "get",
                200,
                held_machine("ready", "held-kernel-restart-clean"),
            ),
            (
                "get",
                200,
                held_machine("ready", "held-kernel-restart-clean"),
            ),
        ],
    )
    .await;
    let started = run_on_held(
        &state,
        &run,
        &task,
        serde_json::json!({"command":["/bin/sh","-c","printf held > held.bin"],"outputs":["held.bin"],"machine":"held-1","wait_s":1}),
    )
    .await
    .unwrap();
    let id = started["job_id"].as_str().unwrap().to_owned();
    machine.reached().await;
    // The home kernel dies here, waiting for the machine to answer the removal. What it saved
    // must already say the files are home and carry the whole result.
    let record = saved(&state, &id);
    assert_eq!(
        record.remote,
        RemoteStage::Settled,
        "files are home: {}",
        record.state
    );
    let result = record
        .result
        .expect("the result is saved before the workspace goes");
    assert_eq!(result.files_back, ["held.bin"]);
    assert_eq!(result.outcome, JobOutcome::Completed);
    restart(&state, &id, true);
    let status = finished(&state, &run, &task, &id).await;
    served(service).await;
    assert_eq!(status["result"]["outcome"], "completed", "{status}");
    assert_eq!(
        status["result"]["files_back"],
        serde_json::json!(["held.bin"])
    );
    assert_eq!(status["return_pending"], false);
    assert!(!uploaded_copy(home.path(), &id).exists());
    assert!(!plan_saved(&state, &id));
}

#[tokio::test]
async fn a_held_machine_that_could_not_clean_up_is_cleaned_when_the_job_is_checked_again() {
    let (state, run, task, _workspace) = super::tests::remote_fixture().await;
    let home = crate::test_support::TestWorktree::new("held-retry-clean-home");
    let machine = Machine::stand_in("held-kernel-retry", home.path(), Fault::CleanupDown);
    let service = service(&state, two_gets("held-kernel-retry")).await;
    let value = run_on_held(
        &state,
        &run,
        &task,
        serde_json::json!({"command":["/bin/true"],"machine":"held-1","wait_s":50}),
    )
    .await
    .unwrap();
    served(service).await;
    let id = value["job_id"].as_str().unwrap();
    assert_eq!(value["result"]["outcome"], "completed", "{value}");
    assert!(uploaded_copy(home.path(), id).exists(), "cleanup was down");
    assert!(
        plan_saved(&state, id),
        "the plan names what is left to remove"
    );
    machine.set(Fault::None);
    let status = job_status(&state, &run, &task, "chariox.jobs.status", id).await;
    assert_eq!(status["result"]["outcome"], "completed");
    assert!(!uploaded_copy(home.path(), id).exists());
    assert!(!plan_saved(&state, id));
}

#[tokio::test]
async fn a_removal_the_machine_refused_runs_again_when_the_job_is_checked_again() {
    let (state, run, task, _workspace) = super::tests::remote_fixture().await;
    let home = crate::test_support::TestWorktree::new("held-refused-clean-home");
    Machine::stand_in(
        "held-kernel-refused",
        home.path(),
        Fault::TargetRefusesRemoval,
    );
    let service = service(&state, two_gets("held-kernel-refused")).await;
    let value = run_on_held(
        &state,
        &run,
        &task,
        serde_json::json!({"command":["/bin/true"],"machine":"held-1","wait_s":50}),
    )
    .await
    .unwrap();
    served(service).await;
    let id = value["job_id"].as_str().unwrap();
    assert_eq!(value["result"]["outcome"], "completed", "{value}");
    assert!(
        uploaded_copy(home.path(), id).exists(),
        "the machine refused the removal"
    );
    assert!(plan_saved(&state, id));
    // The machine's gateway keeps its answer to the first removal under that attempt's command
    // ids, so only a retry with ids of its own runs.
    let status = job_status(&state, &run, &task, "chariox.jobs.status", id).await;
    assert_eq!(status["result"]["outcome"], "completed");
    assert!(
        !uploaded_copy(home.path(), id).exists(),
        "the retry was answered with the first attempt's failure"
    );
    assert!(!plan_saved(&state, id));
}

#[tokio::test]
async fn a_kernel_restart_removes_what_a_finished_held_machine_job_could_not() {
    let (state, run, task, _workspace) = super::tests::remote_fixture().await;
    let home = crate::test_support::TestWorktree::new("held-startup-clean-home");
    let machine = Machine::stand_in("held-kernel-startup", home.path(), Fault::CleanupDown);
    let service = service(
        &state,
        vec![
            ("get", 200, held_machine("ready", "held-kernel-startup")),
            ("get", 200, held_machine("ready", "held-kernel-startup")),
            ("get", 200, held_machine("ready", "held-kernel-startup")),
        ],
    )
    .await;
    let value = run_on_held(
        &state,
        &run,
        &task,
        serde_json::json!({"command":["/bin/true"],"machine":"held-1","wait_s":50}),
    )
    .await
    .unwrap();
    let id = value["job_id"].as_str().unwrap();
    assert!(uploaded_copy(home.path(), id).exists(), "cleanup was down");
    assert!(plan_saved(&state, id));
    // The completion was delivered and the wake ended, so recovery of active wakes will not see it.
    let wake = state
        .owned
        .durable_state_store
        .agent_wake(&saved(&state, id).wake_id)
        .unwrap()
        .unwrap();
    assert!(
        !matches!(wake.state.as_str(), "starting" | "running" | "cancelling"),
        "the wake is {}",
        wake.state
    );
    machine.set(Fault::None);
    let id = id.to_owned();
    restart(&state, &id, false);
    state
        .recover_driver_wakes(crate::session::unix_epoch_ms())
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        while uploaded_copy(home.path(), &id).exists() || plan_saved(&state, &id) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the restart left the workspace on the held machine");
    served(service).await;
}

async fn job_with_upload_fault(kernel_id: &str, fault: Fault) -> (serde_json::Value, usize) {
    let (state, run, task, workspace) = super::tests::remote_fixture().await;
    three_files(workspace.path());
    let home = crate::test_support::TestWorktree::new("held-upload-fault");
    let machine = Machine::stand_in(kernel_id, home.path(), fault);
    let service = service(&state, two_gets(kernel_id)).await;
    let job = run_on_held(
        &state,
        &run,
        &task,
        serde_json::json!({"command":["/bin/true"],"machine":"held-1","timeout_s":1,"wait_s":6}),
    )
    .await
    .unwrap();
    served(service).await;
    let writes = machine.chunks.load(std::sync::atomic::Ordering::SeqCst);
    (job, writes)
}

#[tokio::test]
async fn a_stalled_held_upload_finishes_with_a_time_limit_and_a_next_step() {
    let (job, writes) =
        job_with_upload_fault("held-kernel-stalled-upload", Fault::HangUpload(1)).await;
    assert_eq!(writes, 2);
    assert_eq!(job["state"], "finished", "{job}");
    assert_eq!(job["result"]["outcome"], "time_limit", "{job}");
    assert!(job["result"]["exit_code"].is_null());
    let output = job["result"]["output_tail"].as_str().unwrap();
    assert!(
        output.contains("uploading the workspace") && output.contains("chariox.machines.status"),
        "{output}"
    );
}

#[tokio::test]
async fn a_missing_held_upload_reply_is_retried_and_reported() {
    let (job, writes) =
        job_with_upload_fault("held-kernel-missing-reply", Fault::LoseUploadReplies(1)).await;
    assert_eq!(
        writes, 3,
        "the second chunk was retried after its reply was lost"
    );
    assert_eq!(job["state"], "finished", "{job}");
    assert!(job["result"]["exit_code"].is_null());
    let output = job["result"]["output_tail"].as_str().unwrap();
    assert!(
        output.contains("reply could not be recovered")
            && output.contains("before the command started")
            && output.contains("chariox.machines.status"),
        "{output}"
    );
}
