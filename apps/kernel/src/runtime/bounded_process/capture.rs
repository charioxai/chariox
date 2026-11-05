//! Short-lived provider utilities own their output pipes and process cleanup.
//! Reads never block, and stdout plus stderr have one fixed memory budget.
use std::io::{self, Read};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

#[cfg(unix)]
#[path = "capture/unix.rs"]
mod platform;
#[cfg(windows)]
use crate::io::windows_pipe_process as platform;

pub(crate) const OUTPUT_LIMIT: usize = 1024 * 1024;
const READ_BUDGET: usize = 64 * 1024;

#[derive(Debug)]
pub(crate) enum CaptureError {
    Io(io::Error),
    TimedOut,
    OutputLimit,
}

impl From<io::Error> for CaptureError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub(crate) fn capture(mut command: Command, timeout: Duration) -> Result<Output, CaptureError> {
    capture_command_with_limit(&mut command, timeout, OUTPUT_LIMIT)
}

pub(super) fn capture_command(
    command: &mut Command,
    timeout: Duration,
) -> Result<Output, CaptureError> {
    capture_command_with_limit(command, timeout, 64 * 1024 * 1024)
}

fn capture_command_with_limit(
    command: &mut Command,
    timeout: Duration,
    output_limit: usize,
) -> Result<Output, CaptureError> {
    let deadline = Instant::now() + timeout;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let process = platform::Process::spawn(command)?;
    capture_process(process, deadline, output_limit)
}

pub(super) fn capture_child(mut child: Child, timeout: Duration) -> Result<Output, CaptureError> {
    drop(child.stdin.take());
    if child.stdout.is_none() || child.stderr.is_none() {
        crate::runtime::child_shutdown::stop(&mut child, "capture_missing_pipes")?;
        return Err(
            io::Error::other("child output capture requires piped stdout and stderr").into(),
        );
    }
    let deadline = Instant::now() + timeout;
    capture_process(
        platform::Process::from_child(child)?,
        deadline,
        64 * 1024 * 1024,
    )
}

fn capture_process(
    mut process: platform::Process,
    deadline: Instant,
    output_limit: usize,
) -> Result<Output, CaptureError> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut stdout_closed = false;
    let mut stderr_closed = false;
    loop {
        drain(
            process.child.stdout.as_mut().expect("captured stdout"),
            &mut stdout,
            stderr.len(),
            output_limit,
            &mut stdout_closed,
        )?;
        drain(
            process.child.stderr.as_mut().expect("captured stderr"),
            &mut stderr,
            stdout.len(),
            output_limit,
            &mut stderr_closed,
        )?;
        if let Some(status) = process.child.try_wait()? {
            while drain(
                process.child.stdout.as_mut().expect("captured stdout"),
                &mut stdout,
                stderr.len(),
                output_limit,
                &mut stdout_closed,
            )? {}
            while drain(
                process.child.stderr.as_mut().expect("captured stderr"),
                &mut stderr,
                stdout.len(),
                output_limit,
                &mut stderr_closed,
            )? {}
            if stdout_closed && stderr_closed {
                // No inherited writer exists. Normal commands such as ps must
                // not scan every host descriptor after every successful exit.
                process.finish_closed_output()?;
            } else {
                process.stop()?;
                while drain(
                    process.child.stdout.as_mut().expect("captured stdout"),
                    &mut stdout,
                    stderr.len(),
                    output_limit,
                    &mut stdout_closed,
                )? {}
                while drain(
                    process.child.stderr.as_mut().expect("captured stderr"),
                    &mut stderr,
                    stdout.len(),
                    output_limit,
                    &mut stderr_closed,
                )? {}
            }
            return Ok(Output {
                status,
                stdout,
                stderr,
            });
        }
        if Instant::now() >= deadline {
            return Err(CaptureError::TimedOut);
        }
        std::thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(5)),
        );
    }
}

// Return true only when this iteration used its read budget. Flooding either
// stream cannot starve the other stream or the process deadline.
fn drain<P: Read + platform::Pipe>(
    pipe: &mut P,
    bytes: &mut Vec<u8>,
    other_len: usize,
    output_limit: usize,
    closed: &mut bool,
) -> Result<bool, CaptureError> {
    let mut buffer = [0_u8; 8192];
    let mut read = 0;
    while read < READ_BUDGET {
        let available = platform::available(pipe)?;
        if available == 0 {
            return Ok(false);
        }
        let len = buffer.len().min(available);
        match pipe.read(&mut buffer[..len]) {
            Ok(0) => {
                *closed = true;
                return Ok(false);
            }
            Ok(len) => {
                if bytes.len() + other_len + len > output_limit {
                    return Err(CaptureError::OutputLimit);
                }
                bytes.extend_from_slice(&buffer[..len]);
                read += len;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(true)
}

#[cfg(test)]
#[path = "capture/tests.rs"]
mod tests;
