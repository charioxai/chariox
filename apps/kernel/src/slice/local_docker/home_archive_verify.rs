use std::fs::OpenOptions;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::error::DaemonError;

// MP-08/MP-10/MP-11: pin the archive, then hash in the same cancellable,
// bounded-memory process supervisor used by broker preverification.
pub(super) fn digest(path: &Path, operation: &'static str) -> Result<(u64, String), DaemonError> {
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
    let guard = super::linux_docker_slice_script()?.with_file_name("slice-command-guard.py");
    let output = Command::new("python3")
        .arg(guard)
        .arg("digest-stdin")
        .stdin(Stdio::from(file.try_clone().map_err(|e| {
            error(format!("failed to retain slice archive pin: {e}"))
        })?))
        .output()
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
}
