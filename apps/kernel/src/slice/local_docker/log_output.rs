//! Bounded log tails from regular descriptors and concurrently drained commands.
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

pub(super) const MAX_LOG_BYTES: usize = 65536;
/// How long a running log command may take; a remote Docker context or a busy
/// engine answers in seconds. Shutdown of a command that missed it is bounded
/// separately by `child_shutdown`.
const LOG_COMMAND_DEADLINE: Duration = Duration::from_secs(30);

pub(super) fn tail_file(path: &Path, maximum: usize) -> io::Result<(String, bool)> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let mut file: File = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let truncated = metadata.len() > maximum as u64;
    if truncated {
        file.seek(SeekFrom::End(-(maximum as i64)))?;
    }
    let mut bytes = Vec::new();
    file.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
    let overflow = bytes.len() > maximum;
    let start = bytes.len().saturating_sub(maximum);
    Ok((
        bounded_text(&bytes[start..], maximum),
        truncated || overflow,
    ))
}

fn bounded_text(bytes: &[u8], maximum: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut start = text.len().saturating_sub(maximum);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

fn drain(mut reader: impl Read, maximum: usize) -> io::Result<(Vec<u8>, bool)> {
    let mut tail = VecDeque::with_capacity(maximum);
    let mut chunk = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        for byte in &chunk[..count] {
            if tail.len() == maximum {
                tail.pop_front();
                truncated = true;
            }
            tail.push_back(*byte);
        }
    }
    Ok((tail.into_iter().collect(), truncated))
}

pub(super) struct BoundedLogOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncated: bool,
}

impl BoundedLogOutput {
    pub(super) fn text(&self) -> String {
        let mut bytes = self.stdout.clone();
        bytes.extend_from_slice(&self.stderr);
        bounded_text(&bytes, MAX_LOG_BYTES)
    }
}

pub(super) trait LogCommandOutput {
    fn bounded_log_output(&mut self) -> io::Result<BoundedLogOutput> {
        self.bounded_log_output_within(LOG_COMMAND_DEADLINE)
    }
    fn bounded_log_output_within(&mut self, deadline: Duration) -> io::Result<BoundedLogOutput>;
}

/// Waits for a running command until `deadline`, then stops and reaps it.
fn wait_for_log_command(child: &mut Child, deadline: Duration) -> io::Result<ExitStatus> {
    let deadline = Instant::now() + deadline;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            crate::runtime::child_shutdown::stop(child, "docker_log_deadline")?;
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the log command did not finish before its deadline",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

impl LogCommandOutput for Command {
    fn bounded_log_output_within(&mut self, deadline: Duration) -> io::Result<BoundedLogOutput> {
        let mut child = self.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let output = std::thread::spawn(move || drain(stdout, MAX_LOG_BYTES / 2));
        let diagnostic = std::thread::spawn(move || drain(stderr, MAX_LOG_BYTES / 2));
        let status = wait_for_log_command(&mut child, deadline);
        let (stdout, out_truncated) = output.join().map_err(|_| io::ErrorKind::Other)??;
        let (stderr, err_truncated) = diagnostic.join().map_err(|_| io::ErrorKind::Other)??;
        Ok(BoundedLogOutput {
            status: status?,
            stdout,
            stderr,
            truncated: out_truncated || err_truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_stdout_and_stderr_are_bounded_while_both_are_drained() {
        let output = crate::process_spawn::command("sh")
            .args([
                "-c",
                "head -c 2097152 /dev/zero; head -c 2097152 /dev/zero >&2",
            ])
            .bounded_log_output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.truncated);
        assert!(output.stdout.len() + output.stderr.len() <= MAX_LOG_BYTES);
    }

    #[test]
    fn a_log_command_slower_than_the_reap_limit_still_returns_its_tail() {
        let started = Instant::now();
        let output = crate::process_spawn::command("sh")
            .args(["-c", "sleep 2; echo tail"])
            .bounded_log_output()
            .unwrap();
        assert!(started.elapsed() >= Duration::from_secs(2));
        assert!(output.status.success());
        assert_eq!(output.text().trim(), "tail");
    }

    #[test]
    fn a_hung_log_command_is_stopped_and_reaped_at_the_deadline() {
        let started = Instant::now();
        let mut command = crate::process_spawn::command("sleep");
        command.arg("300");
        let error = match command.bounded_log_output_within(Duration::from_millis(300)) {
            Err(error) => error,
            Ok(_) => panic!("a hung command cannot succeed"),
        };
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_hung_log_command_process_is_gone_after_the_deadline() {
        let pid_file = std::env::temp_dir().join(format!("log-hang-{}", std::process::id()));
        let script = format!("echo $$ > {}; exec sleep 300", pid_file.display());
        let mut command = crate::process_spawn::command("sh");
        command.args(["-c", &script]);
        let _ = command.bounded_log_output_within(Duration::from_millis(500));
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        let _ = std::fs::remove_file(&pid_file);
        assert!(
            !std::path::Path::new(&format!("/proc/{}", pid.trim())).exists(),
            "the timed-out log command must be reaped"
        );
    }
}
