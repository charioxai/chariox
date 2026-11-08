use super::placement::{ExecPlacement, PlacementFuture};
use super::queue_tests::{limited, queued, TempDir};
use super::*;
use crate::runtime::process_resources::SessionProcessOwner;

/// A second local kernel: its own Room exec service, pool and directory.
struct SecondKernel {
    service: RoomExecService,
    cwd: PathBuf,
    refuse: bool,
}

impl ExecPlacement for SecondKernel {
    fn open<'a>(
        &'a self,
        intent: &'a LaunchIntent,
        attributed_agent: Option<String>,
    ) -> PlacementFuture<'a, String> {
        Box::pin(async move {
            if self.refuse {
                return Err(error(
                    "exec_target_refused",
                    "target kernel refused the launch",
                ));
            }
            self.service.open_as(
                "room",
                crate::session::DEFAULT_LOCAL_USER_ID,
                &self.cwd,
                &intent.command,
                None,
                attributed_agent,
            )
        })
    }
    fn write<'a>(&'a self, id: &'a str, offset: u64, data: &'a str) -> PlacementFuture<'a, u64> {
        Box::pin(async move { self.service.write("room", id, offset, data) })
    }
    fn close_stdin<'a>(&'a self, id: &'a str, offset: u64) -> PlacementFuture<'a, u64> {
        Box::pin(async move { self.service.close_stdin("room", id, offset) })
    }
    fn read<'a>(
        &'a self,
        id: &'a str,
        after_cursor: u64,
        wait: Duration,
    ) -> PlacementFuture<'a, ExecOutput> {
        Box::pin(async move { self.service.read("room", id, after_cursor, wait).await })
    }
    fn cancel<'a>(&'a self, id: &'a str) -> PlacementFuture<'a, ()> {
        Box::pin(async move { self.service.cancel("room", id) })
    }
}

fn second_kernel(cwd: &Path, refuse: bool) -> (RoomExecService, Arc<SecondKernel>) {
    let service = RoomExecService::default();
    let kernel = Arc::new(SecondKernel {
        service: service.clone(),
        cwd: cwd.to_path_buf(),
        refuse,
    });
    (service, kernel)
}

fn open_held(service: &RoomExecService, cwd: &Path, script: &str, agent: Option<&str>) -> String {
    service
        .open_as(
            "room",
            crate::session::DEFAULT_LOCAL_USER_ID,
            cwd,
            &["/bin/bash".into(), "-c".into(), script.into()],
            None,
            agent.map(str::to_owned),
        )
        .unwrap()
}

#[tokio::test]
async fn held_launch_runs_on_the_second_kernel_with_the_same_io_and_its_agent() {
    let home = TempDir::new();
    let away = TempDir::new();
    let (second, target) = second_kernel(away.path(), false);
    let first = limited(0, u64::MAX / 2);
    let id = open_held(
        &first,
        home.path(),
        "pwd; cat; printf '\\377err' >&2; exit 7",
        Some("agent-1"),
    );
    queued(&first, &id).await;

    first.place("room", &id, "kernel-a", target).unwrap();
    let input = b"\x00bytes\xff";
    let encoded = base64::engine::general_purpose::STANDARD.encode(input);
    assert_eq!(
        first.write("room", &id, 0, &encoded).unwrap(),
        input.len() as u64
    );
    first.close_stdin("room", &id, input.len() as u64).unwrap();
    let (stdout, stderr, outcome) = super::tests::collect(&first, &id).await;

    let mut expected = format!("{}\n", away.path().canonicalize().unwrap().display()).into_bytes();
    expected.extend_from_slice(input);
    assert_eq!(
        stdout, expected,
        "ran in the second kernel's directory with the original stdin"
    );
    assert_eq!(stderr, b"\xfferr");
    assert_eq!(outcome, ExecOutcome::Exited { code: 7 });
    assert!(!first.has_running_work());
    let remote = second.list("room");
    assert_eq!(remote.len(), 1);
    assert_eq!(
        remote[0].owner,
        SessionProcessOwner::Agent("kernel-a/agent-1".into()),
        "the second kernel shows the owning agent of the first"
    );
    assert_eq!(
        first.list("room")[0].owner,
        SessionProcessOwner::Agent("agent-1".into())
    );
}

#[tokio::test]
async fn a_started_process_cannot_be_placed_elsewhere() {
    let away = TempDir::new();
    let (second, target) = second_kernel(away.path(), false);
    let first = RoomExecService::default();
    let id = super::tests::open(&first, "cat");
    tokio::time::timeout(Duration::from_secs(5), async {
        while first.list("room")[0].state != ExecState::Running {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let refused = first.place("room", &id, "kernel-a", target).unwrap_err();
    assert!(
        matches!(&refused, DaemonError::MachineRequest { code, .. } if code == "exec_not_queued"),
        "{refused}"
    );
    assert!(second.list("room").is_empty());
    first.close_stdin("room", &id, 0).unwrap();
    assert_eq!(
        super::tests::collect(&first, &id).await.2,
        ExecOutcome::Exited { code: 0 }
    );
}

#[tokio::test]
async fn cancelling_a_placed_launch_cancels_the_second_kernel_once() {
    let away = TempDir::new();
    let marker = away.path().join("marker");
    let (second, target) = second_kernel(away.path(), false);
    let first = limited(0, u64::MAX / 2);
    let id = open_held(
        &first,
        Path::new("/"),
        &format!("sleep 30; printf late > '{}'", marker.display()),
        None,
    );
    queued(&first, &id).await;
    first.place("room", &id, "kernel-a", target).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while second.list("room").is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    first.cancel("room", &id).unwrap();
    let (_, _, outcome) = super::tests::collect(&first, &id).await;
    assert!(matches!(outcome, ExecOutcome::Failed { .. }), "{outcome:?}");
    let remote = second.list("room");
    assert_eq!(remote.len(), 1, "one execution on the second kernel");
    let (_, _, remote_outcome) = super::tests::collect(&second, &remote[0].exec_id).await;
    assert!(matches!(remote_outcome, ExecOutcome::Failed { .. }));
    assert!(!marker.exists());
    assert!(!first.has_running_work());
}

#[tokio::test]
async fn a_refusing_target_ends_the_launch_without_running_it_at_home() {
    let home = TempDir::new();
    let away = TempDir::new();
    let marker = home.path().join("marker");
    let (second, target) = second_kernel(away.path(), true);
    let first = limited(0, u64::MAX / 2);
    let id = open_held(
        &first,
        home.path(),
        &format!("printf home > '{}'", marker.display()),
        None,
    );
    queued(&first, &id).await;
    first.place("room", &id, "kernel-a", target).unwrap();
    let (_, _, outcome) = super::tests::collect(&first, &id).await;
    match outcome {
        ExecOutcome::Failed { message } => {
            assert!(message.contains("target kernel refused"), "{message}")
        }
        other => panic!("{other:?}"),
    }
    assert!(second.list("room").is_empty());
    assert!(
        first.start_now("room", &id).is_err(),
        "a failed placement is final"
    );
    assert!(!marker.exists(), "the held command is not started at home");
}
