use super::*;

pub(super) struct TempDir(PathBuf);
impl TempDir {
    pub(super) fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chariox-exec-queue-{:032x}",
            rand::random::<u128>()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(super) fn limited(memory: u64, disk: u64) -> RoomExecService {
    RoomExecService {
        inner: Arc::new(Inner {
            admission: LaunchAdmission::limited(memory, disk),
            ..Inner::default()
        }),
    }
}

pub(super) async fn queued(service: &RoomExecService, id: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !service
            .list("room")
            .iter()
            .any(|r| r.exec_id == id && matches!(r.state, ExecState::Queued { .. }))
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn queue_has_no_side_effect_and_cancel_never_spawns() {
    let directory = TempDir::new();
    let marker = directory.path().join("marker");
    let service = limited(0, u64::MAX / 2);
    let id = tests::open(
        &service,
        &format!("printf executed > '{}'", marker.display()),
    );
    queued(&service, &id).await;
    assert!(!marker.exists(), "held intent has no user process");
    assert!(
        service.has_running_work(),
        "a queue cannot be reported idle"
    );
    service.cancel("room", &id).unwrap();
    let (_, _, result) = tests::collect(&service, &id).await;
    assert!(matches!(result, ExecOutcome::Failed { .. }));
    assert!(!marker.exists(), "cancelled before claim");
    assert!(!service.has_running_work());
}

#[tokio::test]
async fn start_now_keeps_binary_input_offsets_streams_and_one_execution() {
    let directory = TempDir::new();
    let marker = directory.path().join("marker");
    // Only memory holds a command whose output volume no completed run has shown.
    let service = limited(0, u64::MAX / 2);
    let id = tests::open(
        &service,
        &format!(
            "printf x >> '{}'; cat; printf '\\377\\000err' >&2; exit 23",
            marker.display()
        ),
    );
    queued(&service, &id).await;
    let bytes: Vec<u8> = (0..=255).cycle().take(CHUNK_BYTES + 17).collect();
    for (index, chunk) in bytes.chunks(CHUNK_BYTES).enumerate() {
        let offset = (index * CHUNK_BYTES) as u64;
        let encoded = base64::engine::general_purpose::STANDARD.encode(chunk);
        assert_eq!(
            service.write("room", &id, offset, &encoded).unwrap(),
            offset + chunk.len() as u64
        );
        assert_eq!(
            service.write("room", &id, offset, &encoded).unwrap(),
            offset + chunk.len() as u64
        );
    }
    service
        .close_stdin("room", &id, bytes.len() as u64)
        .unwrap();
    service.start_now("room", &id).unwrap();
    let (stdout, stderr, result) = tests::collect(&service, &id).await;
    assert_eq!(stdout, bytes);
    assert_eq!(stderr, b"\xff\0err");
    assert_eq!(result, ExecOutcome::Exited { code: 23 });
    assert_eq!(std::fs::read(marker).unwrap(), b"x");
    assert!(service.start_now("room", &id).is_err());
}

#[tokio::test]
async fn auto_resume_reserves_capacity_before_the_next_command() {
    let service = limited(32 * 1024 * 1024, u64::MAX / 2);
    let first = tests::open(&service, "cat");
    tokio::time::timeout(Duration::from_secs(5), async {
        while !service
            .list("room")
            .iter()
            .any(|r| r.exec_id == first && r.state == ExecState::Running)
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let second = tests::open(&service, "printf second");
    queued(&service, &second).await;
    service.close_stdin("room", &first, 0).unwrap();
    assert_eq!(
        tests::collect(&service, &first).await.2,
        ExecOutcome::Exited { code: 0 }
    );
    let (out, _, result) = tests::collect(&service, &second).await;
    assert_eq!(out, b"second");
    assert_eq!(result, ExecOutcome::Exited { code: 0 });
}

#[test]
fn concurrent_binary_admissions_cannot_overbook() {
    let admission = LaunchAdmission::limited(32 * 1024 * 1024, u64::MAX / 2);
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let handles = (0..2)
        .map(|index| {
            let admission = admission.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let permit =
                    admission.reserve(&index.to_string(), &["cat".into()], Path::new("/"), false);
                barrier.wait();
                permit
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
}
