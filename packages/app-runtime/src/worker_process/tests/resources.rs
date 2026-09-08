use super::*;
use std::sync::Mutex;

enum Check {
    Error(WorkerError),
    Panic,
    #[cfg(target_os = "macos")]
    Growth,
}

#[derive(Default)]
struct Checks {
    verified: Mutex<Option<Instant>>,
    samples: Mutex<Vec<Instant>>,
    failure: Mutex<Option<Instant>>,
    reaped: Mutex<Option<Instant>>,
}

struct CheckedDomain {
    inner: Box<dyn ResourceDomain>,
    check: Check,
    observed: Arc<Checks>,
    #[cfg(target_os = "macos")]
    monitor: Option<worker_platform::MacResourceMonitor>,
}

impl ResourceDomain for CheckedDomain {
    fn verify_before_continue(&mut self, pid: libc::pid_t) -> Result<(), WorkerError> {
        self.inner.verify_before_continue(pid)?;
        #[cfg(target_os = "macos")]
        if matches!(self.check, Check::Growth) {
            let mut monitor = worker_platform::MacResourceMonitor::attach(pid, Instant::now())?;
            monitor.lower_memory_for_growth_fixture();
            self.monitor = Some(monitor);
        }
        *self.observed.verified.lock().unwrap() = Some(Instant::now());
        Ok(())
    }

    fn check_running(&mut self, _pid: libc::pid_t, now: Instant) -> Result<(), WorkerError> {
        let count = {
            let mut samples = self.observed.samples.lock().unwrap();
            samples.push(now);
            samples.len()
        };
        #[cfg(target_os = "macos")]
        if matches!(self.check, Check::Growth) {
            let result = self.monitor.as_mut().unwrap().check_running(_pid, now);
            if result.is_err() {
                *self.observed.failure.lock().unwrap() = Some(Instant::now());
            }
            return result;
        }
        if count < 2 {
            return Ok(());
        }
        *self.observed.failure.lock().unwrap() = Some(Instant::now());
        match self.check {
            Check::Error(error) => Err(error),
            Check::Panic => panic!("test-only running domain panic"),
            #[cfg(target_os = "macos")]
            Check::Growth => unreachable!(),
        }
    }

    fn terminate(&mut self, pid: libc::pid_t) {
        self.inner.terminate(pid);
    }

    fn reap_domain_blocking(&mut self) {
        self.inner.reap_domain_blocking();
        *self.observed.reaped.lock().unwrap() = Some(Instant::now());
    }
}

fn checked(mut prepared: PreparedWorker, check: Check) -> (PreparedWorker, Arc<Checks>) {
    let observed = Arc::new(Checks::default());
    prepared.domain = Box::new(CheckedDomain {
        inner: prepared.domain,
        check,
        observed: observed.clone(),
        #[cfg(target_os = "macos")]
        monitor: None,
    });
    (prepared, observed)
}

fn wait_with_deadline(worker: WorkerProcess) -> Result<WorkerExit, WorkerError> {
    let cancellation = worker.cancellation();
    let (done, wait) = mpsc::channel();
    let watchdog = thread::spawn(move || {
        if matches!(
            wait.recv_timeout(Duration::from_secs(2)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ) {
            cancellation.cancel();
        }
    });
    let result = worker.wait_blocking();
    let _ = done.send(());
    watchdog.join().unwrap();
    result
}

fn assert_sample_and_reap_times(checks: &Checks, name: &str) {
    let verified = checks.verified.lock().unwrap().unwrap();
    let samples = checks.samples.lock().unwrap();
    assert!(samples[0].duration_since(verified) >= monitor::RESOURCE_CHECK_INTERVAL);
    for pair in samples.windows(2) {
        assert!(pair[1].duration_since(pair[0]) >= monitor::RESOURCE_CHECK_INTERVAL);
    }
    let detected = checks.failure.lock().unwrap().unwrap();
    let reaped = checks.reaped.lock().unwrap().unwrap();
    let detection = detected.duration_since(verified);
    let cleanup = reaped.duration_since(detected);
    assert!(detection < Duration::from_secs(2), "{name}: detection");
    assert!(cleanup < Duration::from_secs(2), "{name}: cleanup");
    eprintln!(
        "{name}: first-sample={}ms detection={}ms detection-to-reap={}us",
        samples[0].duration_since(verified).as_millis(),
        detection.as_millis(),
        cleanup.as_micros()
    );
}

#[test]
fn running_resource_failures_retain_admission_through_actual_reap() {
    let fixture = Fixture::compile();
    for error in [
        WorkerError::MemoryLimit,
        WorkerError::ThreadLimit,
        WorkerError::CpuLimit,
        WorkerError::ResourceTelemetry,
    ] {
        let (prepared, observed, marker) = fixture.prepare("hang", false, false);
        let (prepared, checks) = checked(prepared, Check::Error(error));
        let result = wait_with_deadline(
            WorkerProcess::spawn_blocking(prepared, WorkerLimits::default()).unwrap(),
        )
        .unwrap();
        assert_eq!(result.failure, Some(error));
        assert_eq!(result.signal, Some(libc::SIGKILL));
        assert!(marker.exists());
        assert_released(&observed);
        assert_sample_and_reap_times(&checks, &error.to_string());
    }
    let (prepared, observed, _) = fixture.prepare("hang", false, false);
    let (prepared, checks) = checked(prepared, Check::Panic);
    let result = wait_with_deadline(
        WorkerProcess::spawn_blocking(prepared, WorkerLimits::default()).unwrap(),
    );
    assert!(matches!(result, Err(WorkerError::Supervisor)));
    assert_released(&observed);
    assert_sample_and_reap_times(&checks, "running monitor panic");
}

#[test]
fn reaped_or_panicked_monitor_retains_preparation_until_lifecycle_join() {
    let fixture = Fixture::compile();
    for panic_monitor in [false, true] {
        let (prepared, observed, _) =
            fixture.prepare(if panic_monitor { "hang" } else { "normal" }, false, false);
        let held_directory = prepared._objects[0].as_raw_fd();
        let prepared = if panic_monitor {
            checked(prepared, Check::Panic).0
        } else {
            prepared
        };
        let worker = WorkerProcess::spawn_blocking(prepared, WorkerLimits::default()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !worker.monitor.as_ref().unwrap().is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        // A finished monitor has already reaped its native process. Its caller
        // can still be draining actual SDK broker work before it joins the owner.
        assert!(worker.monitor.as_ref().unwrap().is_finished());
        assert!(observed.reaped.load(Ordering::SeqCst));
        assert!(!observed.dropped.load(Ordering::SeqCst));
        assert_ne!(unsafe { libc::fcntl(held_directory, libc::F_GETFD) }, -1);
        let result = worker.wait_blocking();
        if panic_monitor {
            assert!(matches!(result, Err(WorkerError::Supervisor)));
        } else {
            assert_eq!(result.unwrap().code, Some(0));
        }
        assert_released(&observed);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_libproc_detects_tiny_private_memory_growth_after_continue() {
    let fixture = Fixture::compile();
    let (prepared, observed, marker) = fixture.prepare("grow", false, false);
    let (prepared, checks) = checked(prepared, Check::Growth);
    let result = wait_with_deadline(
        WorkerProcess::spawn_blocking(prepared, WorkerLimits::default()).unwrap(),
    )
    .unwrap();
    assert_eq!(result.failure, Some(WorkerError::MemoryLimit));
    assert_eq!(result.signal, Some(libc::SIGKILL));
    assert!(marker.exists());
    assert_released(&observed);
    assert_sample_and_reap_times(&checks, "macOS real 8MiB growth/baseline+2MiB limit");
}
