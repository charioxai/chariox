use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub(super) const VALIDATION_SCRATCH_DIR_ENV: &str =
    "CHARIOX_PROJECT_ENVIRONMENT_SETUP_SCRATCH_DIR";

const SCRATCH_PREFIX: &str = "chariox-project-environment-setup-";
const OWNER_MARKER: &str = ".chariox-project-setup-owner";

pub(super) struct WorkerValidationScratch {
    path: PathBuf,
    owner_token: String,
}

impl WorkerValidationScratch {
    pub(super) fn create(
        temporary_root: &Path,
        workspace_root: &Path,
        durable_home: &Path,
        operation_id: &str,
        attempt: u32,
        command_index: usize,
    ) -> Result<Self, String> {
        if operation_id.is_empty() || attempt == 0 {
            return Err("validation scratch identity is incomplete".to_string());
        }
        let temporary_root = canonical_directory(temporary_root, "worker temporary root")?;
        let workspace_root = canonical_directory(workspace_root, "worker project worktree")?;
        let durable_home = canonical_directory(durable_home, "durable worker HOME")?;
        let owner_token = owner_token(operation_id, attempt, command_index);
        let path = temporary_root.join(format!("{SCRATCH_PREFIX}{}", &owner_token[7..]));
        if paths_overlap(&path, &workspace_root) {
            return Err("validation scratch overlaps the project worktree".to_string());
        }
        if paths_overlap(&path, &durable_home) {
            return Err("validation scratch overlaps durable worker HOME".to_string());
        }

        fs::create_dir(&path).map_err(|error| {
            format!("operation-owned validation scratch could not be created: {error}")
        })?;
        #[cfg(unix)]
        if let Err(error) = fs::set_permissions(
            &path,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        ) {
            let _ = fs::remove_dir(&path);
            return Err(format!("validation scratch permissions could not be set: {error}"));
        }

        let marker_path = path.join(OWNER_MARKER);
        let marker_result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
            }
            let mut marker = options
                .open(&marker_path)
                .map_err(|error| format!("validation scratch owner marker could not be created: {error}"))?;
            marker
                .write_all(owner_token.as_bytes())
                .and_then(|()| marker.sync_all())
                .map_err(|error| format!("validation scratch owner marker could not be written: {error}"))
        })();
        if let Err(error) = marker_result {
            // Do not remove a marker whose ownership was not established by
            // the create_new open. Removing the empty directory is safe; a
            // raced or foreign marker leaves the command failed and intact.
            let _ = fs::remove_dir(&path);
            return Err(error);
        }

        Ok(Self { path, owner_token })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn cleanup(&self) -> Result<(), String> {
        let directory = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("validation scratch could not be inspected: {error}")),
        };
        if directory.file_type().is_symlink() || !directory.is_dir() {
            return Err("validation scratch is not an owned real directory".to_string());
        }

        let marker_path = self.path.join(OWNER_MARKER);
        let marker_metadata = fs::symlink_metadata(&marker_path)
            .map_err(|error| format!("validation scratch owner marker could not be inspected: {error}"))?;
        if marker_metadata.file_type().is_symlink() || !marker_metadata.is_file() {
            return Err("validation scratch owner marker is not a regular file".to_string());
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let mut marker = options
            .open(&marker_path)
            .map_err(|error| format!("validation scratch owner marker could not be read: {error}"))?;
        let mut actual_owner = String::new();
        marker
            .read_to_string(&mut actual_owner)
            .map_err(|error| format!("validation scratch owner marker could not be read: {error}"))?;
        if actual_owner != self.owner_token {
            return Err("validation scratch owner marker changed".to_string());
        }

        fs::remove_dir_all(&self.path)
            .map_err(|error| format!("owned validation scratch could not be removed: {error}"))?;
        match fs::symlink_metadata(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err("owned validation scratch remains after cleanup".to_string()),
            Err(error) => Err(format!("validation scratch cleanup could not be verified: {error}")),
        }
    }
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(path)
        .map_err(|error| format!("{label} could not be canonicalized: {error}"))?;
    let metadata = fs::symlink_metadata(&canonical)
        .map_err(|error| format!("{label} could not be inspected: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("{label} must be a real directory"));
    }
    Ok(canonical)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn owner_token(operation_id: &str, attempt: u32, command_index: usize) -> String {
    let mut digest = Sha256::new();
    digest.update(b"chariox-project-environment-setup-validation-scratch-v1\0");
    digest.update(operation_id.as_bytes());
    digest.update([0]);
    digest.update(attempt.to_be_bytes());
    digest.update((command_index as u64).to_be_bytes());
    format!("sha256:{:x}", digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::project_environment_setup_validation::run_worker_validation_command_with_scratch;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    struct ScratchFixture {
        root: PathBuf,
        workspace: PathBuf,
        durable_home: PathBuf,
        temporary_root: PathBuf,
    }

    impl ScratchFixture {
        fn new() -> Self {
            static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "chariox-validation-scratch-test-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
            ));
            let workspace = root.join("workspace");
            let durable_home = root.join("durable-home");
            let temporary_root = root.join("external-temp");
            fs::create_dir_all(&workspace).expect("test worktree should be created");
            fs::create_dir_all(&durable_home).expect("test durable HOME should be created");
            fs::create_dir_all(&temporary_root).expect("test temp root should be created");
            Self { root, workspace, durable_home, temporary_root }
        }

        fn scratch(&self, operation_id: &str, attempt: u32, command_index: usize) -> WorkerValidationScratch {
            WorkerValidationScratch::create(
                &self.temporary_root,
                &self.workspace,
                &self.durable_home,
                operation_id,
                attempt,
                command_index,
            )
            .expect("operation-scoped scratch should be created")
        }

        fn environment(&self) -> BTreeMap<String, String> {
            BTreeMap::from([
                ("PATH".to_string(), "/usr/bin:/bin".to_string()),
                ("HOME".to_string(), self.durable_home.display().to_string()),
            ])
        }

        fn assert_durable_cache(&self) {
            assert_eq!(
                fs::read(self.durable_home.join(".cargo/registry/cache/keep"))
                    .expect("durable Cargo cache should remain"),
                b"preserve",
            );
        }
    }

    impl Drop for ScratchFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn validation_scratch_is_operation_attempt_and_command_scoped() {
        let fixture = ScratchFixture::new();
        let first = fixture.scratch("setup-one", 1, 4);
        let retry = fixture.scratch("setup-one", 2, 4);
        let next_command = fixture.scratch("setup-one", 1, 5);
        assert_ne!(first.path(), retry.path());
        assert_ne!(first.path(), next_command.path());
        first.cleanup().unwrap();
        retry.cleanup().unwrap();
        next_command.cleanup().unwrap();
    }

    #[test]
    fn validation_scratch_rejects_worktree_or_durable_home_overlap() {
        let fixture = ScratchFixture::new();
        assert!(WorkerValidationScratch::create(
            &fixture.workspace,
            &fixture.workspace,
            &fixture.durable_home,
            "setup-overlap-workspace",
            1,
            0,
        )
        .is_err());
        assert!(WorkerValidationScratch::create(
            &fixture.durable_home,
            &fixture.workspace,
            &fixture.durable_home,
            "setup-overlap-home",
            1,
            0,
        )
        .is_err());
    }

    #[test]
    fn validation_scratch_cleanup_rejects_foreign_owner_marker() {
        let fixture = ScratchFixture::new();
        let scratch = fixture.scratch("setup-foreign-marker", 1, 0);
        fs::write(scratch.path.join(OWNER_MARKER), "foreign-owner")
            .expect("foreign marker should replace fixture marker");
        assert!(scratch.cleanup().unwrap_err().contains("marker changed"));
        assert!(scratch.path.exists(), "foreign-owned scratch must be preserved");
    }

    #[cfg(unix)]
    #[test]
    fn validation_scratch_cleanup_rejects_symlink_replacement() {
        use std::os::unix::fs::symlink;

        let fixture = ScratchFixture::new();
        let scratch = fixture.scratch("setup-symlink", 1, 0);
        let moved = fixture.temporary_root.join("moved-owned-scratch");
        fs::rename(&scratch.path, &moved).expect("owned scratch should move for the fixture");
        let sentinel = fixture.root.join("foreign-sentinel");
        fs::create_dir(&sentinel).expect("foreign sentinel should exist");
        fs::write(sentinel.join("keep"), b"preserve").expect("sentinel data should be written");
        symlink(&sentinel, &scratch.path).expect("replacement symlink should be created");

        assert!(scratch.cleanup().unwrap_err().contains("real directory"));
        assert_eq!(fs::read(sentinel.join("keep")).unwrap(), b"preserve");
        assert!(fs::symlink_metadata(&scratch.path).unwrap().file_type().is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn successful_and_failed_validation_commands_both_clean_only_their_scratch() {
        let fixture = ScratchFixture::new();
        fs::create_dir_all(fixture.durable_home.join(".cargo/registry/cache"))
            .expect("Cargo cache fixture should be created");
        fs::write(fixture.durable_home.join(".cargo/registry/cache/keep"), b"preserve")
            .expect("Cargo cache fixture should be written");

        for (operation_id, command, expected_exit) in [
            ("setup-success", 0),
            ("setup-failure", 29),
        ] {
            let scratch = fixture.scratch(operation_id, 1, 0);
            let mut environment = fixture.environment();
            environment.insert(
                VALIDATION_SCRATCH_DIR_ENV.to_string(),
                "/foreign/pre-sanitization-value".to_string(),
            );
            environment.insert(
                "SCRATCH_TEST_EXPECTED_DIR".to_string(),
                scratch.path().display().to_string(),
            );
            let command = format!(
                "set -eu; test \"${VALIDATION_SCRATCH_DIR_ENV}\" = \"$SCRATCH_TEST_EXPECTED_DIR\"; mkdir -p \"${VALIDATION_SCRATCH_DIR_ENV}/target\"; exit {expected_exit}"
            );
            let result = run_worker_validation_command_with_scratch(
                &command,
                &fixture.workspace,
                &environment,
                &scratch,
                || false,
                None,
            );
            assert_eq!(result.expect("ordinary command exit should be recorded").0, expected_exit);
            assert!(!scratch.path.exists(), "validation scratch must be gone after command completion");
            fixture.assert_durable_cache();
        }
    }

    #[cfg(unix)]
    #[test]
    fn timeout_and_cancellation_reap_process_group_before_scratch_cleanup() {
        let fixture = ScratchFixture::new();
        fs::create_dir_all(fixture.durable_home.join(".cargo/registry/cache"))
            .expect("Cargo cache fixture should be created");
        fs::write(fixture.durable_home.join(".cargo/registry/cache/keep"), b"preserve")
            .expect("Cargo cache fixture should be written");

        let timed_out = fixture.scratch("setup-timeout", 1, 0);
        let timeout_started = fixture.root.join("timeout-command-started");
        let mut timeout_environment = fixture.environment();
        timeout_environment.insert(
            "SCRATCH_TEST_STARTED_MARKER".to_string(),
            timeout_started.display().to_string(),
        );
        let timeout = run_worker_validation_command_with_scratch(
            "mkdir -p \"$CHARIOX_PROJECT_ENVIRONMENT_SETUP_SCRATCH_DIR/target\"; : > \"$SCRATCH_TEST_STARTED_MARKER\"; sleep 5",
            &fixture.workspace,
            &timeout_environment,
            &timed_out,
            || false,
            Some(Instant::now() + Duration::from_millis(300)),
        );
        assert!(timeout.unwrap_err().contains("timed out"));
        assert!(timeout_started.exists(), "timed command must start before its deadline");
        assert!(!timed_out.path.exists(), "timeout cleanup must follow process-group kill and reap");
        fixture.assert_durable_cache();

        let cancelled = fixture.scratch("setup-cancelled", 1, 0);
        let cancelled_started = fixture.root.join("cancelled-command-started");
        let mut cancellation_environment = fixture.environment();
        cancellation_environment.insert(
            "SCRATCH_TEST_STARTED_MARKER".to_string(),
            cancelled_started.display().to_string(),
        );
        let cancellation = run_worker_validation_command_with_scratch(
            "mkdir -p \"$CHARIOX_PROJECT_ENVIRONMENT_SETUP_SCRATCH_DIR/target\"; : > \"$SCRATCH_TEST_STARTED_MARKER\"; sleep 5",
            &fixture.workspace,
            &cancellation_environment,
            &cancelled,
            || cancelled_started.exists(),
            None,
        );
        assert!(cancellation.unwrap_err().contains("cancelled"));
        assert!(cancelled_started.exists(), "cancelled command must start before cancellation");
        assert!(!cancelled.path.exists(), "cancellation cleanup must follow process-group kill and reap");
        fixture.assert_durable_cache();
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_failure_converts_zero_command_exit_to_validation_failure() {
        let fixture = ScratchFixture::new();
        let scratch = fixture.scratch("setup-cleanup-failure", 1, 0);
        let command = [
            "printf foreign > \"${",
            VALIDATION_SCRATCH_DIR_ENV,
            "}/",
            OWNER_MARKER,
            "\"; exit 0",
        ]
        .concat();
        let result = run_worker_validation_command_with_scratch(
            &command,
            &fixture.workspace,
            &fixture.environment(),
            &scratch,
            || false,
            None,
        );
        assert!(result.unwrap_err().contains("cleanup failed"));
        assert!(scratch.path.exists(), "cleanup must not delete scratch with a changed owner marker");
    }
}
