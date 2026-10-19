//! Background jobs of a terminal's shell live in their own process groups, so
//! stopping the shell and its foreground group leaves them behind. Everything
//! still in the shell's session ends with the terminal, by exact PID.

/// Who belongs to a terminal's shell, captured while the shell still runs.
pub(super) struct SessionSweep {
    leader: i32,
}

impl SessionSweep {
    pub(super) fn capture(session_leader: i32) -> Self {
        Self {
            leader: session_leader,
        }
    }

    pub(super) fn stop(&self) {
        #[cfg(target_os = "linux")]
        stop_session(self.leader);
        #[cfg(not(target_os = "linux"))]
        stop_ps_session(self.leader);
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

// A session ID outlives its controlling terminal and leader. A tty selector
// would miss disowned jobs after the shell exits and detaches that terminal.
#[cfg(any(not(target_os = "linux"), test))]
fn stop_ps_session(session_leader: i32) {
    if session_leader <= 1 {
        return;
    }
    let Ok(output) = std::process::Command::new("ps")
        .args(["-A", "-o", "pid="])
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }
    for pid in session_members(
        &String::from_utf8_lossy(&output.stdout),
        session_leader,
        |pid| {
            // SAFETY: query a positive PID; this does not signal it.
            let session = unsafe { libc::getsid(pid) };
            (session > 1).then_some(session)
        },
    ) {
        // SAFETY: a positive PID whose session still belongs to this terminal.
        let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
    }
}

#[cfg(any(not(target_os = "linux"), test))]
fn session_members(
    processes: &str,
    leader: i32,
    session_of: impl Fn(i32) -> Option<i32>,
) -> Vec<i32> {
    if leader <= 1 {
        return Vec::new();
    }
    processes
        .split_whitespace()
        .filter_map(|pid| pid.parse::<i32>().ok())
        .filter(|pid| *pid > 1 && session_of(*pid) == Some(leader))
        .collect()
}

/// Field 6 of `/proc/<pid>/stat`; the command name (field 2) may hold spaces
/// and parentheses, so fields are counted after its closing parenthesis.
#[cfg(target_os = "linux")]
fn session_of(pid: i32) -> Option<i32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_command = &stat[stat.rfind(')')? + 1..];
    after_command.split_whitespace().nth(3)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ps_sweep_stops_a_job_after_its_session_leader_exits() {
        use std::os::unix::process::CommandExt;
        use std::time::{Duration, Instant};
        let root =
            std::env::temp_dir().join(format!("terminal-detached-job-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("job.pid");
        let mut command = std::process::Command::new("/bin/bash");
        command
            .args([
                "--noprofile",
                "--norc",
                "-c",
                "nohup sleep 8884 >/dev/null 2>&1 & echo $! > \"$1\"",
                "bash",
            ])
            .arg(&file);
        // SAFETY: setsid is async-signal-safe and touches only this child.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut shell = command.spawn().unwrap();
        let leader = i32::try_from(shell.id()).unwrap();
        assert!(shell.wait().unwrap().success());
        let pid = std::fs::read_to_string(&file)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        assert!(pid > 1 && leader > 1);
        let alive = || {
            std::process::Command::new("ps")
                .args(["-o", "stat=", "-p", &pid.to_string()])
                .output()
                .is_ok_and(|output| {
                    let status = String::from_utf8_lossy(&output.stdout);
                    !status.trim().is_empty() && !status.trim().starts_with('Z')
                })
        };
        assert!(alive());
        stop_ps_session(leader);
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let survived = alive();
        // Exact-PID cleanup also runs if selection was broken (the RED case).
        if survived && pid > 1 {
            let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        std::fs::remove_dir_all(root).unwrap();
        assert!(
            !survived,
            "the detached job survived its terminal session sweep"
        );
    }

    #[test]
    fn detached_jobs_are_selected_by_session_without_the_leader_or_a_tty() {
        let listed = " 0\n1\n-1\nnot-a-pid\n42\n43\n44\n";
        // Leader 40 has already exited; job 42 has no controlling tty.
        let lookup = |pid| match pid {
            42 => Some(40),
            43 => Some(43),
            _ => None,
        };
        assert_eq!(session_members(listed, 40, lookup), vec![42]);
        for invalid in [-1, 0, 1] {
            assert!(session_members(listed, invalid, lookup).is_empty());
        }
    }
}
