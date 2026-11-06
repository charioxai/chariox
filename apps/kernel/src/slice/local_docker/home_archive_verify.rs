use crate::runtime::bounded_process::BoundedCommandExt as _;
use std::fs::OpenOptions;
use std::io::Seek as _;
use std::path::Path;
use std::process::Stdio;

use crate::error::DaemonError;

// MP-08/MP-10/MP-11: pin the archive, then hash in the same cancellable,
// bounded-memory process supervisor used by broker preverification. Hashing a
// large archive is bulk work: it lives while the worker keeps reading the
// pinned file and ends once the read offset stalls for the archive progress
// timeout, the same policy the guard applies per block.
pub(super) fn digest(path: &Path, operation: &'static str) -> Result<(u64, String), DaemonError> {
    let guard = super::linux_docker_slice_script()?.with_file_name("slice-command-guard.py");
    let mut command = crate::process_spawn::command("python3");
    command.arg(guard).arg("digest-stdin");
    digest_with(
        command,
        path,
        operation,
        super::home_archive_capture::progress_timeout(),
    )
}

fn digest_with(
    mut command: std::process::Command,
    path: &Path,
    operation: &'static str,
    progress_timeout: std::time::Duration,
) -> Result<(u64, String), DaemonError> {
    let error = |message: String| DaemonError::LocalTransport { operation, message };
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|e| error(format!("failed to pin slice archive: {e}")))?;
    let before = file
        .metadata()
        .map_err(|e| error(format!("failed to inspect slice archive: {e}")))?;
    if !before.is_file() || before.len() == 0 || before.len() > 9_007_199_254_740_991 {
        return Err(error(
            "slice archive is not a nonempty regular file of supported size".into(),
        ));
    }
    // The worker shares this open file description, so its reads move this
    // offset: the only progress signal that needs no cooperation from the guard.
    let mut offset = {
        let pin = file
            .try_clone()
            .map_err(|e| error(format!("failed to retain slice archive pin: {e}")))?;
        move || (&pin).stream_position()
    };
    let output = command
        .stdin(Stdio::from(file.try_clone().map_err(|e| {
            error(format!("failed to retain slice archive pin: {e}"))
        })?))
        .output_with_progress(progress_timeout, &mut offset)
        .map_err(|e| {
            error(format!(
                "failed to supervise slice archive verification: {e}"
            ))
        })?;
    let after = file
        .metadata()
        .map_err(|e| error(format!("failed to reinspect slice archive: {e}")))?;
    #[cfg(unix)]
    let same = {
        use std::os::unix::fs::MetadataExt;
        let identity = |m: &std::fs::Metadata| {
            (
                m.dev(),
                m.ino(),
                m.len(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime(),
                m.ctime_nsec(),
            )
        };
        identity(&before) == identity(&after)
    };
    #[cfg(not(unix))]
    let same = before.len() == after.len() && before.modified().ok() == after.modified().ok();
    let digest = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || !same || !super::state::valid_sha256_digest(&digest) {
        return Err(error(
            "slice archive verification failed, changed, or made no progress".into(),
        ));
    }
    Ok((before.len(), digest))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn mp08_mp11_archive_preverification_pins_regular_input_and_hashes_it() {
        let _lock = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-archive-verify-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("archive");
        std::fs::write(&path, b"abc").unwrap();
        let result = digest(&path, "test.archive.verify").unwrap();
        assert_eq!(
            result,
            (
                3,
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into()
            )
        );
        symlink(&path, root.join("link")).unwrap();
        assert!(digest(&root.join("link"), "test.archive.verify").is_err());
        assert!(digest(&root, "test.archive.verify").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    // Stand-in for the guard's digest worker: reads stdin one byte per step.
    fn stand_in(
        steps: u32,
        pause: &str,
        then_stall: bool,
        pid_file: &Path,
    ) -> std::process::Command {
        let script = format!(
            "import hashlib,os,sys,time\n\
             open({pid:?},'w').write(str(os.getpid()))\n\
             h=hashlib.sha256()\n\
             for _ in range({steps}):\n\
             \td=os.read(0,1)\n\
             \th.update(d)\n\
             \ttime.sleep({pause})\n\
             if {then_stall}:\n\
             \ttime.sleep(60)\n\
             print(h.hexdigest())\n",
            pid = pid_file.to_str().unwrap(),
            then_stall = if then_stall { "True" } else { "False" },
        );
        let mut command = std::process::Command::new("python3");
        command.args(["-c", &script]);
        command
    }

    fn archive_in(root: &Path, len: usize) -> std::path::PathBuf {
        std::fs::create_dir_all(root).unwrap();
        let path = root.join("archive");
        std::fs::write(&path, vec![b'a'; len]).unwrap();
        path
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "chariox-archive-progress-{name}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ))
    }

    fn alive(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[test]
    fn mp08_mp11_archive_verification_outlives_generic_command_cap_while_progressing() {
        let _lock = crate::env_lock::lock();
        let root = scratch("slow");
        let path = archive_in(&root, 15);
        let started = std::time::Instant::now();
        // 15 steps of 200 ms is 3 s of steady progress against a 1 s generic cap.
        let result = crate::runtime::bounded_process::with_test_command_limit(
            std::time::Duration::from_secs(1),
            || {
                digest_with(
                    stand_in(15, "0.2", false, &root.join("pid")),
                    &path,
                    "test.archive.verify",
                    std::time::Duration::from_secs(1),
                )
            },
        );
        assert!(started.elapsed() > std::time::Duration::from_secs(2));
        let (len, digest) = result.unwrap();
        assert_eq!(len, 15);
        assert!(super::super::state::valid_sha256_digest(&digest));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mp08_mp11_archive_verification_that_stops_progressing_ends_at_the_progress_deadline() {
        let _lock = crate::env_lock::lock();
        let root = scratch("stalled");
        let path = archive_in(&root, 15);
        let pid_file = root.join("pid");
        let started = std::time::Instant::now();
        let result = digest_with(
            stand_in(3, "0.1", true, &pid_file),
            &path,
            "test.archive.verify",
            std::time::Duration::from_secs(1),
        );
        let elapsed = started.elapsed();
        assert!(result.is_err());
        assert!(elapsed < std::time::Duration::from_secs(5), "{elapsed:?}");
        let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
        assert!(!alive(pid), "stalled verifier must be reaped");
        std::fs::remove_dir_all(root).unwrap();
    }
}
