//! Bounded offline compiler/identity subprocesses owned by the kernel.
use super::*;
use std::{
    process::Child,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

struct OwnedChild(Option<Child>);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            if child.id() > 1 {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}
pub(super) fn run(
    program: &Path,
    args: &[&str],
    input: &[u8],
    limit: usize,
) -> Result<Vec<u8>, AotError> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut owned = OwnedChild(Some(child));
    let child = owned.0.as_mut().unwrap();
    let stdout = child.stdout.take().ok_or(AotError::Incompatible)?;
    let stderr = child.stderr.take().ok_or(AotError::Incompatible)?;
    let exceeded = Arc::new(AtomicBool::new(false));
    let overflow = exceeded.clone();
    let output = thread::Builder::new()
        .name("app-compiler-output".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            stdout.take(limit as u64 + 1).read_to_end(&mut bytes)?;
            overflow.store(bytes.len() > limit, Ordering::Release);
            Ok::<_, std::io::Error>(bytes)
        })?;
    let errors = thread::Builder::new()
        .name("app-compiler-errors".into())
        .spawn(move || {
            let mut stderr = stderr;
            let mut bytes = Vec::new();
            Read::by_ref(&mut stderr)
                .take(4096)
                .read_to_end(&mut bytes)?;
            std::io::copy(&mut stderr, &mut std::io::sink())?;
            Ok::<_, std::io::Error>(bytes)
        })?;
    let stdin = child.stdin.take().ok_or(AotError::Incompatible)?;
    let input = input.to_owned();
    let deadline = Instant::now() + Duration::from_secs(120);
    let writer = thread::Builder::new()
        .name("app-compiler-input".into())
        .spawn(move || {
            let mut stdin = stdin;
            stdin.write_all(&input)
        })?;
    let status = loop {
        if exceeded.load(Ordering::Acquire) {
            return Err(AotError::Limit);
        }
        if Instant::now() >= deadline {
            return Err(AotError::Compile(
                "offline compilation timed out; retry with the current runtime".into(),
            ));
        }
        if let Some(status) = owned.0.as_mut().unwrap().try_wait()? {
            // try_wait reaped this PID. The drop guard must never signal it.
            owned.0.take();
            break status;
        }
        thread::sleep(Duration::from_millis(20));
    };
    writer
        .join()
        .map_err(|_| AotError::Compile("compiler input writer stopped".into()))??;
    let bytes = output
        .join()
        .map_err(|_| AotError::Compile("compiler output reader stopped".into()))??;
    let errors = errors
        .join()
        .map_err(|_| AotError::Compile("compiler error reader stopped".into()))??;
    if bytes.len() > limit {
        return Err(AotError::Limit);
    }
    if !status.success() {
        return Err(AotError::Compile(String::from_utf8_lossy(&errors).into()));
    }
    Ok(bytes)
}
