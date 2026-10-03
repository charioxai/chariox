//! Short-lived provider utilities own their output pipes and process cleanup.
//! Reads never block, and stdout plus stderr have one fixed memory budget.
use std::io::{self, Read};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

#[cfg(unix)]
#[path = "probe_capture/unix.rs"]
mod platform;
#[cfg(windows)]
use crate::io::windows_pipe_process as platform;

pub(super) const OUTPUT_LIMIT: usize = 1024 * 1024;
const READ_BUDGET: usize = 64 * 1024;

#[derive(Debug)]
pub(super) enum CaptureError {
    Io(io::Error),
    TimedOut,
    OutputLimit,
}

impl From<io::Error> for CaptureError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub(super) fn capture(mut command: Command, timeout: Duration) -> Result<Output, CaptureError> {
    let deadline = Instant::now() + timeout;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut process = platform::Process::spawn(&mut command)?;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    loop {
        drain(
            process.child.stdout.as_mut().expect("captured stdout"),
            &mut stdout,
            stderr.len(),
        )?;
        drain(
            process.child.stderr.as_mut().expect("captured stderr"),
            &mut stderr,
            stdout.len(),
        )?;
        if let Some(status) = process.child.try_wait()? {
            // A successful launcher can leave pipe writers behind. End them
            // before collecting the bytes already buffered in our pipes.
            process.stop()?;
            while drain(
                process.child.stdout.as_mut().expect("captured stdout"),
                &mut stdout,
                stderr.len(),
            )? {}
            while drain(
                process.child.stderr.as_mut().expect("captured stderr"),
                &mut stderr,
                stdout.len(),
            )? {}
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
            Ok(0) => return Ok(false),
            Ok(len) => {
                if bytes.len() + other_len + len > OUTPUT_LIMIT {
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
#[path = "probe_capture/tests.rs"]
mod tests;
