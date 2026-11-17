use std::fs::{self, File, OpenOptions};
use std::path::Path;

use fs2::FileExt;

use crate::error::DaemonError;

/// Exclusive hold on the lock file beside `config.json`, released when dropped or when the
/// process ends. The lock is on a stable sidecar because saving replaces `config.json` itself.
pub(super) struct DaemonConfigLock {
    _file: File,
}

impl DaemonConfigLock {
    /// Waits until no other thread or process holds the lock for the config at `config_path`.
    pub(super) fn acquire(
        config_path: &Path,
        operation: &'static str,
    ) -> Result<Self, DaemonError> {
        let failed = |error: std::io::Error| DaemonError::LocalTransport {
            operation,
            message: format!("lock the daemon config: {error}"),
        };
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).map_err(failed)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(config_path.with_file_name("config.json.lock"))
            .map_err(failed)?;
        file.lock_exclusive().map_err(failed)?;
        Ok(Self { _file: file })
    }
}
