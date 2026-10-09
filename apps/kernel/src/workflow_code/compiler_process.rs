//! Bounded serialized compiler I/O and supervision, shared by isolation backends.
use super::*;
use std::io::Read;
#[cfg(target_os = "macos")]
use std::process::Child;
use std::process::Output;
use std::time::Instant;

fn error(message: impl Into<String>) -> crate::DaemonError {
    crate::DaemonError::LocalTransport {
        operation: "workflow_code.compile",
        message: message.into(),
    }
}

fn read_output(mut stream: impl Read, maximum: u64) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    (&mut stream).take(maximum + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "compiler output exceeds configured limit",
        ));
    }
    Ok(bytes)
}

/// Like Linux's address-space allowance, sampled memory gets headroom above the
/// V8 heap cap for Node's resident baseline and native allocations.
#[cfg(target_os = "macos")]
const NATIVE_MEMORY_ALLOWANCE: u64 = 256 * 1024 * 1024;

#[cfg(target_os = "macos")]
fn check_memory(child: &mut Child, maximum: u64) -> Result<(), crate::DaemonError> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v0>::zeroed();
    let status = unsafe {
        libc::proc_pid_rusage(
            child.id() as i32,
            libc::RUSAGE_INFO_V0,
            usage.as_mut_ptr().cast(),
        )
    };
    if status != 0 {
        // A child can exit between the wait and the sample. Any live child whose
        // usage cannot be measured is refused, rather than left unbounded.
        if child
            .try_wait()
            .map_err(io_error("workflow_code.compile"))?
            .is_some()
        {
            return Ok(());
        }
        return Err(error(
            "cannot enforce compiler memory limit on this macOS host",
        ));
    }
    let usage = unsafe { usage.assume_init() };
    if usage.ri_resident_size.max(usage.ri_phys_footprint) > maximum {
        return Err(error(
            "workflow-code script exceeded configured memory limit",
        ));
    }
    Ok(())
}

pub(super) fn run(
    command: &mut Command,
    input: Vec<u8>,
    limits: &WorkflowCodeLimitsConfig,
    deadline: Instant,
) -> Result<Output, crate::DaemonError> {
    if Instant::now() >= deadline {
        return Err(error("workflow-code script exceeded configured timeout"));
    }
    let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|cause| error(format!("failed to start isolated Node workflow-code compiler; verify the host isolation backend: {cause}")))?;
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    // Drain pipes while the child runs; neither large serialized input nor output
    // may block the parent past the total deadline.
    let maximum = 16 * 1024 * 1024
        + u64::from(limits.max_schema_bytes)
        + u64::from(limits.max_generated_prompt_bytes);
    std::thread::scope(|scope| {
        let input_writer = scope.spawn(move || stdin.write_all(&input));
        let output_reader = scope.spawn(move || read_output(stdout, maximum));
        let error_reader = scope.spawn(move || read_output(stderr, 64 * 1024));
        let result = (|| loop {
            #[cfg(target_os = "macos")]
            check_memory(
                &mut child,
                limits
                    .script_memory_bytes
                    .saturating_add(NATIVE_MEMORY_ALLOWANCE),
            )?;
            if let Some(status) = child
                .try_wait()
                .map_err(io_error("workflow_code.compile"))?
            {
                return Ok(status);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(error("workflow-code script exceeded configured timeout"));
            }
            child
                .wait_timeout(remaining.min(Duration::from_millis(5)))
                .map_err(io_error("workflow_code.compile"))?;
        })();
        if result.is_err() && child.id() > 1 {
            let _ = child.kill();
            let _ = child.wait();
        }
        let written = input_writer
            .join()
            .map_err(|_| error("compiler input thread failed"))?;
        let stdout = output_reader
            .join()
            .map_err(|_| error("compiler output thread failed"))?;
        let stderr = error_reader
            .join()
            .map_err(|_| error("compiler error thread failed"))?;
        let status = result?;
        let stdout = stdout.map_err(io_error("workflow_code.compile"))?;
        let stderr = stderr.map_err(io_error("workflow_code.compile"))?;
        // On launch failure, report the isolated process error instead of its
        // secondary closed-stdin error.
        if status.success() {
            written.map_err(io_error("workflow_code.compile"))?;
        }
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    })
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn macos_compiler_memory_monitor_bounds_native_allocations() {
        let node = discover_workflow_code_node_path().unwrap();
        let limits = WorkflowCodeLimitsConfig {
            script_memory_bytes: 64 * 1024 * 1024,
            script_timeout_ms: 5_000,
            ..WorkflowCodeLimitsConfig::default()
        };
        let mut command =
            super::super::compiler_isolation::compiler_command(&node, &limits).unwrap();
        let diagnostic = include_str!("compiler_native_memory_test.mjs");
        command.args([
            "--disable-wasm-trap-handler",
            "--input-type=module",
            "-e",
            &diagnostic,
        ]);
        let result = run(
            &mut command,
            Vec::new(),
            &limits,
            Instant::now() + Duration::from_secs(5),
        );
        assert!(result.unwrap_err().to_string().contains("memory limit"));
    }
}
