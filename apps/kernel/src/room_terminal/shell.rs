//! The user's login shell on this machine.

use std::ffi::CStr;
use std::path::Path;

use crate::error::DaemonError;

const BASH_LOCATIONS: &[&str] = &["/bin/bash", "/usr/bin/bash", "/usr/local/bin/bash"];

/// `$SHELL` when the kernel was started from a login environment. Else the
/// shell the account database lists for the kernel's user (a service manager
/// does not set `SHELL`), except that a bare POSIX `sh` gives way to bash
/// when it is installed: a slice's account lists `/bin/sh`, whose login
/// profile would not carry the user's aliases.
pub(super) fn login_shell() -> Result<String, DaemonError> {
    choose_login_shell(
        std::env::var("SHELL").ok(),
        account_shell(),
        BASH_LOCATIONS
            .iter()
            .find(|path| Path::new(path).is_file())
            .map(|path| path.to_string()),
    )
    .ok_or_else(|| DaemonError::LocalTransport {
        operation: "room terminal",
        message:
            "no login shell: $SHELL is unset and the account lists none; set SHELL for the kernel"
                .to_string(),
    })
}

fn choose_login_shell(
    environment: Option<String>,
    account: Option<String>,
    bash: Option<String>,
) -> Option<String> {
    if let Some(shell) = environment.filter(|shell| Path::new(shell).is_absolute()) {
        return Some(shell);
    }
    let is_bare_sh = account.as_deref().is_none_or(|shell| {
        matches!(
            Path::new(shell).file_name().and_then(|name| name.to_str()),
            Some("sh" | "dash" | "ash")
        )
    });
    match (is_bare_sh, bash) {
        (true, Some(bash)) => Some(bash),
        _ => account,
    }
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

#[cfg(test)]
mod tests {
    use super::choose_login_shell;

    fn some(path: &str) -> Option<String> {
        Some(path.to_string())
    }

    #[test]
    fn shell_variable_wins_and_bash_replaces_a_bare_sh_account() {
        assert_eq!(
            choose_login_shell(some("/usr/bin/zsh"), some("/bin/sh"), some("/bin/bash")),
            some("/usr/bin/zsh")
        );
        // A slice: no $SHELL, account shell /bin/sh.
        assert_eq!(
            choose_login_shell(None, some("/bin/sh"), some("/bin/bash")),
            some("/bin/bash")
        );
        assert_eq!(
            choose_login_shell(None, some("/bin/sh"), None),
            some("/bin/sh")
        );
        assert_eq!(
            choose_login_shell(None, some("/usr/bin/fish"), some("/bin/bash")),
            some("/usr/bin/fish")
        );
        assert_eq!(
            choose_login_shell(Some("bash".into()), None, some("/bin/bash")),
            some("/bin/bash")
        );
        assert_eq!(choose_login_shell(None, None, None), None);
    }
}
