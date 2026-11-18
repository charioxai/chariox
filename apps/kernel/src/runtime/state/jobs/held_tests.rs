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

#[tokio::test]
async fn a_job_naming_a_machine_with_remote_jobs_off_is_refused_without_calling_the_service() {
    let (state, run, task, _workspace) = super::tests::fixture().await;
    let service = service(&state, vec![("get", 200, serde_json::Value::Null)]).await;
    let error = run_on_held(
        &state,
        &run,
        &task,
        serde_json::json!({"command":["/bin/true"],"machine":"held-1","wait_s":5}),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("switched off"), "{error}");
    assert!(!service.is_finished(), "the machine service was called");
    service.abort();
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
