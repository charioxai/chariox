use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use crate::error::DaemonError;

pub(super) fn capture(
    helper: &str,
    archive_path: &Path,
    operation: &'static str,
) -> Result<(PathBuf, u64, String), DaemonError> {
    let error = |message: String| DaemonError::LocalTransport { operation, message };
    if super::broker::configured() {
        return Err(error(
            "managed home archives must use broker capture".into(),
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(archive_path)
        .map_err(|e| error(format!("failed to create private slice home archive: {e}")))?;
    let result = (|| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|e| error(format!("failed to protect slice home archive: {e}")))?;
        }
        // Docker writes directly to the private product file. Archive bytes never
        // enter a helper layer, an output Vec, or a diagnostic string.
        let output = file
            .try_clone()
            .map_err(|e| error(format!("failed to open slice archive output: {e}")))?;
        let mut child = Command::new("docker")
            .args([
                "exec",
                "-u",
                "root",
                helper,
                "bash",
                "-lc",
                "set -euo pipefail; cd /home-src; tar --zstd -cf - .",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::from(output))
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| error(format!("failed to stream slice home archive: {e}")))?;
        let status = match child.wait() {
            Ok(status) => status,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error(format!(
                    "failed to settle slice archive process: {e}"
                )));
            }
        };
        if !status.success() {
            return Err(error(format!("slice home archive failed with {status}")));
        }
        file.sync_all()
            .map_err(|e| error(format!("failed to sync slice home archive: {e}")))?;
        let size = file
            .metadata()
            .map_err(|e| error(format!("failed to inspect slice home archive: {e}")))?
            .len();
        if size == 0 {
            return Err(error("slice home archive is empty".into()));
        }
        file.seek(SeekFrom::Start(0))
            .map_err(|e| error(format!("failed to read slice archive digest: {e}")))?;
        let mut hash = Sha256::new();
        std::io::copy(&mut file, &mut hash)
            .map_err(|e| error(format!("failed to digest slice home archive: {e}")))?;
        Ok((
            archive_path.to_path_buf(),
            size,
            format!("{:x}", hash.finalize()),
        ))
    })();
    if result.is_err() {
        remove_created_archive(&file, archive_path);
    }
    result
}

fn remove_created_archive(file: &File, path: &Path) {
    // Never remove an existing generation or a path replaced while capturing.
    let Ok(created) = file.metadata() else { return };
    let Ok(current) = std::fs::symlink_metadata(path) else {
        return;
    };
    if !current.is_file() {
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if created.dev() != current.dev() || created.ino() != current.ino() {
            return;
        }
    }
    #[cfg(not(unix))]
    if created.created().ok() != current.created().ok() || created.len() != current.len() {
        return;
    }
    let _ = std::fs::remove_file(path);
}
