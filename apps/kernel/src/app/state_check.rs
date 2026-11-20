use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use crate::config::state_check::StateCheckTarget;
use crate::durable_state::DurableKernelStateStore;
use rusqlite::{Connection, OpenFlags};

use super::durable_runtime_state::check_durable_replay;

/// Check startup replay without initializing the daemon.
pub fn check_state(home: Option<&Path>) -> i32 {
    match run_check(home) {
        Ok(()) => {
            println!("Kernel state check passed.");
            0
        }
        Err(CheckError::Unavailable(reason)) => {
            eprintln!("Could not check kernel state. {reason}\nCorrect the reported problem and retry before switching.");
            2
        }
        Err(CheckError::Failed(reason)) => {
            eprintln!(
                "Kernel state check failed. {reason}\nKeep the current kernel and share this result with the release maintainer."
            );
            1
        }
    }
}

enum CheckError {
    Unavailable(String),
    Failed(String),
}

fn run_check(home: Option<&Path>) -> Result<(), CheckError> {
    let target = StateCheckTarget::resolve(home).map_err(CheckError::Unavailable)?;
    let source = target.database.canonicalize().map_err(|error| {
        CheckError::Unavailable(format!(
            "Cannot open state database {} ({error}).",
            target.database.display()
        ))
    })?;
    let scratch = scratch_directory(&target.home)?;
    let database = scratch.path().join("state.db");
    copy_database_file(&source, &database, false)?;
    // Opening SQLite on the source could create or change shared-memory files.
    // Recover and migrate only the private copy, including uncheckpointed data.
    for suffix in ["-wal", "-journal"] {
        copy_database_file(
            &with_suffix(&source, suffix),
            &with_suffix(&database, suffix),
            true,
        )?;
    }
    require_state_schema(&database)?;
    let store = DurableKernelStateStore::open(database)
        .map_err(|error| CheckError::Failed(error.to_string()))?;
    check_durable_replay(&target.daemon_id, &store)
        .map_err(|error| CheckError::Failed(error.to_string()))
}

fn scratch_directory(home: &Path) -> Result<tempfile::TempDir, CheckError> {
    let temporary_error = |error: io::Error| {
        CheckError::Unavailable(format!(
            "Cannot prepare temporary storage ({error}). Set TMPDIR to a writable directory outside the kernel home."
        ))
    };
    let base = std::env::temp_dir()
        .canonicalize()
        .map_err(temporary_error)?;
    let home = home.canonicalize().map_err(|error| {
        CheckError::Unavailable(format!(
            "Cannot access kernel home {} ({error}).",
            home.display()
        ))
    })?;
    if base.starts_with(&home) {
        return Err(CheckError::Unavailable(
            "Set TMPDIR to a directory outside the kernel home.".into(),
        ));
    }
    tempfile::Builder::new()
        .prefix("chariox-check-state-")
        .tempdir_in(base)
        .map_err(temporary_error)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path);
    name.push(suffix);
    name.into()
}

fn copy_database_file(source: &Path, destination: &Path, optional: bool) -> Result<(), CheckError> {
    let read_error = |error: io::Error| {
        CheckError::Unavailable(format!(
            "Cannot read state file {} ({error}).",
            source.display()
        ))
    };
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let mut source_file = match options.open(source) {
        Ok(file) => file,
        Err(error) if optional && error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(read_error(error)),
    };
    if !source_file.metadata().map_err(read_error)?.is_file() {
        return Err(CheckError::Unavailable(format!(
            "State file {} is not a regular file.",
            source.display()
        )));
    }
    let write_error = |error: io::Error| {
        CheckError::Unavailable(format!(
            "Cannot copy state into temporary storage ({error})."
        ))
    };
    let mut destination = File::create(destination).map_err(write_error)?;
    io::copy(&mut source_file, &mut destination).map_err(write_error)?;
    Ok(())
}

fn require_state_schema(database: &Path) -> Result<(), CheckError> {
    let database_error = |error: rusqlite::Error| {
        CheckError::Failed(format!("Cannot read the kernel state database ({error})."))
    };
    // This connection may recover a rollback journal, so open just the
    // already-created scratch copy read/write.
    let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(database_error)?;
    let has_events: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'durable_state_events')",
            [],
            |row| row.get(0),
        )
        .map_err(database_error)?;
    if !has_events {
        return Err(CheckError::Failed(
            "The database has no kernel state event table.".into(),
        ));
    }
    Ok(())
}
