//! Workflow evaluations reuse the App worker's verified executable, clean
//! spawn, continuation, cancellation and exact-owned-PID reap. No installation
//! identity, App private data or broker is created for an evaluation.
use super::{
    record::PreparedLaunch,
    wasm::{self, WasmRuntime},
    PreparedWorker, ResourceDomain, WorkerError, WorkerLimits, WorkerProcess,
};
use crate::{
    private_fs::Dir,
    workflow_gate::{self as contract, Failure, Launch, Report, Response},
};
use std::{
    ffi::CString,
    fs::File,
    io::Read,
    os::unix::net::UnixStream,
    path::Path,
    time::{Duration, Instant},
};

struct GateDomain {
    descriptors: [File; 4],
    _runtime: WasmRuntime,
    process: wasm::domain::OwnedProcess,
}
impl ResourceDomain for GateDomain {
    fn setup_descriptors(&self) -> &[File] {
        &self.descriptors
    }
    fn verify_before_continue(&mut self, pid: libc::pid_t) -> Result<(), WorkerError> {
        self.process.verify(pid)
    }
    fn check_running(&mut self, pid: libc::pid_t, now: Instant) -> Result<(), WorkerError> {
        self.process.check(pid, now)
    }
    fn terminate(&mut self, pid: libc::pid_t) {
        self.process.terminate(pid);
    }
    fn reap_domain_blocking(&mut self) {}
}
impl PreparedWorker {
    /// All paths/identities come from the authority. Uploaded portable bytes
    /// remain untrusted until the contained worker has admitted them.
    fn gate(
        runtime: WasmRuntime,
        launch: &Launch,
        component: &[u8],
        scratch: &Path,
        deadline: Instant,
    ) -> Result<Self, Failure> {
        launch.evaluation.validate()?;
        if component.len() > 2 * 1024 * 1024
            || contract::sha256(component) != launch.module_sha256
            || launch.entry_id != launch.evaluation.input.entry_id
        {
            return Err(Failure::GateInvalidInput);
        }
        let config = serde_json::to_vec(launch).map_err(|_| Failure::GateInvalidInput)?;
        if config.len() > contract::INPUT_BYTES + 1024 {
            return Err(Failure::GateInputLimit);
        }
        let root = Dir::open_private(scratch).map_err(|_| Failure::GateRuntimeUnavailable)?;
        let record = PreparedLaunch::Gate {
            generation: launch.generation.clone(),
            entry_id: launch.entry_id.clone(),
            module_sha256: launch.module_sha256.clone(),
        };
        record.encode().map_err(|_| Failure::GateInvalidInput)?;
        let prepare = || -> Result<_, WorkerError> {
            Ok([
                wasm::input_file(&root, &config)?,
                wasm::input_file(&root, component)?,
                File::open("/dev/null").map_err(|_| WorkerError::Preparation)?,
                runtime
                    .executable
                    .try_clone()
                    .map_err(|_| WorkerError::Preparation)?,
            ])
        };
        let descriptors = prepare().map_err(worker_failure)?;
        Ok(Self {
            program: CString::new(
                runtime
                    .program(8)
                    .map_err(|_| Failure::GateRuntimeUnavailable)?
                    .as_os_str()
                    .as_encoded_bytes(),
            )
            .map_err(|_| Failure::GateRuntimeUnavailable)?,
            arguments: vec![CString::new("gate-supervised").unwrap()],
            record,
            _objects: vec![],
            domain: Box::new(GateDomain {
                descriptors,
                _runtime: runtime,
                process: wasm::domain::OwnedProcess::new(Some(deadline)),
            }),
        })
    }
}
impl WorkerProcess {
    fn take_gate_stream(&mut self) -> Result<UnixStream, Failure> {
        if !matches!(
            self._preparation
                .lock()
                .map_err(|_| Failure::GateWorkerFailed)?
                .record,
            PreparedLaunch::Gate { .. }
        ) {
            return Err(Failure::GateInvalidInput);
        }
        let stream = self.sdk.take().ok_or(Failure::GateWorkerFailed)?;
        stream
            .set_nonblocking(false)
            .map_err(|_| Failure::GateWorkerFailed)?;
        Ok(stream)
    }
}

/// Blocking bounded operation, including successful exit/reap before accepting
/// the result. The kernel must call this off its writer and async executor.
pub fn evaluate_blocking(
    runtime: WasmRuntime,
    launch: &Launch,
    component: &[u8],
    scratch: &Path,
) -> Result<Report, Failure> {
    let deadline = Instant::now() + Duration::from_millis(contract::TOTAL_MS);
    let prepared = PreparedWorker::gate(runtime, launch, component, scratch, deadline)?;
    let mut worker = WorkerProcess::spawn_blocking(
        prepared,
        WorkerLimits {
            startup_timeout: Duration::from_millis(contract::TOTAL_MS),
            log_tail_bytes: 1,
            log_bytes_per_second: 1024,
        },
    )
    .map_err(worker_failure)?;
    let read = (|| -> Result<Report, Failure> {
        let mut stream = worker.take_gate_stream()?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or(Failure::GateTimeout)?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|_| Failure::GateWorkerFailed)?;
        let mut size = [0u8; 4];
        stream.read_exact(&mut size).map_err(read_failure)?;
        let size = u32::from_be_bytes(size) as usize;
        if size > contract::OUTPUT_BYTES + 1024 {
            return Err(Failure::GateLimit);
        }
        let mut bytes = vec![0; size];
        stream.read_exact(&mut bytes).map_err(read_failure)?;
        let mut extra = [0u8; 1];
        if stream.read(&mut extra).map_err(read_failure)? != 0 {
            return Err(Failure::GateInvalidOutput);
        }
        let value = contract::parse_json(&bytes, contract::OUTPUT_BYTES + 1024)
            .map_err(|_| Failure::GateInvalidOutput)?;
        let response: Response =
            serde_json::from_value(value).map_err(|_| Failure::GateInvalidOutput)?;
        if response.generation != launch.generation
            || response.entry_id != launch.entry_id
            || response.module_sha256 != launch.module_sha256
            || response.report.profile != contract::PROFILE
            || response.report.fuel_consumed > contract::FUEL
        {
            return Err(Failure::GateInvalidOutput);
        }
        if let Ok(output) = &response.report.outcome {
            output.validate(&launch.evaluation)?;
        }
        Ok(response.report)
    })();
    if read.is_err() {
        worker.cancellation().cancel();
    }
    let exit = worker.wait_blocking().map_err(worker_failure)?;
    if Instant::now() >= deadline {
        return Err(Failure::GateTimeout);
    }
    if let Some(error) = exit.failure {
        return Err(worker_failure(error));
    }
    if exit.signal == Some(libc::SIGXCPU) || exit.signal == Some(libc::SIGKILL) {
        return Err(Failure::GateResourceLimit);
    }
    if exit.code != Some(0) || exit.signal.is_some() {
        return Err(Failure::GateWorkerFailed);
    }
    read
}
fn read_failure(error: std::io::Error) -> Failure {
    if matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        Failure::GateTimeout
    } else {
        Failure::GateWorkerFailed
    }
}
fn worker_failure(error: WorkerError) -> Failure {
    match error {
        WorkerError::StartupTimeout => Failure::GateTimeout,
        WorkerError::Containment | WorkerError::Preparation | WorkerError::Spawn => {
            Failure::GateRuntimeUnavailable
        }
        WorkerError::MemoryLimit | WorkerError::ThreadLimit | WorkerError::CpuLimit => {
            Failure::GateResourceLimit
        }
        _ => Failure::GateWorkerFailed,
    }
}
