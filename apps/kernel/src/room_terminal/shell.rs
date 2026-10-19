//! The user's login shell on this machine.

use std::ffi::CStr;
use std::path::Path;

use crate::error::DaemonError;

/// `$SHELL` when the kernel was started from a login environment, else the
/// shell the account database lists for the kernel's user (a service manager
/// does not set `SHELL`).
pub(super) fn login_shell() -> Result<String, DaemonError> {
    if let Ok(shell) = std::env::var("SHELL") {
        if Path::new(&shell).is_absolute() {
            return Ok(shell);
        }
    }
    account_shell().ok_or_else(|| DaemonError::LocalTransport {
        operation: "room terminal",
        message:
            "no login shell: $SHELL is unset and the account lists none; set SHELL for the kernel"
                .to_string(),
    })
}

#[cfg(unix)]
fn account_shell() -> Option<String> {
    let mut buffer = vec![0 as libc::c_char; 4096];
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let status = unsafe {
        libc::getpwuid_r(
            libc::geteuid(),
            &mut entry,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut found,
        )
    };
    if status != 0 || found.is_null() || entry.pw_shell.is_null() {
        return None;
    }
    let shell = unsafe { CStr::from_ptr(entry.pw_shell) }.to_str().ok()?;
    Path::new(shell).is_absolute().then(|| shell.to_string())
}

#[cfg(not(unix))]
fn account_shell() -> Option<String> {
    None
}
