//! One run of a watcher's script: its own session with a claim, so a later kernel can end what
//! it left; bounded time, output and line length; its reports handed over one line at a time.
use super::contract::{parse_line, Input, Report};
use crate::local::WatcherManifest;
use crate::runtime::bounded_process::AsyncChildOwner;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;

const MAX_LINE_BYTES: usize = 64 * 1024;
/// Output a run may write before what it reported is committed: a poll's whole run, a stream's
/// lines between two cursor lines. The kernel holds that much, so more fails the run.
const MAX_UNCOMMITTED_BYTES: usize = 256 * 1024;
/// The end of the error output kept for the agent that repairs the script.
const STDERR_TAIL_BYTES: usize = 8 * 1024;
const STDERR_DRAIN_GRACE: Duration = Duration::from_secs(2);

pub(crate) struct RunSpec {
    pub dir: PathBuf,
    pub manifest: WatcherManifest,
    pub input: Input,
    pub claim: String,
    /// The longest a poll or dry run may take; a stream runs until it ends or is stopped, and
    /// commits at each cursor line.
    pub limit: Option<Duration>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum RunEnd {
    /// Exited with 0.
    Finished,
    Failed {
        reason: String,
        output: String,
    },
    Stopped,
}

/// Runs the script once. `report` gets each line in order; an error it returns fails the run.
pub(crate) async fn run(
    spec: RunSpec,
    mut stop: watch::Receiver<bool>,
    mut report: impl FnMut(Report) -> Result<(), String>,
) -> RunEnd {
    let failed = |reason: String, output: String| RunEnd::Failed { reason, output };
    let input = match serde_json::to_string(&spec.input) {
        Ok(input) => input + "\n",
        Err(e) => return failed(format!("input could not be written: {e}"), String::new()),
    };
    let mut command = crate::process_spawn::async_command(&spec.manifest.command[0]);
    command
        .args(&spec.manifest.command[1..])
        .env(
            crate::runtime::owned_process_signals::CLAIM_VARIABLE,
            &spec.claim,
        )
        .current_dir(&spec.dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // Its own session, so a later kernel can tell the script's processes from others.
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            return failed(
                format!("{} could not start: {e}", spec.manifest.command[0]),
                String::new(),
            )
        }
    };
    let pid = child.id();
    if let Some(mut stdin) = child.stdin.take() {
        // A cursor can be larger than a pipe holds; a script that never reads must not stall.
        tokio::spawn(async move {
            let _ = stdin.write_all(input.as_bytes()).await;
        });
    }
    let stderr = child.stderr.take().map(|mut pipe| {
        tokio::spawn(async move {
            let mut tail = Vec::new();
            let mut read = [0u8; 4096];
            while let Ok(count) = pipe.read(&mut read).await {
                if count == 0 {
                    break;
                }
                tail.extend_from_slice(&read[..count]);
                let over = tail.len().saturating_sub(STDERR_TAIL_BYTES);
                tail.drain(..over);
            }
            String::from_utf8_lossy(&tail).into_owned()
        })
    });
    let deadline = spec.limit.map(|limit| tokio::time::Instant::now() + limit);
    let outcome = read_reports(child.stdout.take(), &spec, deadline, &mut stop, &mut report).await;
    let outcome = match outcome {
        // A script that closed its output can still be running: a stop ends it all the same.
        Ok(()) => tokio::select! {
            ended = child.supervised_wait() => match ended {
                Ok(status) if status.success() => Ok(()),
                Ok(status) => Err(Some(format!("exited with {status}"))),
                Err(e) => Err(Some(format!("could not be awaited: {e}"))),
            },
            () = stopped(&mut stop) => Err(None),
            () = until(deadline) => Err(Some(timed_out(&spec))),
        },
        Err(reason) => Err(reason),
    };
    if outcome.is_err() {
        if let Some(pid) = pid {
            crate::runtime::process_health::terminate_process_tree(pid);
        }
        let _ = child.supervised_wait().await;
    }
    // What the script left in the background ends with its run.
    let _ = crate::runtime::owned_process_signals::OwnedProcessSignals::end_claimed_tree(
        &spec.claim,
        None,
    );
    let output = match stderr {
        Some(reader) => {
            let abort = reader.abort_handle();
            match tokio::time::timeout(STDERR_DRAIN_GRACE, reader).await {
                Ok(Ok(tail)) => tail,
                _ => {
                    abort.abort();
                    String::new()
                }
            }
        }
        None => String::new(),
    };
    match outcome {
        Ok(()) => RunEnd::Finished,
        Err(None) => RunEnd::Stopped,
        Err(Some(reason)) => failed(reason, output),
    }
}

async fn stopped(stop: &mut watch::Receiver<bool>) {
    if !*stop.borrow() {
        let _ = stop.changed().await;
    }
}

async fn until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

fn timed_out(spec: &RunSpec) -> String {
    format!(
        "took longer than {} s",
        spec.limit.unwrap_or_default().as_secs()
    )
}

/// Ok when standard output ended; Err(None) when stopped, Err(Some(reason)) when it failed.
async fn read_reports(
    stdout: Option<tokio::process::ChildStdout>,
    spec: &RunSpec,
    deadline: Option<tokio::time::Instant>,
    stop: &mut watch::Receiver<bool>,
    report: &mut impl FnMut(Report) -> Result<(), String>,
) -> Result<(), Option<String>> {
    let Some(mut stdout) = stdout else {
        return Ok(());
    };
    let mut pending = Vec::new();
    let mut uncommitted = Uncommitted::default();
    let mut read = vec![0u8; 16 * 1024];
    loop {
        if *stop.borrow() {
            return Err(None);
        }
        let count = tokio::select! {
            read = stdout.read(&mut read) => read.map_err(|e| Some(format!("output could not be read: {e}")))?,
            _ = stop.changed() => return Err(None),
            () = until(deadline) => return Err(Some(timed_out(spec))),
        };
        if count == 0 {
            if !pending.iter().all(u8::is_ascii_whitespace) {
                uncommitted.line(spec, &pending, report)?;
            }
            return Ok(());
        }
        pending.extend_from_slice(&read[..count]);
        while let Some(end) = pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            uncommitted.line(spec, &line, report)?;
        }
        if pending.len() > MAX_LINE_BYTES {
            return Err(Some(format!("wrote a line over {MAX_LINE_BYTES} bytes")));
        }
    }
}

/// The bytes of the lines read since what the run reported was last committed.
#[derive(Default)]
struct Uncommitted(usize);

impl Uncommitted {
    /// Hands over one line, ended by its newline or by the end of the output.
    fn line(
        &mut self,
        spec: &RunSpec,
        line: &[u8],
        report: &mut impl FnMut(Report) -> Result<(), String>,
    ) -> Result<(), Option<String>> {
        self.0 += line.len();
        if self.0 > MAX_UNCOMMITTED_BYTES {
            return Err(Some(match spec.limit {
                Some(_) => format!("wrote more than {MAX_UNCOMMITTED_BYTES} bytes"),
                None => format!(
                    "wrote more than {MAX_UNCOMMITTED_BYTES} bytes without a cursor line; a stream writes a cursor line after each batch of events so the kernel can commit them"
                ),
            }));
        }
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        if handle_line(spec, line, report)? && spec.limit.is_none() {
            self.0 = 0;
        }
        Ok(())
    }
}

/// Ok(true) when the line was a cursor.
fn handle_line(
    spec: &RunSpec,
    line: &[u8],
    report: &mut impl FnMut(Report) -> Result<(), String>,
) -> Result<bool, Option<String>> {
    if line.len() > MAX_LINE_BYTES {
        return Err(Some(format!("wrote a line over {MAX_LINE_BYTES} bytes")));
    }
    let line =
        std::str::from_utf8(line).map_err(|_| Some("wrote a line that is not UTF-8".into()))?;
    if line.trim().is_empty() {
        return Ok(false);
    }
    let parsed = parse_line(&spec.manifest, line.trim()).map_err(Some)?;
    let cursor = matches!(parsed, Report::Cursor(_));
    report(parsed).map_err(Some)?;
    Ok(cursor)
}
