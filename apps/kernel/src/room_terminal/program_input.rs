//! What a program opened with `command` reads on its standard input.
//!
//! The program's terminal carries its output; its input is a file, so a script of any bytes
//! reaches it whole and the program sees end-of-file where the input ends.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use super::service::refused;
use crate::error::DaemonError;

/// The most input one request carries inline; more goes in a file on the kernel.
pub const MAX_INLINE_INPUT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProgramInput {
    /// The bytes themselves, up to `MAX_INLINE_INPUT_BYTES`.
    Inline { data_base64: String },
    /// A file already on the kernel (uploaded with `files put`), read as it is.
    File { path: String },
}

/// The file the program reads, and whether the wrapper removes it once it holds it open.
pub(super) struct PreparedInput {
    pub path: PathBuf,
    pub spooled: bool,
}

impl PreparedInput {
    /// The wrapper that starts the program with this file as input, run as
    /// `sh -c <script> chariox-command <file> argv...`. A spooled copy is removed as soon as the
    /// program holds it open; a file the caller named stays.
    pub(super) fn wrapper_script(&self) -> &'static str {
        if self.spooled {
            "trap '' HUP; f=$1; shift; exec <\"$f\" || exit 126; rm -f -- \"$f\"; exec \"$@\""
        } else {
            "trap '' HUP; f=$1; shift; exec <\"$f\" || exit 126; exec \"$@\""
        }
    }
}

/// The wrapper for a program with no `stdin`: its input stays the terminal.
pub(super) const NO_INPUT_WRAPPER: &str = "trap '' HUP; exec \"$@\"";

pub(super) fn prepare(
    input: &ProgramInput,
    working_directory: &Path,
    terminal_id: &str,
) -> Result<PreparedInput, DaemonError> {
    match input {
        ProgramInput::Inline { data_base64 } => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data_base64)
                .map_err(|_| refused("stdin is not valid base64".to_string()))?;
            if bytes.len() > MAX_INLINE_INPUT_BYTES {
                return Err(refused(format!(
                    "stdin is {} bytes; one request carries at most {} bytes (64 KiB). Upload the \
                     file to the kernel (chariox files put) and pass it as a stdin file",
                    bytes.len(),
                    MAX_INLINE_INPUT_BYTES
                )));
            }
            spool(&bytes, terminal_id)
        }
        ProgramInput::File { path } => {
            let path = working_directory.join(path);
            let resolved = path.canonicalize().map_err(|failure| {
                refused(format!(
                    "stdin file `{}` cannot be read on this kernel: {failure}",
                    path.display()
                ))
            })?;
            if !resolved.is_file() {
                return Err(refused(format!(
                    "stdin file `{}` is not a file",
                    resolved.display()
                )));
            }
            Ok(PreparedInput {
                path: resolved,
                spooled: false,
            })
        }
    }
}

/// A private copy outside the kernel's state folder, readable by the confined program too.
#[cfg(unix)]
fn spool(bytes: &[u8], terminal_id: &str) -> Result<PreparedInput, DaemonError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = PathBuf::from("/tmp").join(format!("chariox-stdin-{terminal_id}"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|failure| refused(format!("could not stage stdin: {failure}")))?;
    if let Err(failure) = file.write_all(bytes) {
        let _ = std::fs::remove_file(&path);
        return Err(refused(format!("could not stage stdin: {failure}")));
    }
    Ok(PreparedInput {
        path,
        spooled: true,
    })
}

#[cfg(not(unix))]
fn spool(_: &[u8], _: &str) -> Result<PreparedInput, DaemonError> {
    Err(refused(
        "stdin is not supported on this platform".to_string(),
    ))
}
