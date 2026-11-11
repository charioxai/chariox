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
