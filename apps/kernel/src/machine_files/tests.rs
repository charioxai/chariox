//! MP-08 / MP-09 / MP-10 / MP-11 (files r1): what Files does on a machine and between machines,
//! from the transfer job a client or an agent starts to the bytes on the destination's disk. The
//! source is either this machine or another one behind a link that drops requests and loses
//! answers on cue, and serves them with the real `serve`.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::*;
use crate::error::DaemonError;
use crate::local::{
    FileTransferConflict, FileTransferEndpoint, FileTransferJob, FileTransferState, KernelFileKind,
    ListKernelDirectoryRequest, LocalDaemonResponse, ReadKernelFileRequest,
    StartFileTransferRequest, StatKernelFileRequest, WriteKernelFileRequest,
};
use crate::runtime::kernel_files;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

const SLICE: usize = crate::agent_file_copy::SLICE_BYTES as usize;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "machine-files-{name}-{:08x}",
        rand::random::<u32>()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn machine() -> Machine {
    Machine::new(StateGuard::default())
}

/// A file of `size` bytes no two slices of which are alike; its digest.
fn plant(path: &Path, size: usize) -> String {
    let bytes: Vec<u8> = (0..size).map(|i| (i * 31 + i / 4099) as u8).collect();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, &bytes).unwrap();
    format!("{:x}", Sha256::digest(&bytes))
}

fn digest_of(path: &Path) -> String {
    format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
}

/// A hidden staging file or folder of a transfer anywhere below `root`.
fn staging_left(root: &Path) -> bool {
    walk(root).iter().any(|path| {
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(".chariox-copy-"))
    })
}

fn walk(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        found.push(entry.path());
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            found.extend(walk(&entry.path()));
        }
    }
    found
}

// ---- the link to another machine --------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fault {
    None,
    /// The link drops before the other machine sees the request.
    Dropped,
}

/// Decides the fault for the n-th request.
type Cue = Box<dyn FnMut(usize, &RelayPeerRequest) -> Fault + Send>;

/// Another machine's files behind a link a test can cut and slow down.
struct Link {
    files: Machine,
    cue: Mutex<Cue>,
    served: AtomicUsize,
    requests: Mutex<Vec<RelayPeerRequest>>,
    delay: Duration,
}

impl Link {
    fn new(files: Machine) -> Arc<Self> {
        Arc::new(Self {
            files,
            cue: Mutex::new(Box::new(|_, _| Fault::None)),
            served: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
            delay: Duration::ZERO,
        })
    }

    fn with_cue(
        files: Machine,
        delay: Duration,
        cue: impl FnMut(usize, &RelayPeerRequest) -> Fault + Send + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            files,
            cue: Mutex::new(Box::new(cue)),
            served: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
            delay,
        })
    }

    fn source(self: &Arc<Self>) -> FileSource {
        let link = self.clone();
        FileSource::Peer(PeerFiles::new(
            "The machine `b3`",
            Arc::new(move |request| {
                let link = link.clone();
                Box::pin(async move {
                    let n = link.served.fetch_add(1, Ordering::SeqCst);
                    link.requests.lock().unwrap().push(request.clone());
                    if !link.delay.is_zero() {
                        tokio::time::sleep(link.delay).await;
                    }
                    let fault = (link.cue.lock().unwrap())(n, &request);
                    if fault == Fault::Dropped {
                        return Err(DaemonError::RelayTransport {
                            operation: "read relay peer response",
                            code: "target_not_connected".into(),
                            message: "target daemon is not connected to relay".into(),
                            retryable: true,
                        });
                    }
                    Ok(serve(&link.files, request))
                })
            }),
        ))
    }

    fn reads(&self) -> Vec<u64> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter_map(|request| match request {
                RelayPeerRequest::ReadMachineFile { offset, .. } => Some(*offset),
                _ => None,
            })
            .collect()
    }
}

// ---- running a job -----------------------------------------------------------------------

fn request(source: &Path, folder: &Path) -> StartFileTransferRequest {
    StartFileTransferRequest {
        source: FileTransferEndpoint {
            kernel: Some("b3".into()),
            path: source.display().to_string(),
        },
        destination_folder: folder.display().to_string(),
        name: None,
        conflict: FileTransferConflict::Fail,
        delete_source: false,
    }
}

async fn finished(jobs: &TransferJobs, id: &str) -> FileTransferJob {
    for _ in 0..1500 {
        let job = jobs.get(id).unwrap();
        if job.state.is_finished() {
            return job;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the transfer did not end: {:?}", jobs.get(id));
}

async fn run(
    jobs: &Arc<TransferJobs>,
    source: FileSource,
    request: StartFileTransferRequest,
) -> FileTransferJob {
    let started = jobs.start(request, source, machine());
    finished(jobs, &started.job_id).await
}

// ---- Files on one machine never reach the kernel's own state -----------------------------

struct EnvRestore(Vec<(&'static str, Option<std::ffi::OsString>)>);

impl EnvRestore {
    fn set(vars: &[(&'static str, &Path)]) -> Self {
        let saved = vars
            .iter()
            .map(|(name, value)| {
                let previous = std::env::var_os(name);
                std::env::set_var(name, value);
                (*name, previous)
            })
            .collect();
        Self(saved)
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (name, previous) in self.0.drain(..) {
            match previous {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}

#[test]
fn files_never_reach_the_kernels_own_state_or_the_vault() {
    use std::os::unix::fs::symlink;
    let _env = crate::env_lock::lock();
    let root = scratch("guard");
    let (kernel_home, user_home, outside) = (
        root.join("kernel-home"),
        root.join("user"),
        root.join("outside"),
    );
    for folder in [
        kernel_home.join("vault"),
        user_home.join(".chariox"),
        outside.clone(),
    ] {
        std::fs::create_dir_all(folder).unwrap();
    }
    std::fs::write(kernel_home.join("vault/vault.json"), b"secret").unwrap();
    std::fs::write(user_home.join(".chariox/config.toml"), b"token").unwrap();
    std::fs::write(outside.join("notes.txt"), b"mine").unwrap();
    let custom_vault = outside.join("my-vault.json");
    std::fs::write(&custom_vault, b"secret").unwrap();
    symlink(&kernel_home, outside.join("shortcut")).unwrap();
    let _restore = EnvRestore::set(&[("CHARIOX_HOME", &kernel_home), ("HOME", &user_home)]);
    let guard = StateGuard::new(Some(&custom_vault.display().to_string()));

    for protected in [
        kernel_home.clone(),
        kernel_home.join("vault/vault.json"),
        kernel_home.join("not-made-yet/deeper/file"),
        kernel_home.join("vault/../vault/vault.json"),
        outside.join("shortcut/vault/vault.json"),
        user_home.join(".chariox/config.toml"),
        custom_vault.clone(),
    ] {
        assert!(guard.check(&protected).is_err(), "{}", protected.display());
    }
    for open in [
        outside.join("notes.txt"),
        outside.clone(),
        root.clone(),
        user_home.clone(),
    ] {
        assert!(guard.check(&open).is_ok(), "{}", open.display());
    }

    // The same refusal on every Files request, and a folder holding the state does not show it.
    let say = |error: DaemonError| match error {
        DaemonError::MachineRequest { code, .. } => code,
        other => other.to_string(),
    };
    let secret = kernel_home.join("vault/vault.json").display().to_string();
    assert_eq!(
        say(kernel_files::stat(
            &StatKernelFileRequest {
                path: secret.clone(),
                with_sha256: true
            },
            &guard
        )
        .unwrap_err()),
        "file_state_protected"
    );
    assert_eq!(
        say(kernel_files::read(
            &ReadKernelFileRequest {
                path: secret.clone(),
                offset: 0,
                length: 6
            },
            &guard
        )
        .unwrap_err()),
        "file_state_protected"
    );
    assert_eq!(
        say(kernel_files::list(
            &ListKernelDirectoryRequest {
                path: kernel_home.display().to_string(),
                offset: 0
            },
            &guard
        )
        .unwrap_err()),
        "file_state_protected"
    );
    let write = |path: String| {
        kernel_files::write(
            &WriteKernelFileRequest {
                path,
                offset: 0,
                data_base64: "aGk=".into(),
                total_size: 2,
                sha256: None,
                overwrite: true,
            },
            &guard,
        )
    };
    assert_eq!(
        say(write(secret.clone()).unwrap_err()),
        "file_state_protected"
    );
    assert_eq!(std::fs::read(&secret).unwrap(), b"secret");
    let LocalDaemonResponse::KernelDirectoryListed { entries, .. } = kernel_files::list(
        &ListKernelDirectoryRequest {
            path: user_home.display().to_string(),
            offset: 0,
        },
        &guard,
    )
    .unwrap() else {
        panic!("a listing")
    };
    assert!(entries.is_empty(), "{entries:?}");
    let LocalDaemonResponse::KernelDirectoryListed { entries, .. } = kernel_files::list(
        &ListKernelDirectoryRequest {
            path: outside.display().to_string(),
            offset: 0,
        },
        &guard,
    )
    .unwrap() else {
        panic!("a listing")
    };
    let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
    assert!(names.contains(&"notes.txt"), "{names:?}");
    // The Vault file and a link that leads into the kernel's state are not shown.
    assert!(
        !names.contains(&"my-vault.json") && !names.contains(&"shortcut"),
        "{names:?}"
    );

    // A machine that serves another kernel refuses the same, in the same words.
    let machine = Machine::new(guard);
    for request in [
        RelayPeerRequest::StatMachinePath {
            path: secret.clone(),
        },
        RelayPeerRequest::ListMachineDirectory {
            path: kernel_home.display().to_string(),
            offset: 0,
        },
        RelayPeerRequest::ReadMachineFile {
            path: secret.clone(),
            offset: 0,
        },
        RelayPeerRequest::DeleteMachinePath {
            path: secret.clone(),
            expect_revision: None,
        },
    ] {
        let RelayPeerResponse::FileRefused { reason } = serve(&machine, request) else {
            panic!("served a path of the kernel's own state")
        };
        assert!(reason.contains("own state or Vault"), "{reason}");
    }
    assert_eq!(std::fs::read(&secret).unwrap(), b"secret");
}

#[tokio::test]
async fn a_transfer_never_writes_into_the_kernels_own_state_nor_reads_from_it() {
    let _env = crate::env_lock::lock();
    let root = scratch("state-transfer");
    let (kernel_home, other) = (root.join("kernel-home"), root.join("other-machine"));
    std::fs::create_dir_all(kernel_home.join("vault")).unwrap();
    std::fs::write(kernel_home.join("vault/vault.json"), b"secret").unwrap();
    plant(&other.join("report.bin"), 1000);
    plant(&other.join("state/keys"), 10);
    let _restore = EnvRestore::set(&[("CHARIOX_HOME", &kernel_home)]);
    let guarded = Machine::new(StateGuard::new(None));
    let jobs = Arc::new(TransferJobs::default());
    // Into the state: refused, and nothing appears there.
    let link = Link::new(Machine::new(StateGuard::default()));
    let job = jobs.start(
        request(&other.join("report.bin"), &kernel_home.join("vault")),
        link.source(),
        guarded.clone(),
    );
    let job = finished(&jobs, &job.job_id).await;
    assert_eq!(job.state, FileTransferState::Failed, "{job:?}");
    assert!(job.error.unwrap().contains("own state or Vault"));
    assert!(!kernel_home.join("vault/report.bin").exists());
    assert!(!staging_left(&kernel_home));
    // From the state of the machine that holds the source: refused by that machine.
    let link = Link::new(guarded);
    let job = run(
        &jobs,
        link.source(),
        request(&kernel_home.join("vault/vault.json"), &other),
    )
    .await;
    assert_eq!(job.state, FileTransferState::Failed, "{job:?}");
    assert!(job.error.unwrap().contains("own state or Vault"));
    assert!(!other.join("vault.json").exists());
}

// ---- one file -------------------------------------------------------------------------------

#[tokio::test]
async fn a_file_arrives_whole_with_the_sources_digest_and_permissions_and_no_staging_left() {
    use std::os::unix::fs::PermissionsExt;
    let (source_side, here) = (scratch("one-source"), scratch("one-here"));
    let size = 2 * SLICE + 4321;
    let sha256 = plant(&source_side.join("tools/run.sh"), size);
    std::fs::set_permissions(
        source_side.join("tools/run.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let link = Link::new(machine());
    let jobs = Arc::new(TransferJobs::default());

    let job = run(
        &jobs,
        link.source(),
        request(&source_side.join("tools/run.sh"), &here),
    )
    .await;

    assert_eq!(job.state, FileTransferState::Done, "{job:?}");
    assert_eq!(digest_of(&here.join("run.sh")), sha256);
    assert_eq!(job.sha256.as_deref(), Some(sha256.as_str()));
    assert_eq!(
        (
            job.bytes_done,
            job.bytes_total,
            job.files_done,
            job.files_total
        ),
        (size as u64, size as u64, 1, 1)
    );
    assert_eq!(job.destination, here.join("run.sh").display().to_string());
    assert_eq!(
        std::fs::metadata(here.join("run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert!(!staging_left(&here));
    assert!(
        source_side.join("tools/run.sh").exists(),
        "a copy leaves the source"
    );
}

#[tokio::test]
async fn a_file_larger_than_an_agents_workspace_copy_cap_comes_across() {
    // The 256 MiB cap of a workspace copy does not apply to machines: a sparse 300 MiB file.
    let (source_side, here) = (scratch("big-source"), scratch("big-here"));
    let big = source_side.join("disk.img");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(300 * 1024 * 1024)
        .unwrap();
    let jobs = Arc::new(TransferJobs::default());
    let job = run(&jobs, Link::new(machine()).source(), request(&big, &here)).await;
    assert_eq!(job.state, FileTransferState::Done, "{job:?}");
    assert_eq!(
        std::fs::metadata(here.join("disk.img")).unwrap().len(),
        300 * 1024 * 1024
    );
}

#[tokio::test]
async fn a_dropped_link_is_waited_out_and_the_copy_still_ends_with_the_right_digest() {
    let (source_side, here) = (scratch("drop-source"), scratch("drop-here"));
    let sha256 = plant(&source_side.join("a.bundle"), 4 * SLICE + 99);
    // Requests 3 to 6 never reach the other machine.
    let link = Link::with_cue(machine(), Duration::ZERO, |n, _| {
        if (3..7).contains(&n) {
            Fault::Dropped
        } else {
            Fault::None
        }
    });
    let jobs = Arc::new(TransferJobs::default());
    let job = run(
        &jobs,
        link.source(),
        request(&source_side.join("a.bundle"), &here),
    )
    .await;
    assert_eq!(job.state, FileTransferState::Done, "{job:?}");
    assert_eq!(digest_of(&here.join("a.bundle")), sha256);
    assert!(!staging_left(&here));
}

#[tokio::test]
async fn a_copy_stopped_by_a_long_outage_continues_from_the_staged_bytes_when_started_again() {
    let (source_side, here) = (scratch("resume-source"), scratch("resume-here"));
    let size = 5 * SLICE + 17;
    let sha256 = plant(&source_side.join("big.bin"), size);
    // The link carries the stat and the first three reads, then stays down.
    let down = Arc::new(AtomicUsize::new(0));
    let cut = down.clone();
    let link = Link::with_cue(machine(), Duration::ZERO, move |_, request| {
        if matches!(request, RelayPeerRequest::ReadMachineFile { .. })
            && cut.fetch_add(1, Ordering::SeqCst) >= 4
        {
            return Fault::Dropped;
        }
        Fault::None
    });
    let jobs = Arc::new(TransferJobs::default());
    let failed = run(
        &jobs,
        link.source(),
        request(&source_side.join("big.bin"), &here),
    )
    .await;
    assert_eq!(failed.state, FileTransferState::Failed, "{failed:?}");
    assert!(failed.error.unwrap().contains("stayed unreachable"));
    assert!(!here.join("big.bin").exists(), "nothing is published");
    assert!(
        staging_left(&here),
        "the staged bytes wait for the next try"
    );

    let back = Link::new(machine());
    let job = run(
        &jobs,
        back.source(),
        request(&source_side.join("big.bin"), &here),
    )
    .await;
    assert_eq!(job.state, FileTransferState::Done, "{job:?}");
    assert_eq!(digest_of(&here.join("big.bin")), sha256);
    // Only what was missing travelled: the second try never read from the start.
    let reads = back.reads();
    assert_eq!(reads[0], 0, "the digest comes with the first slice");
    assert!(
        reads[1..].iter().all(|offset| *offset >= 2 * SLICE as u64),
        "{reads:?}"
    );
    assert!(!staging_left(&here));
}

#[tokio::test]
async fn a_source_that_changes_during_the_copy_ends_it_with_nothing_published() {
    let (source_side, here) = (scratch("change-source"), scratch("change-here"));
    let file = source_side.join("live.log");
    plant(&file, 4 * SLICE);
    let touched = file.clone();
    let link = Link::with_cue(machine(), Duration::ZERO, move |n, request| {
        if n == 3 && matches!(request, RelayPeerRequest::ReadMachineFile { .. }) {
            std::fs::write(&touched, vec![7_u8; 4 * SLICE]).unwrap();
        }
        Fault::None
    });
    let jobs = Arc::new(TransferJobs::default());
    let job = run(&jobs, link.source(), request(&file, &here)).await;
    assert_eq!(job.state, FileTransferState::Failed, "{job:?}");
    assert!(job.error.unwrap().contains("changed while it was copied"));
    assert!(!here.join("live.log").exists());
    assert!(!staging_left(&here));
}

// ---- a folder -------------------------------------------------------------------------------

fn plant_tree(root: &Path) -> Vec<(String, String)> {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let mut planted = Vec::new();
    for (relative, size) in [
        ("top.txt", 10),
        ("src/main.rs", 2000),
        ("src/deep/er/data.bin", SLICE + 5),
        ("src/empty.txt", 0),
        ("docs/readme.md", 77),
    ] {
        planted.push((relative.to_owned(), plant(&root.join(relative), size)));
    }
    std::fs::create_dir_all(root.join("no-files/at-all")).unwrap();
    std::fs::set_permissions(
        root.join("src/main.rs"),
        std::fs::Permissions::from_mode(0o750),
    )
    .unwrap();
    symlink("top.txt", root.join("link-to-top")).unwrap();
    planted
}

#[tokio::test]
async fn a_folder_arrives_as_the_same_tree_published_all_at_once() {
    use std::os::unix::fs::PermissionsExt;
    let (source_side, here) = (scratch("tree-source"), scratch("tree-here"));
    let tree = source_side.join("project");
    let planted = plant_tree(&tree);
    let link = Link::new(machine());
    let jobs = Arc::new(TransferJobs::default());

    let job = run(&jobs, link.source(), request(&tree, &here)).await;

    assert_eq!(job.state, FileTransferState::Done, "{job:?}");
    for (relative, sha256) in &planted {
        assert_eq!(
            &digest_of(&here.join("project").join(relative)),
            sha256,
            "{relative}"
        );
    }
    assert!(here.join("project/no-files/at-all").is_dir());
    assert!(
        !here.join("project/link-to-top").exists(),
        "a link is not followed or copied"
    );
    assert_eq!(job.skipped, 1);
    assert_eq!(job.files_total, planted.len() as u64);
    assert_eq!(job.files_done, job.files_total);
    assert_eq!(job.bytes_done, job.bytes_total);
    assert_eq!(
        std::fs::metadata(here.join("project/src/main.rs"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o750
    );
    assert!(!staging_left(&here));
}

#[tokio::test]
async fn a_folder_that_fails_halfway_publishes_nothing_and_leaves_no_staging() {
    let (source_side, here) = (scratch("half-source"), scratch("half-here"));
    let tree = source_side.join("project");
    plant_tree(&tree);
    // One file vanishes from the source just as it is read.
    let gone = tree.join("src/deep/er/data.bin");
    let link = Link::with_cue(machine(), Duration::ZERO, move |_, request| {
        if matches!(request, RelayPeerRequest::ReadMachineFile { path, .. } if path.ends_with("data.bin"))
        {
            let _ = std::fs::remove_file(&gone);
        }
        Fault::None
    });
    let jobs = Arc::new(TransferJobs::default());
    let job = run(&jobs, link.source(), request(&tree, &here)).await;
    assert_eq!(job.state, FileTransferState::Failed, "{job:?}");
    assert!(job.error.unwrap().contains("data.bin"));
    assert!(
        !here.join("project").exists(),
        "a half-copied folder is never published"
    );
    assert!(!staging_left(&here));
}

// ---- names that are taken ---------------------------------------------------------------------

#[tokio::test]
async fn a_taken_name_is_replaced_kept_beside_skipped_or_refused_as_asked() {
    let (source_side, here) = (scratch("name-source"), scratch("name-here"));
    let new = plant(&source_side.join("notes.txt"), 300);
    std::fs::write(here.join("notes.txt"), b"old").unwrap();
    let jobs = Arc::new(TransferJobs::default());
    let link = Link::new(machine());
    let with = |conflict| StartFileTransferRequest {
        conflict,
        ..request(&source_side.join("notes.txt"), &here)
    };

    let refused = run(&jobs, link.source(), with(FileTransferConflict::Fail)).await;
    assert_eq!(refused.state, FileTransferState::Failed);
    let reason = refused.error.unwrap();
    assert!(
        reason.contains("already exists")
            && reason.contains("replace")
            && reason.contains("keep_both")
            && reason.contains("skip"),
        "{reason}"
    );
    assert_eq!(std::fs::read(here.join("notes.txt")).unwrap(), b"old");

    let skipped = run(&jobs, link.source(), with(FileTransferConflict::Skip)).await;
    assert_eq!(
        (skipped.state, skipped.skipped),
        (FileTransferState::Done, 1)
    );
    assert_eq!(std::fs::read(here.join("notes.txt")).unwrap(), b"old");

    let both = run(&jobs, link.source(), with(FileTransferConflict::KeepBoth)).await;
    assert_eq!(both.state, FileTransferState::Done, "{both:?}");
    assert_eq!(std::fs::read(here.join("notes.txt")).unwrap(), b"old");
    assert_eq!(digest_of(&here.join("notes (2).txt")), new);
    let again = run(&jobs, link.source(), with(FileTransferConflict::KeepBoth)).await;
    assert_eq!(
        again.destination,
        here.join("notes (3).txt").display().to_string()
    );

    let replaced = run(&jobs, link.source(), with(FileTransferConflict::Replace)).await;
    assert_eq!(replaced.state, FileTransferState::Done, "{replaced:?}");
    assert_eq!(digest_of(&here.join("notes.txt")), new);
    assert!(!staging_left(&here));
}

#[tokio::test]
async fn replacing_a_folder_swaps_it_whole_and_keep_both_and_skip_leave_it_alone() {
    let (source_side, here) = (scratch("dir-name-source"), scratch("dir-name-here"));
    let tree = source_side.join("photos");
    plant(&tree.join("new.jpg"), 500);
    std::fs::create_dir_all(here.join("photos")).unwrap();
    std::fs::write(here.join("photos/old.jpg"), b"old").unwrap();
    let jobs = Arc::new(TransferJobs::default());
    let link = Link::new(machine());
    let with = |conflict| StartFileTransferRequest {
        conflict,
        ..request(&tree, &here)
    };

    let skipped = run(&jobs, link.source(), with(FileTransferConflict::Skip)).await;
    assert_eq!(skipped.state, FileTransferState::Done);
    assert!(here.join("photos/old.jpg").exists() && !here.join("photos/new.jpg").exists());

    let both = run(&jobs, link.source(), with(FileTransferConflict::KeepBoth)).await;
    assert_eq!(both.state, FileTransferState::Done, "{both:?}");
    assert!(here.join("photos (2)/new.jpg").exists() && here.join("photos/old.jpg").exists());

    let replaced = run(&jobs, link.source(), with(FileTransferConflict::Replace)).await;
    assert_eq!(replaced.state, FileTransferState::Done, "{replaced:?}");
    assert!(here.join("photos/new.jpg").exists());
    assert!(
        !here.join("photos/old.jpg").exists(),
        "replace swaps the whole folder"
    );
    assert!(!staging_left(&here));
}

// ---- cancel and the job outliving its caller ---------------------------------------------------

#[tokio::test]
async fn a_canceled_copy_publishes_nothing_and_leaves_no_staging_file_or_folder() {
    let (source_side, here) = (scratch("cancel-source"), scratch("cancel-here"));
    plant(&source_side.join("big.bin"), 12 * SLICE);
    plant(&source_side.join("tree/a.bin"), 6 * SLICE);
    plant(&source_side.join("tree/b.bin"), 6 * SLICE);
    let jobs = Arc::new(TransferJobs::default());
    for item in ["big.bin", "tree"] {
        let link = Link::with_cue(machine(), Duration::from_millis(40), |_, _| Fault::None);
        let started = jobs.start(
            request(&source_side.join(item), &here),
            link.source(),
            machine(),
        );
        // Wait until bytes are on their way, then cancel.
        for _ in 0..500 {
            if jobs.get(&started.job_id).unwrap().bytes_done > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(jobs.get(&started.job_id).unwrap().bytes_done > 0);
        jobs.cancel(&started.job_id).unwrap();
        let job = finished(&jobs, &started.job_id).await;
        assert_eq!(job.state, FileTransferState::Canceled, "{item}: {job:?}");
        assert!(!here.join(item).exists(), "{item}");
        assert!(!staging_left(&here), "{item}");
    }
}

#[tokio::test]
async fn a_transfer_goes_on_and_stays_readable_after_whoever_asked_has_gone() {
    let (source_side, here) = (scratch("away-source"), scratch("away-here"));
    plant(&source_side.join("f.bin"), 3 * SLICE);
    let jobs = Arc::new(TransferJobs::default());
    let link = Link::with_cue(machine(), Duration::from_millis(30), |_, _| Fault::None);
    let id = jobs
        .start(
            request(&source_side.join("f.bin"), &here),
            link.source(),
            machine(),
        )
        .job_id;
    // Nothing polls it: no client is attached.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let listed = jobs.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].job_id, id);
    assert_eq!(listed[0].state, FileTransferState::Done, "{:?}", listed[0]);
    assert!(listed[0].finished_ms.is_some());
    assert!(here.join("f.bin").exists());
}

// ---- move --------------------------------------------------------------------------------------

#[tokio::test]
async fn a_move_deletes_the_source_only_after_the_copy_is_verified() {
    let (source_side, here) = (scratch("move-source"), scratch("move-here"));
    let sha256 = plant(&source_side.join("one.bin"), SLICE + 1);
    let tree = source_side.join("tree");
    let planted = plant_tree(&tree);
    let jobs = Arc::new(TransferJobs::default());
    let link = Link::new(machine());
    let moving = |path: &Path| StartFileTransferRequest {
        delete_source: true,
        ..request(path, &here)
    };

    let file = run(&jobs, link.source(), moving(&source_side.join("one.bin"))).await;
    assert_eq!(file.state, FileTransferState::Done, "{file:?}");
    assert_eq!(digest_of(&here.join("one.bin")), sha256);
    assert!(!source_side.join("one.bin").exists());

    let folder = run(&jobs, link.source(), moving(&tree)).await;
    assert_eq!(folder.state, FileTransferState::Done, "{folder:?}");
    for (relative, sha256) in &planted {
        assert_eq!(&digest_of(&here.join("tree").join(relative)), sha256);
    }
    // The files and folders it copied are gone from the source; the link it skipped stays with its folder.
    assert!(!tree.join("src/main.rs").exists() && !tree.join("src/deep").exists());
    assert!(
        std::fs::symlink_metadata(tree.join("link-to-top")).is_ok(),
        "what was not copied is not deleted"
    );
}

#[tokio::test]
async fn a_move_whose_source_changed_after_the_copy_keeps_the_source_and_says_so() {
    let (source_side, here) = (scratch("keep-source"), scratch("keep-here"));
    let file = source_side.join("doc.txt");
    plant(&file, 5000);
    let edited = file.clone();
    // The file is edited just before the other machine is asked to delete it.
    let link = Link::with_cue(machine(), Duration::ZERO, move |_, request| {
        if matches!(request, RelayPeerRequest::DeleteMachinePath { .. }) {
            std::fs::write(&edited, b"edited meanwhile").unwrap();
        }
        Fault::None
    });
    let jobs = Arc::new(TransferJobs::default());
    let job = run(
        &jobs,
        link.source(),
        StartFileTransferRequest {
            delete_source: true,
            ..request(&file, &here)
        },
    )
    .await;
    assert_eq!(job.state, FileTransferState::Failed, "{job:?}");
    let reason = job.error.unwrap();
    assert!(
        reason.contains("copy is complete and verified")
            && reason.contains("changed after it was copied"),
        "{reason}"
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"edited meanwhile");
    assert_eq!(std::fs::metadata(here.join("doc.txt")).unwrap().len(), 5000);
}

#[tokio::test]
async fn a_move_whose_copy_fails_keeps_the_source() {
    let (source_side, here) = (scratch("fail-source"), scratch("fail-here"));
    let file = source_side.join("keep.bin");
    plant(&file, 3 * SLICE);
    let link = Link::with_cue(machine(), Duration::ZERO, |n, _| {
        if n >= 2 {
            Fault::Dropped
        } else {
            Fault::None
        }
    });
    let jobs = Arc::new(TransferJobs::default());
    let job = run(
        &jobs,
        link.source(),
        StartFileTransferRequest {
            delete_source: true,
            ..request(&file, &here)
        },
    )
    .await;
    assert_eq!(job.state, FileTransferState::Failed);
    assert!(file.exists());
    assert!(!here.join("keep.bin").exists());
}

// ---- listing a folder with revisions and pages --------------------------------------------------

#[test]
fn a_listing_names_each_entrys_kind_size_time_and_revision_and_pages_by_offset() {
    let root = scratch("listing");
    for n in 0..5 {
        plant(&root.join(format!("f{n}.txt")), 10 + n);
    }
    std::fs::create_dir(root.join("sub")).unwrap();
    let guard = StateGuard::default();
    let page = |offset| {
        let LocalDaemonResponse::KernelDirectoryListed {
            entries, truncated, ..
        } = kernel_files::list(
            &ListKernelDirectoryRequest {
                path: root.display().to_string(),
                offset,
            },
            &guard,
        )
        .unwrap()
        else {
            panic!("a listing")
        };
        (entries, truncated)
    };
    let (entries, truncated) = page(0);
    assert!(!truncated);
    assert_eq!(entries.len(), 6);
    assert_eq!(entries[0].name, "f0.txt");
    assert_eq!(
        (entries[0].kind, entries[0].size),
        (KernelFileKind::File, 10)
    );
    assert!(entries[0].modified_ms.is_some() && !entries[0].revision.is_empty());
    assert_eq!(entries[5].kind, KernelFileKind::Directory);
    let (rest, _) = page(4);
    assert_eq!(
        rest.iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["f4.txt", "sub"]
    );
    // A write to the file changes its revision.
    let before = entries[0].revision.clone();
    std::thread::sleep(Duration::from_millis(5));
    std::fs::write(root.join("f0.txt"), b"changed content").unwrap();
    assert_ne!(page(0).0[0].revision, before);
}

// ---- a kernel that stops and starts again -------------------------------------------------

/// One life of a kernel: the transfers it runs die with its runtime, as they do with the process.
fn kernel_life() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

/// The record a job has on disk, once it holds `needle`.
async fn recorded(state: &Path, id: &str, needle: &str) {
    let path = state.join(format!("{id}.json"));
    for _ in 0..500 {
        if std::fs::read_to_string(&path).is_ok_and(|text| text.contains(needle)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no record of {id} holding {needle}");
}

async fn first_life(
    state: &Path,
    source: FileSource,
    request: StartFileTransferRequest,
) -> (tokio::runtime::Runtime, Arc<TransferJobs>, FileTransferJob) {
    let life = kernel_life();
    let jobs = Arc::new(TransferJobs::default());
    assert!(jobs.attach(state.to_owned()).is_empty());
    let (started, jobs_in) = (jobs.clone(), jobs.clone());
    let job = life
        .spawn(async move { started.start(request, source, machine()) })
        .await
        .unwrap();
    drop(jobs_in);
    (life, jobs, job)
}

#[tokio::test]
async fn a_file_transfer_stopped_by_a_kernel_restart_is_listed_again_and_finishes_from_the_staged_bytes(
) {
    let (source_side, here, state) = (
        scratch("restart-source"),
        scratch("restart-here"),
        scratch("restart-state"),
    );
    let size = 12 * SLICE + 5;
    let sha256 = plant(&source_side.join("big.bin"), size);
    let slow = Link::with_cue(machine(), Duration::from_millis(60), |_, _| Fault::None);
    let (life, jobs, started) = first_life(
        &state,
        slow.source(),
        request(&source_side.join("big.bin"), &here),
    )
    .await;
    for _ in 0..500 {
        if jobs.get(&started.job_id).unwrap().bytes_done >= 3 * SLICE as u64 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    recorded(&state, &started.job_id, "\"copying\"").await;
    // The kernel stops without removing staged bytes. The shutdown barrier
    // joins the transfer and drains its records before a new life attaches.
    jobs.shutdown().await.unwrap();
    tokio::task::spawn_blocking(move || life.shutdown_timeout(Duration::from_secs(1)))
        .await
        .unwrap();
    drop(jobs);
    assert!(!here.join("big.bin").exists());
    assert!(
        staging_left(&here),
        "the staged bytes wait for the next life"
    );

    // The kernel starts again.
    let jobs = Arc::new(TransferJobs::default());
    let interrupted = jobs.attach(state.clone());
    assert_eq!(interrupted.len(), 1);
    assert_eq!(interrupted[0].job_id, started.job_id);
    let listed = jobs.list();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].job_id, started.job_id);
    assert!(!listed[0].state.is_finished(), "{:?}", listed[0]);

    let back = Link::new(machine());
    jobs.resume(&interrupted[0], back.source(), machine());
    let done = finished(&jobs, &started.job_id).await;
    assert_eq!(done.state, FileTransferState::Done, "{done:?}");
    assert_eq!(done.sha256.as_deref(), Some(sha256.as_str()));
    assert_eq!(digest_of(&here.join("big.bin")), sha256);
    let reads = back.reads();
    assert_eq!(reads[0], 0, "the digest comes with the first slice");
    assert!(
        reads[1..].iter().all(|offset| *offset >= 3 * SLICE as u64),
        "only what was missing travelled: {reads:?}"
    );
    assert!(!staging_left(&here));

    // The next store reads the durable outcome after the old writer settles.
    recorded(&state, &started.job_id, "\"done\"").await;
    let jobs = Arc::new(TransferJobs::default());
    assert!(jobs.attach(state).is_empty());
    let kept = jobs.get(&started.job_id).unwrap();
    assert_eq!(kept.state, FileTransferState::Done);
    assert_eq!(kept.sha256.as_deref(), Some(sha256.as_str()));
}

#[tokio::test]
async fn a_folder_transfer_stopped_by_a_kernel_restart_finishes_as_the_same_tree() {
    let (source_side, here, state) = (
        scratch("restart-tree-source"),
        scratch("restart-tree-here"),
        scratch("restart-tree-state"),
    );
    let tree = source_side.join("project");
    let planted: Vec<(String, String)> = (0..6)
        .map(|n| {
            let relative = format!("part-{n}/data.bin");
            let sha256 = plant(&tree.join(&relative), 3 * SLICE + n);
            (relative, sha256)
        })
        .collect();
    let slow = Link::with_cue(machine(), Duration::from_millis(60), |_, _| Fault::None);
    let (life, jobs, started) = first_life(&state, slow.source(), request(&tree, &here)).await;
    for _ in 0..500 {
        if jobs.get(&started.job_id).unwrap().bytes_done >= 4 * SLICE as u64 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    recorded(&state, &started.job_id, "\"copying\"").await;
    // The kernel stops without removing staged bytes. The shutdown barrier
    // joins the transfer and drains its records before a new life attaches.
    jobs.shutdown().await.unwrap();
    tokio::task::spawn_blocking(move || life.shutdown_timeout(Duration::from_secs(1)))
        .await
        .unwrap();
    drop(jobs);
    assert!(!here.join("project").exists());

    let jobs = Arc::new(TransferJobs::default());
    let interrupted = jobs.attach(state);
    assert_eq!(interrupted.len(), 1);
    jobs.resume(&interrupted[0], Link::new(machine()).source(), machine());
    let done = finished(&jobs, &started.job_id).await;
    assert_eq!(done.state, FileTransferState::Done, "{done:?}");
    assert_eq!(done.files_done, 6);
    for (relative, sha256) in &planted {
        assert_eq!(&digest_of(&here.join("project").join(relative)), sha256);
    }
    assert!(!staging_left(&here));
}

#[tokio::test]
async fn a_move_stopped_while_it_deleted_the_source_ends_with_a_clear_outcome_and_the_source_kept()
{
    let (source_side, here, state) = (
        scratch("restart-move-source"),
        scratch("restart-move-here"),
        scratch("restart-move-state"),
    );
    plant(&source_side.join("f.bin"), SLICE);
    plant(&here.join("f.bin"), SLICE);
    let mut moving = request(&source_side.join("f.bin"), &here);
    moving.delete_source = true;
    let job_id = "transfer-00000000000000aa";
    std::fs::write(
        state.join(format!("{job_id}.json")),
        serde_json::json!({
            "job": {
                "job_id": job_id, "state": "finishing",
                "source": moving.source, "destination": here.join("f.bin"),
                "delete_source": true, "bytes_done": SLICE, "bytes_total": SLICE,
                "files_done": 1, "files_total": 1, "skipped": 0, "started_ms": 1
            },
            "request": moving,
        })
        .to_string(),
    )
    .unwrap();

    let jobs = Arc::new(TransferJobs::default());
    assert!(jobs.attach(state).is_empty(), "nothing is repeated blindly");
    let job = jobs.get(job_id).unwrap();
    assert_eq!(job.state, FileTransferState::Failed, "{job:?}");
    let error = job.error.unwrap();
    assert!(error.contains("copy is complete and verified"), "{error}");
    assert!(error.contains("restart"), "{error}");
    assert!(source_side.join("f.bin").exists());
}

// ---- review of #1346: what a replace may touch, modes, overlapping jobs, retried staging -----

#[tokio::test]
async fn replacing_a_folder_that_holds_the_kernels_state_is_refused_before_anything_moves() {
    let _env = crate::env_lock::lock();
    let root = scratch("holds-state");
    let kernel_home = root.join("data/kernel-home");
    std::fs::create_dir_all(kernel_home.join("vault")).unwrap();
    std::fs::write(kernel_home.join("vault/vault.json"), b"secret").unwrap();
    plant(&root.join("data/notes.txt"), 10);
    let source_side = scratch("holds-state-source");
    plant(&source_side.join("data/new.txt"), 10);
    let _restore = EnvRestore::set(&[("CHARIOX_HOME", &kernel_home)]);
    let guarded = Machine::new(StateGuard::new(None));
    let jobs = Arc::new(TransferJobs::default());
    let mut replace = request(&source_side.join("data"), &root);
    replace.conflict = FileTransferConflict::Replace;
    let job = jobs.start(
        replace,
        Link::new(Machine::new(StateGuard::default())).source(),
        guarded,
    );
    let job = finished(&jobs, &job.job_id).await;
    assert_eq!(job.state, FileTransferState::Failed, "{job:?}");
    assert!(job.error.unwrap().contains("holds"));
    assert_eq!(
        std::fs::read(kernel_home.join("vault/vault.json")).unwrap(),
        b"secret"
    );
    assert!(root.join("data/notes.txt").exists());
    assert!(!root.join("data/new.txt").exists());
    assert!(!staging_left(&root));
}

#[tokio::test]
async fn a_private_folder_keeps_its_mode_and_its_children_stay_private_while_they_are_copied() {
    use std::os::unix::fs::PermissionsExt;
    let (source_side, here) = (scratch("private-source"), scratch("private-here"));
    let tree = source_side.join("private");
    plant(&tree.join("readable.txt"), 100);
    plant(&tree.join("inner/deep.txt"), 100);
    let mode_of = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o7777;
    std::fs::set_permissions(&tree, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::set_permissions(tree.join("inner"), std::fs::Permissions::from_mode(0o750)).unwrap();
    // SAFETY: umask only changes this process's creation mask; the test restores it.
    let previous = unsafe { libc::umask(0o022) };
    let jobs = Arc::new(TransferJobs::default());
    let job = run(&jobs, Link::new(machine()).source(), request(&tree, &here)).await;
    unsafe { libc::umask(previous) };
    assert_eq!(job.state, FileTransferState::Done, "{job:?}");
    assert_eq!(mode_of(&here.join("private")), 0o700);
    assert_eq!(mode_of(&here.join("private/inner")), 0o750);
    assert_eq!(mode_of(&here.join("private/readable.txt")), 0o644);
}

#[tokio::test]
async fn two_transfers_to_one_destination_never_share_a_staging_tree() {
    let (source_side, here) = (scratch("overlap-source"), scratch("overlap-here"));
    let (first, second) = (source_side.join("a/project"), source_side.join("b/project"));
    let planted_first: Vec<(String, String)> = (0..3)
        .map(|n| {
            let relative = format!("f{n}.bin");
            (
                relative.clone(),
                plant(&first.join(&relative), 2 * SLICE + n),
            )
        })
        .collect();
    for n in 0..3 {
        plant(&second.join(format!("f{n}.bin")), 2 * SLICE + 7 + n);
    }
    let slow = Link::with_cue(machine(), Duration::from_millis(40), |_, _| Fault::None);
    let jobs = Arc::new(TransferJobs::default());
    let a = jobs.start(request(&first, &here), slow.source(), machine());
    tokio::time::sleep(Duration::from_millis(100)).await;
    // A move from the other machine to the same destination, while the first is under way.
    let mut moving = request(&second, &here);
    moving.source.kernel = Some("other".into());
    moving.delete_source = true;
    let b = jobs.start(moving, slow.source(), machine());
    let (a, b) = (
        finished(&jobs, &a.job_id).await,
        finished(&jobs, &b.job_id).await,
    );
    assert_eq!(a.state, FileTransferState::Done, "{a:?}");
    assert_eq!(b.state, FileTransferState::Failed, "{b:?}");
    assert!(b.error.unwrap().contains("another transfer"));
    for (relative, sha256) in &planted_first {
        assert_eq!(&digest_of(&here.join("project").join(relative)), sha256);
    }
    assert!(
        second.join("f0.bin").exists(),
        "a refused move keeps its source"
    );
    assert!(!staging_left(&here));

    // Under keep_both each lands under its own name.
    let (third, fourth) = (source_side.join("c/work"), source_side.join("d/work"));
    let sha_third = plant(&third.join("x.bin"), 2 * SLICE);
    let sha_fourth = plant(&fourth.join("x.bin"), 2 * SLICE + 1);
    let (mut c, mut d) = (request(&third, &here), request(&fourth, &here));
    c.conflict = FileTransferConflict::KeepBoth;
    d.conflict = FileTransferConflict::KeepBoth;
    let c = jobs.start(c, slow.source(), machine());
    let d = jobs.start(d, slow.source(), machine());
    let (c, d) = (
        finished(&jobs, &c.job_id).await,
        finished(&jobs, &d.job_id).await,
    );
    assert_eq!(c.state, FileTransferState::Done, "{c:?}");
    assert_eq!(d.state, FileTransferState::Done, "{d:?}");
    assert_ne!(c.destination, d.destination);
    for (job, sha256) in [(&c, &sha_third), (&d, &sha_fourth)] {
        assert_eq!(
            &digest_of(&Path::new(&job.destination).join("x.bin")),
            sha256
        );
    }
}

#[tokio::test]
async fn a_retried_folder_drops_staged_entries_the_source_no_longer_has() {
    let (source_side, here) = (scratch("retry-source"), scratch("retry-here"));
    let tree = source_side.join("project");
    plant(&tree.join("a.txt"), 100);
    plant(&tree.join("b.txt"), 100);
    plant(&tree.join("sub/c.txt"), 100);
    plant(&tree.join("keep.txt"), 100);
    plant(&tree.join("zzz.bin"), 5 * SLICE);
    // The big file's link stays down, so the small files are staged and the first try gives up.
    let cut = Link::with_cue(machine(), Duration::ZERO, |_, request| {
        if matches!(request, RelayPeerRequest::ReadMachineFile { path, offset } if path.ends_with("zzz.bin") && *offset > 0)
        {
            return Fault::Dropped;
        }
        Fault::None
    });
    let jobs = Arc::new(TransferJobs::default());
    let first = run(&jobs, cut.source(), request(&tree, &here)).await;
    assert_eq!(first.state, FileTransferState::Failed, "{first:?}");
    assert!(first.error.unwrap().contains("stayed unreachable"));
    assert!(staging_left(&here));

    // Between the tries: a file goes, a folder becomes a file, a file becomes a folder.
    std::fs::remove_file(tree.join("a.txt")).unwrap();
    std::fs::remove_dir_all(tree.join("sub")).unwrap();
    plant(&tree.join("sub"), 50);
    std::fs::remove_file(tree.join("b.txt")).unwrap();
    plant(&tree.join("b.txt/inner.txt"), 50);

    let second = run(&jobs, Link::new(machine()).source(), request(&tree, &here)).await;
    assert_eq!(second.state, FileTransferState::Done, "{second:?}");
    let listing = |root: &Path| {
        let mut found: Vec<String> = walk(root)
            .iter()
            .map(|path| path.strip_prefix(root).unwrap().display().to_string())
            .collect();
        found.sort();
        found
    };
    assert_eq!(listing(&here.join("project")), listing(&tree));
    for relative in ["sub", "b.txt/inner.txt", "keep.txt", "zzz.bin"] {
        assert_eq!(
            digest_of(&here.join("project").join(relative)),
            digest_of(&tree.join(relative)),
            "{relative}"
        );
    }
    assert!(!staging_left(&here));
}

#[test]
fn a_folders_staging_is_named_by_the_machine_it_comes_from_as_well() {
    let endpoint = |kernel: &str| FileTransferEndpoint {
        kernel: Some(kernel.into()),
        path: "/srv/project".into(),
    };
    let destination = Path::new("/data/project");
    let (a, b) = (
        super::jobs::folder_stage(&endpoint("a"), destination),
        super::jobs::folder_stage(&endpoint("b"), destination),
    );
    assert_ne!(a, b);
    assert_eq!(a, super::jobs::folder_stage(&endpoint("a"), destination));
}

// ---- review of #1346 (second round): the deleting phase, aliases of one folder -----------------

#[tokio::test]
async fn a_move_is_on_record_as_deleting_before_its_first_source_file_goes() {
    let (source_side, here, state) = (
        scratch("deleting-source"),
        scratch("deleting-here"),
        scratch("deleting-state"),
    );
    let tree = source_side.join("project");
    let planted: Vec<(String, String)> = (0..3)
        .map(|n| {
            let relative = format!("f{n}.bin");
            (relative.clone(), plant(&tree.join(&relative), 1000 + n))
        })
        .collect();
    plant(&here.join("project/old.txt"), 10);
    let jobs = Arc::new(TransferJobs::default());
    assert!(jobs.attach(state.clone()).is_empty());
    // The kernel dies as the second source file is about to be deleted; the first is gone.
    let (killer, deletes) = (jobs.clone(), Arc::new(AtomicUsize::new(0)));
    let link = Link::with_cue(machine(), Duration::ZERO, move |_, request| {
        if matches!(request, RelayPeerRequest::DeleteMachinePath { .. }) {
            if deletes.fetch_add(1, Ordering::SeqCst) == 1 {
                killer.kill_runs();
            }
            if deletes.load(Ordering::SeqCst) > 1 {
                return Fault::Dropped;
            }
        }
        Fault::None
    });
    let mut moving = request(&tree, &here);
    moving.conflict = FileTransferConflict::Replace;
    moving.delete_source = true;
    let id = jobs.start(moving, link.source(), machine()).job_id;
    tokio::time::sleep(Duration::from_millis(800)).await;
    drop(jobs);
    assert_eq!(
        std::fs::read_dir(&tree).unwrap().count(),
        2,
        "one source file was deleted before the kernel died"
    );

    // The kernel starts again: the move is not repeated, whatever the copy it published.
    let jobs = Arc::new(TransferJobs::default());
    let interrupted = jobs.attach(state);
    assert!(
        interrupted.is_empty(),
        "a move that began deleting is never resumed"
    );
    let job = jobs.get(&id).unwrap();
    assert_eq!(job.state, FileTransferState::Failed, "{job:?}");
    assert!(job.error.unwrap().contains("partly deleted"));
    for (relative, sha256) in &planted {
        assert_eq!(&digest_of(&here.join("project").join(relative)), sha256);
    }
}

#[tokio::test]
async fn two_spellings_of_one_destination_and_nested_destinations_are_one_place() {
    let (source_side, here) = (scratch("alias-source"), scratch("alias-here"));
    std::fs::create_dir_all(here.join("data")).unwrap();
    std::os::unix::fs::symlink(here.join("data"), here.join("alias")).unwrap();
    let (first, second) = (
        source_side.join("a/report.bin"),
        source_side.join("b/report.bin"),
    );
    plant(&first, 3 * SLICE);
    plant(&second, 3 * SLICE + 1);
    let slow = Link::with_cue(machine(), Duration::from_millis(40), |_, _| Fault::None);
    let jobs = Arc::new(TransferJobs::default());
    let a = jobs.start(
        request(&first, &here.join("data")),
        slow.source(),
        machine(),
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    let b = jobs.start(
        request(&second, &here.join("alias")),
        slow.source(),
        machine(),
    );
    let (a, b) = (
        finished(&jobs, &a.job_id).await,
        finished(&jobs, &b.job_id).await,
    );
    assert_eq!(a.state, FileTransferState::Done, "{a:?}");
    assert_eq!(b.state, FileTransferState::Failed, "{b:?}");
    assert!(b.error.unwrap().contains("another transfer"));
    assert_eq!(digest_of(&here.join("data/report.bin")), digest_of(&first));

    // A transfer into a folder another one is creating is refused as well.
    let tree = source_side.join("c/proj");
    for n in 0..3 {
        plant(&tree.join(format!("f{n}.bin")), 2 * SLICE + n);
    }
    let inner = source_side.join("d/inner.bin");
    plant(&inner, 100);
    let c = jobs.start(request(&tree, &here), slow.source(), machine());
    tokio::time::sleep(Duration::from_millis(150)).await;
    let d = jobs.start(
        request(&inner, &here.join("proj")),
        slow.source(),
        machine(),
    );
    let (c, d) = (
        finished(&jobs, &c.job_id).await,
        finished(&jobs, &d.job_id).await,
    );
    assert_eq!(c.state, FileTransferState::Done, "{c:?}");
    assert_eq!(d.state, FileTransferState::Failed, "{d:?}");
    assert!(d.error.unwrap().contains("another transfer"));
    assert!(!here.join("proj/inner.bin").exists());
}

#[tokio::test]
async fn a_startup_snapshot_written_late_never_overwrites_a_newer_outcome() {
    let (source_side, here, state) = (
        scratch("late-source"),
        scratch("late-here"),
        scratch("late-state"),
    );
    let sha256 = plant(&source_side.join("f.bin"), 1000);
    let id = "transfer-00000000000000bb";
    std::fs::write(
        state.join(format!("{id}.json")),
        serde_json::json!({
            "job": {
                "job_id": id, "state": "copying",
                "source": { "kernel": "b3", "path": source_side.join("f.bin") },
                "destination": "", "delete_source": false, "bytes_done": 0, "bytes_total": 1000,
                "files_done": 0, "files_total": 1, "skipped": 0, "started_ms": 1
            },
            "request": request(&source_side.join("f.bin"), &here),
        })
        .to_string(),
    )
    .unwrap();
    // The write of the snapshot taken at startup is held back until the job has long ended.
    super::records::FIRST_WRITE_DELAY_MS.store(600, Ordering::SeqCst);
    let jobs = Arc::new(TransferJobs::default());
    let interrupted = jobs.attach(state.clone());
    assert_eq!(interrupted.len(), 1);
    jobs.resume(&interrupted[0], Link::new(machine()).source(), machine());
    let done = finished(&jobs, id).await;
    assert_eq!(done.state, FileTransferState::Done, "{done:?}");
    assert_eq!(digest_of(&here.join("f.bin")), sha256);
    // A visible terminal outcome must already survive a restart, even when
    // the writer was delayed. Do not give the writer time to catch up here.
    let text = std::fs::read_to_string(state.join(format!("{id}.json"))).unwrap();
    assert!(text.contains("\"done\""), "{text}");
    // Once the delayed startup snapshot lands, the newer outcome still stands.
    tokio::time::timeout(Duration::from_secs(30), jobs.startup_recorded())
        .await
        .expect("the delayed startup snapshot should finish")
        .expect("the startup snapshot should be durable");
    let text = std::fs::read_to_string(state.join(format!("{id}.json"))).unwrap();
    assert!(text.contains("\"done\""), "{text}");

    // The next start finds it finished and runs nothing again.
    let jobs = Arc::new(TransferJobs::default());
    assert!(jobs.attach(state).is_empty());
    assert_eq!(jobs.get(id).unwrap().state, FileTransferState::Done);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_keep_both_transfer_into_a_folder_another_transfer_is_creating_is_refused_without_searching_names(
) {
    let (source_side, here) = (scratch("inside-source"), scratch("inside-here"));
    let tree = source_side.join("proj");
    for n in 0..3 {
        plant(&tree.join(format!("f{n}.bin")), 2 * SLICE + n);
    }
    let single = source_side.join("x.bin");
    plant(&single, 100);
    let slow = Link::with_cue(machine(), Duration::from_millis(300), |_, _| Fault::None);
    let jobs = Arc::new(TransferJobs::default());
    let a = jobs.start(request(&tree, &here), slow.source(), machine());
    tokio::time::sleep(Duration::from_millis(700)).await;
    // Every name in that folder lies inside the folder the first transfer is writing.
    let mut inside = request(&single, &here.join("proj"));
    inside.conflict = FileTransferConflict::KeepBoth;
    let b = jobs.start(inside, slow.source(), machine());
    let b = tokio::time::timeout(Duration::from_secs(5), finished(&jobs, &b.job_id))
        .await
        .expect("the transfer ended instead of searching names for ever");
    assert_eq!(b.state, FileTransferState::Failed, "{b:?}");
    assert!(b.error.unwrap().contains("another transfer"));
    let a = finished(&jobs, &a.job_id).await;
    assert_eq!(a.state, FileTransferState::Done, "{a:?}");
    assert!(!here.join("proj/x.bin").exists());
}

// A deterministic resume name is untrusted until the staging root is opened and checked.
#[tokio::test]
async fn l9_a_preexisting_staging_root_link_never_prunes_its_target() {
    use std::os::unix::fs::symlink;
    let root = scratch("stage-link");
    let source = root.join("source/project");
    plant(&source.join("new.txt"), 10);
    let here = root.join("destination");
    std::fs::create_dir(&here).unwrap();
    let victim = root.join("victim");
    let digest = plant(&victim.join("keep.txt"), 20);
    let request = request(&source, &here);
    let stage = super::jobs::folder_stage(&request.source, &here.join("project"));
    symlink(&victim, &stage).unwrap();
    let jobs = Arc::new(TransferJobs::default());
    let job = run(&jobs, FileSource::Local(machine()), request).await;
    assert_eq!(job.state, FileTransferState::Failed, "{job:?}");
    assert_eq!(digest_of(&victim.join("keep.txt")), digest);
    assert!(!here.join("project").exists());
    assert!(std::fs::symlink_metadata(stage).unwrap().is_symlink());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn l9_an_untrusted_existing_staging_directory_is_left_untouched() {
    use std::os::unix::fs::PermissionsExt;
    let root = scratch("stage-permissions");
    let source = root.join("source/project");
    plant(&source.join("new.txt"), 10);
    let here = root.join("destination");
    std::fs::create_dir(&here).unwrap();
    let request = request(&source, &here);
    let stage = super::jobs::folder_stage(&request.source, &here.join("project"));
    let digest = plant(&stage.join("keep.txt"), 20);
    std::fs::set_permissions(&stage, std::fs::Permissions::from_mode(0o777)).unwrap();
    let jobs = Arc::new(TransferJobs::default());
    let job = run(&jobs, FileSource::Local(machine()), request).await;
    assert_eq!(job.state, FileTransferState::Failed, "{job:?}");
    assert_eq!(digest_of(&stage.join("keep.txt")), digest);
    assert!(!here.join("project").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn l9_replacing_the_source_folder_through_a_parent_alias_is_refused() {
    use std::os::unix::fs::symlink;
    let root = scratch("same-folder");
    let source = root.join("project");
    let digest = plant(&source.join("keep.txt"), 20);
    symlink("keep.txt", source.join("link.txt")).unwrap();
    let alias = root.join("alias");
    symlink(&root, &alias).unwrap();
    let mut request = request(&source, &alias);
    request.conflict = FileTransferConflict::Replace;
    let jobs = Arc::new(TransferJobs::default());
    let job = run(&jobs, FileSource::Local(machine()), request).await;
    assert_eq!(job.state, FileTransferState::Failed, "{job:?}");
    assert!(job.error.unwrap().contains("source"));
    assert_eq!(digest_of(&source.join("keep.txt")), digest);
    assert!(std::fs::symlink_metadata(source.join("link.txt"))
        .unwrap()
        .is_symlink());
    assert!(!staging_left(&root));
    std::fs::remove_dir_all(root).unwrap();
}

async fn home_source_replacement_is_refused(source_path: &str, reason: &str) {
    use std::os::unix::fs::symlink;
    let _env = crate::env_lock::lock();
    let root = scratch("same-home-folder");
    let _restore = EnvRestore::set(&[("HOME", &root)]);
    let source = root.join("project");
    let digest = plant(&source.join("keep.txt"), 20);
    symlink("keep.txt", source.join("link.txt")).unwrap();
    let mut request = request(Path::new(source_path), Path::new("~/"));
    request.conflict = FileTransferConflict::Replace;
    let jobs = Arc::new(TransferJobs::default());
    let job = run(&jobs, FileSource::Local(machine()), request).await;
    let file_preserved = digest_of(&source.join("keep.txt")) == digest;
    let link_preserved = std::fs::symlink_metadata(source.join("link.txt"))
        .is_ok_and(|metadata| metadata.is_symlink());
    let staged = staging_left(&root);
    std::fs::remove_dir_all(root).unwrap();
    assert_eq!(job.state, FileTransferState::Failed, "{job:?}");
    assert!(job.error.unwrap().contains(reason));
    assert!(file_preserved);
    assert!(link_preserved);
    assert!(!staged);
}

#[tokio::test]
async fn l9_home_source_replacement_is_refused() {
    home_source_replacement_is_refused("~/project", "source itself").await;
}

#[tokio::test]
async fn l9_home_source_replacement_with_a_trailing_slash_is_refused() {
    home_source_replacement_is_refused("~/project/", "source itself").await;
}

#[tokio::test]
async fn l9_home_source_replacement_with_a_current_directory_is_refused() {
    home_source_replacement_is_refused("~/./project", "source itself").await;
}

#[tokio::test]
async fn l9_home_source_replacement_with_a_parent_directory_is_refused() {
    home_source_replacement_is_refused("~/project/../project", "must not contain").await;
}

#[tokio::test]
async fn a_failed_outcome_write_never_reports_a_successful_transfer() {
    let (source_side, here, state) = (
        scratch("outcome-write-source"),
        scratch("outcome-write-here"),
        scratch("outcome-write-state"),
    );
    let sha256 = plant(&source_side.join("f.bin"), 1000);
    let not_a_folder = state.join("not-a-folder");
    std::fs::write(&not_a_folder, b"cannot store records here").unwrap();
    let jobs = Arc::new(TransferJobs::default());
    assert!(jobs.attach(not_a_folder).is_empty());
    let done = run(
        &jobs,
        Link::new(machine()).source(),
        request(&source_side.join("f.bin"), &here),
    )
    .await;
    assert_eq!(done.state, FileTransferState::Failed, "{done:?}");
    assert!(
        done.error
            .as_deref()
            .unwrap()
            .contains("could not be recorded"),
        "{done:?}"
    );
    assert_eq!(digest_of(&here.join("f.bin")), sha256);
}
