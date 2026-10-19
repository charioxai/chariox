//! Background jobs of a terminal's shell live in their own process groups, so
//! stopping the shell and its foreground group leaves them behind. Everything
//! still in the shell's session ends with the terminal, by exact PID.

/// Who belongs to a terminal's shell, captured while the shell still runs.
pub(super) struct SessionSweep {
    leader: i32,
    /// Hosts without `/proc` find session members by their controlling
    /// terminal, which only the live shell can name.
    #[cfg(not(target_os = "linux"))]
    tty: Option<String>,
}

impl SessionSweep {
    pub(super) fn capture(session_leader: i32) -> Self {
        Self {
            leader: session_leader,
            #[cfg(not(target_os = "linux"))]
            tty: controlling_tty(session_leader),
        }
    }

    pub(super) fn stop(&self) {
        #[cfg(target_os = "linux")]
        stop_session(self.leader);
        #[cfg(not(target_os = "linux"))]
        if let Some(tty) = &self.tty {
            stop_tty_members(self.leader, tty);
        }
    }
}

/// Stop every process in the session led by `session_leader`.
#[cfg(target_os = "linux")]
fn stop_session(session_leader: i32) {
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

/// `ps` prints `??` (or nothing) for a process without a terminal.
#[cfg(not(target_os = "linux"))]
fn controlling_tty(pid: i32) -> Option<String> {
    let output = std::process::Command::new("ps")
        .args(["-o", "tty=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let tty = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!tty.is_empty() && !tty.starts_with('?')).then_some(tty)
}

#[cfg(not(target_os = "linux"))]
fn stop_tty_members(session_leader: i32, tty: &str) {
    if session_leader <= 1 {
        return;
    }
    let Ok(output) = std::process::Command::new("ps")
        .args(["-t", tty, "-o", "pid="])
        .output()
    else {
        return;
    };
    for pid in String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter_map(|pid| pid.parse::<i32>().ok())
        .filter(|pid| *pid > 1)
    {
        // SAFETY: a positive PID `ps` listed for this terminal moments ago.
        let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
    }
}

/// Field 6 of `/proc/<pid>/stat`; the command name (field 2) may hold spaces
/// and parentheses, so fields are counted after its closing parenthesis.
#[cfg(target_os = "linux")]
fn session_of(pid: i32) -> Option<i32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_command = &stat[stat.rfind(')')? + 1..];
    after_command.split_whitespace().nth(3)?.parse().ok()
}
