//! Background jobs of a terminal's shell live in their own process groups, so
//! stopping the shell and its foreground group leaves them behind. Everything
//! still in the shell's session ends with the terminal, by exact PID.

/// Stop every process in the session led by `session_leader`.
#[cfg(target_os = "linux")]
pub(super) fn stop_session(session_leader: i32) {
    if session_leader <= 1 {
        return;
    }
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
            .filter(|pid| *pid > 1)
        else {
            continue;
        };
        if session_of(pid) == Some(session_leader) {
            // SAFETY: a positive PID read from /proc moments ago; never a group.
            let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn stop_session(_session_leader: i32) {}

/// Field 6 of `/proc/<pid>/stat`; the command name (field 2) may hold spaces
/// and parentheses, so fields are counted after its closing parenthesis.
#[cfg(target_os = "linux")]
fn session_of(pid: i32) -> Option<i32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_command = &stat[stat.rfind(')')? + 1..];
    after_command.split_whitespace().nth(3)?.parse().ok()
}
