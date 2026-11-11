//! MP-08 / MP-09 / MP-10 / MP-11: `chariox.file.copy` between a home workspace and a leased
//! agent's workspace behind a fake worker that serves the real slice functions, drops the link
//! and loses answers on cue, and lets a test edit the source mid-copy.
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use super::workspace_slices::SLICE_BYTES;
use super::*;
use crate::runtime::kernel_files::StagedWrite;

/// What happens to one request on its way to the worker.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fault {
    None,
    /// The link drops before the worker sees the request.
    Dropped,
    /// The worker serves the request and the answer is lost on the way back.
    AnswerLost,
}

struct FakeWorker {
    workspace: Workspace,
    /// Decides the fault for the n-th request (from 0) and may touch the files first.
    cue: Mutex<Box<dyn FnMut(usize) -> Fault + Send>>,
    served: Mutex<usize>,
}

impl FakeWorker {
    fn new(workspace: Workspace) -> Self {
        Self::cued(workspace, |_| Fault::None)
    }

    fn cued(workspace: Workspace, cue: impl FnMut(usize) -> Fault + Send + 'static) -> Self {
        Self {
            workspace,
            cue: Mutex::new(Box::new(cue)),
            served: Mutex::new(0),
        }
    }

    fn deliver<T>(&self, serve: impl FnOnce() -> Result<T, String>) -> Result<T, LinkError> {
        let fault = {
            let mut served = self.served.lock().unwrap();
            let fault = (self.cue.lock().unwrap())(*served);
            *served += 1;
            fault
        };
        if fault == Fault::Dropped {
            return Err(LinkError::Offline("relay connection closed".into()));
        }
        let answer = serve().map_err(LinkError::Refused);
        if fault == Fault::AnswerLost {
            return Err(LinkError::Offline("response timed out".into()));
        }
        answer
    }
}

impl RemoteSource for FakeWorker {
    async fn read(&self, path: &str, offset: u64) -> Result<SourceSlice, LinkError> {
        self.deliver(|| read_slice(&self.workspace, path, offset))
    }
}

impl RemoteWorkspace for FakeWorker {
    async fn write(&self, path: &str, slice: &WriteSlice<'_>) -> Result<StagedWrite, LinkError> {
        self.deliver(|| write_slice(&self.workspace, path, slice))
    }

    async fn discard(&self, path: &str) -> Result<(), LinkError> {
        self.deliver(|| {
            discard(&self.workspace, path);
            Ok(())
        })
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "agent-file-copy-{name}-{:08x}",
        rand::random::<u32>()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn workspaces() -> (Workspace, Workspace) {
    let home = Workspace {
        root: scratch("home"),
        read_only: false,
    };
    let worker = Workspace {
        root: scratch("worker"),
        read_only: false,
    };
    (home, worker)
}

/// A file of `size` bytes no two slices of which are alike.
fn plant(root: &Path, relative: &str, size: usize) -> String {
    let bytes: Vec<u8> = (0..size).map(|i| (i * 31 + i / 4099) as u8).collect();
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    format!("{:x}", Sha256::digest(&bytes))
}

fn digest_of(path: &Path) -> String {
    format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
}

/// A copy staging file beside `relative`.
fn staging_left(root: &Path, relative: &str) -> bool {
    let path = root.join(relative);
    std::fs::read_dir(path.parent().unwrap()).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".chariox-copy-")
        })
    })
}

/// Nothing at `relative` and no copy staging beside it.
fn left_nothing(root: &Path, relative: &str) -> bool {
    !root.join(relative).exists() && !staging_left(root, relative)
}

const BUNDLE: usize = 3 * SLICE_BYTES as usize + 12_345;

#[tokio::test]
async fn a_file_crosses_both_ways_byte_for_byte_with_its_digest() {
    let (home, worker) = workspaces();
    let sha256 = plant(&worker.root, "out/x.bundle", BUNDLE);
    let remote = FakeWorker::new(worker.clone());
    let pulled = pull(
        &remote,
        "out/x.bundle",
        &home,
        "inbox/x.bundle",
        &CopyOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        (pulled.bytes, pulled.sha256.as_str()),
        (BUNDLE as u64, sha256.as_str())
    );
    assert_eq!(digest_of(&home.root.join("inbox/x.bundle")), sha256);

    let pushed = push(
        &home,
        "inbox/x.bundle",
        &remote,
        "in/base.bundle",
        &CopyOptions {
            overwrite: false,
            expect_sha256: Some(sha256.to_uppercase()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(pushed.sha256, sha256);
    assert_eq!(digest_of(&worker.root.join("in/base.bundle")), sha256);
    assert!(!staging_left(&worker.root, "in/base.bundle"));

    std::fs::write(home.root.join("empty"), b"").unwrap();
    let empty = push(&home, "empty", &remote, "empty", &CopyOptions::default())
        .await
        .unwrap();
    assert_eq!(empty.bytes, 0);
    assert_eq!(std::fs::read(worker.root.join("empty")).unwrap(), b"");
}

#[tokio::test]
async fn paths_that_leave_the_workspace_or_pass_through_links_are_refused() {
    let (home, worker) = workspaces();
    plant(&worker.root, "real/file", 10);
    let outside = scratch("outside");
    plant(&outside, "secret", 10);
    std::os::unix::fs::symlink(&outside, worker.root.join("linked")).unwrap();
    std::os::unix::fs::symlink(worker.root.join("real/file"), worker.root.join("alias")).unwrap();
    let remote = FakeWorker::new(worker.clone());
    let options = CopyOptions::default();
    for (source, expected) in [
        ("/etc/passwd", "is absolute"),
        ("../outside/secret", "leads out of the agent's workspace"),
        ("real/../../x", "leads out of the agent's workspace"),
        ("linked/secret", "passes through a link"),
        ("alias", "is a link"),
        ("real", "is a folder"),
        ("missing", "does not exist"),
    ] {
        let error = pull(&remote, source, &home, "got", &options)
            .await
            .unwrap_err();
        assert!(error.contains(expected), "{source}: {error}");
    }
    plant(&home.root, "payload", 10);
    for (destination, expected) in [
        ("/tmp/x", "is absolute"),
        ("../x", "leads out of the agent's workspace"),
        ("linked/planted", "passes through a link"),
        ("alias", "is a link"),
    ] {
        let error = push(&home, "payload", &remote, destination, &options)
            .await
            .unwrap_err();
        assert!(error.contains(expected), "{destination}: {error}");
    }
    assert!(!outside.join("planted").exists());
    assert_eq!(
        std::fs::read(worker.root.join("real/file")).unwrap().len(),
        10
    );
}

#[tokio::test]
async fn git_and_chariox_files_and_plan_mode_agents_never_receive_a_file() {
    let (home, worker) = workspaces();
    std::fs::create_dir_all(worker.root.join(".git/hooks")).unwrap();
    plant(&home.root, "hook", 10);
    let remote = FakeWorker::new(worker.clone());
    let options = CopyOptions::default();
    for destination in [
        ".git/hooks/pre-commit",
        "sub/.git/config",
        ".GIT/hooks/post-checkout",
        ".chariox/state.json",
        "nested/.chariox-instructions.md",
    ] {
        let error = push(&home, "hook", &remote, destination, &options)
            .await
            .unwrap_err();
        assert!(
            error.contains(".git") || error.contains("Chariox's own files"),
            "{destination}: {error}"
        );
        assert!(left_nothing(&worker.root, destination), "{destination}");
    }
    assert!(!worker.root.join(".git/hooks/pre-commit").exists());

    // Reading under .git is fine: the leased agent could print it anyway.
    plant(&worker.root, ".git/HEAD", 20);
    pull(&remote, ".git/HEAD", &home, "HEAD.copy", &options)
        .await
        .unwrap();

    let planning = FakeWorker::new(Workspace {
        read_only: true,
        ..worker.clone()
    });
    let error = push(&home, "hook", &planning, "hook", &options)
        .await
        .unwrap_err();
    assert!(error.contains("plan mode"), "{error}");
    assert!(left_nothing(&worker.root, "hook"));
    let planning_home = Workspace {
        read_only: true,
        ..home.clone()
    };
    let error = pull(&remote, ".git/HEAD", &planning_home, "HEAD.again", &options)
        .await
        .unwrap_err();
    assert!(error.contains("plan mode"), "{error}");
    assert!(left_nothing(&home.root, "HEAD.again"));
}

#[tokio::test]
async fn an_existing_file_is_replaced_only_with_overwrite_and_oversized_files_never_move() {
    let (home, worker) = workspaces();
    let sha256 = plant(&home.root, "new", 1000);
    plant(&worker.root, "old", 10);
    let remote = FakeWorker::new(worker.clone());
    let error = push(&home, "new", &remote, "old", &CopyOptions::default())
        .await
        .unwrap_err();
    assert!(error.contains("already exists"), "{error}");
    assert_eq!(std::fs::read(worker.root.join("old")).unwrap().len(), 10);
    let error = pull(&remote, "old", &home, "new", &CopyOptions::default())
        .await
        .unwrap_err();
    assert!(error.contains("already exists"), "{error}");
    assert_eq!(digest_of(&home.root.join("new")), sha256);
    let replace = CopyOptions {
        overwrite: true,
        expect_sha256: None,
        ..Default::default()
    };
    push(&home, "new", &remote, "old", &replace).await.unwrap();
    assert_eq!(digest_of(&worker.root.join("old")), sha256);

    let wrong = CopyOptions {
        overwrite: true,
        expect_sha256: Some("0".repeat(64)),
        ..Default::default()
    };
    let error = push(&home, "new", &remote, "other", &wrong)
        .await
        .unwrap_err();
    assert!(error.contains("not the expected"), "{error}");
    assert!(left_nothing(&worker.root, "other"));

    std::fs::File::create(worker.root.join("huge"))
        .unwrap()
        .set_len(home.max_file_bytes() + 1)
        .unwrap();
    let error = pull(&remote, "huge", &home, "huge", &CopyOptions::default())
        .await
        .unwrap_err();
    assert!(error.contains("at most 256 MiB"), "{error}");
    assert!(left_nothing(&home.root, "huge"));
    std::fs::rename(worker.root.join("huge"), home.root.join("huge")).unwrap();
    let error = push(&home, "huge", &remote, "huge", &CopyOptions::default())
        .await
        .unwrap_err();
    assert!(error.contains("at most 256 MiB"), "{error}");
    assert!(left_nothing(&worker.root, "huge"));
}

#[tokio::test]
async fn a_source_that_changes_mid_copy_ends_the_copy_with_nothing_written() {
    let (home, worker) = workspaces();
    plant(&worker.root, "moving", BUNDLE);
    let source = worker.root.join("moving");
    let remote = FakeWorker::cued(worker.clone(), move |request| {
        if request == 2 {
            let mut bytes = std::fs::read(&source).unwrap();
            bytes[5] ^= 1;
            std::fs::write(&source, bytes).unwrap();
        }
        Fault::None
    });
    let error = pull(&remote, "moving", &home, "landed", &CopyOptions::default())
        .await
        .unwrap_err();
    assert!(error.contains("changed while it was copied"), "{error}");
    assert!(left_nothing(&home.root, "landed"));

    plant(&home.root, "moving", BUNDLE);
    let local = home.root.join("moving");
    let remote = FakeWorker::cued(worker.clone(), move |request| {
        if request == 1 {
            let mut bytes = std::fs::read(&local).unwrap();
            let end = bytes.len() - 1;
            bytes[end] ^= 1;
            std::fs::write(&local, bytes).unwrap();
        }
        Fault::None
    });
    let error = push(&home, "moving", &remote, "landed", &CopyOptions::default())
        .await
        .unwrap_err();
    assert!(error.contains("changed while it was copied"), "{error}");
    assert!(left_nothing(&worker.root, "landed"));
}

#[tokio::test]
async fn a_copy_resumes_where_it_stopped_after_the_link_drops() {
    let (home, worker) = workspaces();
    let sha256 = plant(&worker.root, "big", BUNDLE);
    // The forwarder restarts while slices are in flight: three requests in a row never arrive.
    let remote = FakeWorker::cued(worker.clone(), |request| match request {
        2..=4 => Fault::Dropped,
        _ => Fault::None,
    });
    let pulled = pull(&remote, "big", &home, "big", &CopyOptions::default())
        .await
        .unwrap();
    assert_eq!(pulled.sha256, sha256);
    assert_eq!(digest_of(&home.root.join("big")), sha256);

    // Pushing, the link drops twice and two answers are lost, the last one after the file was
    // published: the home continues from what the worker holds and still gets the digest.
    let slices = BUNDLE.div_ceil(SLICE_BYTES as usize);
    let remote = FakeWorker::cued(worker.clone(), move |request| match request {
        1 => Fault::Dropped,
        2 => Fault::AnswerLost,
        4 => Fault::Dropped,
        n if n == slices + 2 => Fault::AnswerLost,
        _ => Fault::None,
    });
    let pushed = push(&home, "big", &remote, "again", &CopyOptions::default())
        .await
        .unwrap();
    assert_eq!(pushed.sha256, sha256);
    assert_eq!(digest_of(&worker.root.join("again")), sha256);
    assert!(!staging_left(&worker.root, "again"));
}

#[tokio::test]
async fn a_link_that_stays_down_ends_the_copy_with_nothing_published() {
    let (home, worker) = workspaces();
    plant(&home.root, "big", BUNDLE);
    let remote = FakeWorker::cued(worker.clone(), |request| {
        if request >= 1 {
            Fault::Dropped
        } else {
            Fault::None
        }
    });
    let error = push(&home, "big", &remote, "big", &CopyOptions::default())
        .await
        .unwrap_err();
    assert!(error.contains("stayed unreachable"), "{error}");
    assert!(!worker.root.join("big").exists());
}

#[tokio::test]
async fn a_staged_file_is_removed_once_the_link_is_back_after_the_copy_gave_up() {
    let (home, worker) = workspaces();
    plant(&home.root, "big", BUNDLE);
    // The write retries and the first discard both meet the dead link; it returns before the
    // discard gives up.
    let remote = FakeWorker::cued(worker.clone(), |request| {
        if (1..=5).contains(&request) {
            Fault::Dropped
        } else {
            Fault::None
        }
    });
    let error = push(&home, "big", &remote, "big", &CopyOptions::default())
        .await
        .unwrap_err();
    assert!(error.contains("stayed unreachable"), "{error}");
    assert!(left_nothing(&worker.root, "big"));
}

#[tokio::test]
async fn a_staged_file_that_could_not_be_removed_is_named_in_the_error() {
    let (home, worker) = workspaces();
    plant(&home.root, "big", BUNDLE);
    let remote = FakeWorker::cued(worker.clone(), |request| {
        if request >= 1 {
            Fault::Dropped
        } else {
            Fault::None
        }
    });
    let error = push(&home, "big", &remote, "big", &CopyOptions::default())
        .await
        .unwrap_err();
    assert!(error.contains(".chariox-copy-"), "{error}");
    assert!(error.contains("replaces it"), "{error}");
}

#[tokio::test]
async fn a_file_named_like_the_old_staging_sidecar_is_never_touched() {
    let (home, worker) = workspaces();
    let sidecar = plant(&worker.root, "report.chariox-partial", 50);
    plant(&home.root, "report", 70);

    // A finished copy to `report` leaves the neighbour alone, with or without overwrite.
    let remote = FakeWorker::new(worker.clone());
    push(&home, "report", &remote, "report", &CopyOptions::default())
        .await
        .unwrap();
    assert_eq!(
        digest_of(&worker.root.join("report.chariox-partial")),
        sidecar
    );

    // So does a copy that ends early: the source changes after the first slice.
    plant(&home.root, "moving", BUNDLE);
    let local = home.root.join("moving");
    let remote = FakeWorker::cued(worker.clone(), move |request| {
        if request == 1 {
            let mut bytes = std::fs::read(&local).unwrap();
            bytes[0] ^= 1;
            std::fs::write(&local, bytes).unwrap();
        }
        Fault::None
    });
    push(
        &home,
        "moving",
        &remote,
        "report",
        &CopyOptions {
            overwrite: true,
            expect_sha256: None,
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(
        digest_of(&worker.root.join("report.chariox-partial")),
        sidecar
    );

    // The same holds on the pulling side.
    let sidecar = plant(&home.root, "in.chariox-partial", 40);
    let remote = FakeWorker::new(worker.clone());
    pull(&remote, "report", &home, "in", &CopyOptions::default())
        .await
        .unwrap();
    assert_eq!(digest_of(&home.root.join("in.chariox-partial")), sidecar);
}

#[tokio::test]
async fn a_refused_copy_deletes_nothing_beside_its_destination() {
    let (home, worker) = workspaces();
    let sidecar = plant(&worker.root, "docs.chariox-partial", 50);
    let neighbour = plant(&worker.root, "docs", 60);
    plant(&home.root, "src", 70);
    let remote = FakeWorker::new(worker.clone());
    // Existing destination without overwrite, a protected destination and a missing source.
    for (source, destination) in [
        ("src", "docs"),
        ("src", ".git/hooks/pre-commit"),
        ("none", "x"),
    ] {
        push(&home, source, &remote, destination, &CopyOptions::default())
            .await
            .unwrap_err();
    }
    assert_eq!(
        digest_of(&worker.root.join("docs.chariox-partial")),
        sidecar
    );
    assert_eq!(digest_of(&worker.root.join("docs")), neighbour);
}

#[tokio::test]
async fn a_lost_answer_on_a_single_slice_push_still_reports_the_copy() {
    let (home, worker) = workspaces();
    for (name, size) in [("one", 100), ("empty", 0)] {
        let sha256 = plant(&home.root, name, size);
        let remote = FakeWorker::cued(worker.clone(), |request| {
            if request == 0 {
                Fault::AnswerLost
            } else {
                Fault::None
            }
        });
        let pushed = push(&home, name, &remote, name, &CopyOptions::default())
            .await
            .unwrap();
        assert_eq!(pushed.sha256, sha256, "{name}");
        assert_eq!(digest_of(&worker.root.join(name)), sha256, "{name}");
    }
}

#[tokio::test]
async fn a_lost_answer_does_not_make_an_unrelated_destination_count_as_copied() {
    let (home, worker) = workspaces();
    plant(&home.root, "one", 100);
    plant(&worker.root, "one", 90);
    let remote = FakeWorker::cued(worker.clone(), |request| {
        if request == 0 {
            Fault::AnswerLost
        } else {
            Fault::None
        }
    });
    let error = push(&home, "one", &remote, "one", &CopyOptions::default())
        .await
        .unwrap_err();
    assert!(error.contains("already exists"), "{error}");
}
