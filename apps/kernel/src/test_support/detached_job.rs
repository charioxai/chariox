//! Jobs a shell detaches into a session of its own, with no tool beyond what
//! every Unix host ships (macOS has no `setsid` binary).

/// Shell text that starts `sleep <seconds>` in a new session and writes that
/// process's PID to `pid_file` (shell syntax, quoted by the caller; `{pid}` in
/// it stands for that PID). The job
/// is forked off first, so its parent returns at once, as when a script
/// detaches it with `(setsid nohup job &)`.
pub(crate) fn detached_job(pid_file: &str, seconds: u32) -> String {
    session_script("os.fork() and os._exit(0);", pid_file, seconds)
}

/// Like `detached_job`, but the job stays a child of the shell that starts it,
/// as with `setsid job &` in a script (a background child is no group leader).
pub(crate) fn session_job(pid_file: &str, seconds: u32) -> String {
    session_script("", pid_file, seconds)
}

fn session_script(fork_off: &str, pid_file: &str, seconds: u32) -> String {
    format!(
        "python3 -c 'import os,sys;{fork_off}os.setsid();\
         p=str(os.getpid());f=open(sys.argv[1].replace(\"{{pid}}\",p),\"w\");f.write(p);f.close();\
         os.execvp(\"sleep\",[\"sleep\",sys.argv[2]])' {pid_file} {seconds}"
    )
}

/// Whether `pid` is a running process (a zombie only waits to be reaped).
pub(crate) fn process_alive(pid: i32) -> bool {
    if pid <= 1 {
        return false;
    }
    std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .is_ok_and(|output| {
            let status = String::from_utf8_lossy(&output.stdout);
            !status.trim().is_empty() && !status.trim().starts_with('Z')
        })
}
