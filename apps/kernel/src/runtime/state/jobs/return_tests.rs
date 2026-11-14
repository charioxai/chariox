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
