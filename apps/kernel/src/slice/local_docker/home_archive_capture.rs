use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

use crate::error::DaemonError;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArchivePolicy {
    schema_version: u32,
    minimum_free_bytes: u64,
}

pub(super) fn minimum_free_bytes() -> u64 {
    static RESERVE: OnceLock<u64> = OnceLock::new();
    *RESERVE.get_or_init(|| {
        let policy: ArchivePolicy = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/slice-linux-docker/home-archive-policy.json"
        )))
        .expect("packaged home archive policy must be valid");
        assert_eq!(policy.schema_version, 1);
        assert!(policy.minimum_free_bytes <= 9_007_199_254_740_991);
        policy.minimum_free_bytes
    })
}

pub(super) fn capture(
    helper: &str,
    archive_path: &Path,
    operation: &'static str,
) -> Result<(PathBuf, u64, String), DaemonError> {
    let parent = archive_path.parent().unwrap_or_else(|| Path::new("."));
    capture_with_available_space(helper, archive_path, operation, || {
        fs2::available_space(parent)
    })
}

pub(super) fn capture_with_available_space(
    helper: &str,
    archive_path: &Path,
    operation: &'static str,
    mut available: impl FnMut() -> std::io::Result<u64>,
) -> Result<(PathBuf, u64, String), DaemonError> {
    let error = |message: String| DaemonError::LocalTransport { operation, message };
    if super::broker::configured() {
        return Err(error(
            "managed home archives must use broker capture".into(),
        ));
    }
    let reserve = minimum_free_bytes();
    let space_error = || error("insufficient space for slice home archive".into());
    if available().map_err(|e| error(format!("failed to inspect archive storage: {e}")))? <= reserve
    {
        return Err(space_error());
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
        // Direct tar output is streamed through a fixed buffer to private product
        // state. No login shell, helper-layer archive, or diagnostic byte capture.
        let mut child = Command::new("docker")
            .args([
                "exec",
                "-u",
                "root",
                helper,
                "tar",
                "--zstd",
                "-C",
                "/home-src",
                "-cf",
                "-",
                ".",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| error(format!("failed to stream slice home archive: {e}")))?;
        let captured = (|| {
            let mut output = child
                .stdout
                .take()
                .ok_or_else(|| error("archive output pipe is unavailable".into()))?;
            let mut buffer = [0_u8; 64 * 1024];
            let mut hash = Sha256::new();
            let mut size = 0_u64;
            loop {
                let count = read_chunk(&mut output, &mut buffer)
                    .map_err(|e| error(format!("failed to read slice home archive: {e}")))?;
                if count == 0 {
                    break;
                }
                if available()
                    .map_err(|e| error(format!("failed to inspect archive storage: {e}")))?
                    < reserve.saturating_add(count as u64)
                {
                    return Err(space_error());
                }
                size = size
                    .checked_add(count as u64)
                    .filter(|n| *n <= 9_007_199_254_740_991)
                    .ok_or_else(|| {
                        error("slice home archive size cannot be represented safely".into())
                    })?;
                file.write_all(&buffer[..count])
                    .map_err(|e| error(format!("failed to write slice home archive: {e}")))?;
                hash.update(&buffer[..count]);
            }
            let status = child
                .wait()
                .map_err(|e| error(format!("failed to settle slice archive process: {e}")))?;
            if !status.success() {
                return Err(error(format!("slice home archive failed with {status}")));
            }
            if size == 0 {
                return Err(error("slice home archive is empty".into()));
            }
            if available().map_err(|e| error(format!("failed to inspect archive storage: {e}")))?
                < reserve
            {
                return Err(space_error());
            }
            file.sync_all()
                .map_err(|e| error(format!("failed to sync slice home archive: {e}")))?;
            Ok((
                archive_path.to_path_buf(),
                size,
                format!("{:x}", hash.finalize()),
            ))
        })();
        if captured.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        captured
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

fn read_chunk(output: &mut impl Read, buffer: &mut [u8]) -> std::io::Result<usize> {
    loop {
        match output.read(buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_archive_stream_retries_interrupted_reads_without_losing_bytes() {
        struct Interrupted(bool);
        impl Read for Interrupted {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if !self.0 {
                    self.0 = true;
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                buffer[..3].copy_from_slice(&[0, 255, 1]);
                Ok(3)
            }
        }
        let mut buffer = [0; 3];
        assert_eq!(read_chunk(&mut Interrupted(false), &mut buffer).unwrap(), 3);
        assert_eq!(buffer, [0, 255, 1]);
    }
}
