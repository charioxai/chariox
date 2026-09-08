use super::*;
use std::{
    fs, io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicI32, AtomicUsize},
};

mod resources;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let pattern = std::env::temp_dir().join("chariox-worker-process-XXXXXX");
        let mut pattern = CString::new(pattern.as_os_str().as_bytes())
            .unwrap()
            .into_bytes_with_nul();
        let result = unsafe { libc::mkdtemp(pattern.as_mut_ptr().cast()) };
        assert!(!result.is_null(), "private test scratch creation failed");
        let path = PathBuf::from(std::ffi::OsStr::from_bytes(&pattern[..pattern.len() - 1]));
        Self(fs::canonicalize(path).unwrap())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    scratch: Scratch,
    executable: PathBuf,
    next: AtomicUsize,
}
impl Fixture {
    fn compile() -> Self {
        let scratch = Scratch::new();
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/app-worker/src");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/worker_process/fixture.c");
        let executable = scratch.0.join("abi-fixture");
        let diagnostics = scratch.0.join("compiler.log");
        let mut compiler = Command::new("/usr/bin/cc")
            .args([
                "-std=c11",
                "-D_POSIX_C_SOURCE=200809L",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-O1",
                "-I",
            ])
            .arg(&source)
            .arg(source.join("launch_record.c"))
            .arg(fixture)
            .arg("-o")
            .arg(&executable)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(File::create(&diagnostics).unwrap())
            .spawn()
            .expect("start small test-only native ABI compiler");
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = compiler.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = compiler.kill();
                let _ = compiler.wait();
                panic!("native ABI fixture compiler deadline");
            }
            thread::sleep(Duration::from_millis(10));
        };
        assert!(status.success(), "native ABI fixture compile: {}", {
            use std::io::Read;
            let mut bytes = Vec::new();
            File::open(diagnostics)
                .unwrap()
                .take(32768)
                .read_to_end(&mut bytes)
                .unwrap();
            String::from_utf8_lossy(&bytes).into_owned()
        });
        Self {
            scratch,
            executable,
            next: AtomicUsize::new(0),
        }
    }
    fn prepare(
        &self,
        mode: &str,
        reject: bool,
        panic_verify: bool,
    ) -> (PreparedWorker, Arc<Observed>, PathBuf) {
        let index = self.next.fetch_add(1, Ordering::Relaxed);
        let parent = self.scratch.0.join(format!("installation-{index}"));
        fs::create_dir(&parent).unwrap();
        let roots = ["package", "data", "tmp", "runtime"].map(|name| {
            let path = parent.join(name);
            fs::create_dir(&path).unwrap();
            path.to_str().unwrap().to_owned()
        });
        let objects = roots
            .iter()
            .map(File::open)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let marker = Path::new(&roots[1]).join("continued");
        let observed = Arc::new(Observed::default());
        let prepared = PreparedWorker {
            program: CString::new(self.executable.as_os_str().as_bytes()).unwrap(),
            arguments: vec![CString::new(mode).unwrap()],
            record: LaunchRecord {
                generation: "7".into(),
                installation: "installation_1".into(),
                release_digest: format!("sha256:{}", "a".repeat(64)),
                roots,
                bootstrap: "// trusted fixture bootstrap\n".into(),
                nofile: 128,
                cpu_seconds: 30,
                heap_mib: 64,
                v8_threads: 1,
                max_file_bytes: 1048576,
            },
            _objects: objects,
            domain: Box::new(TestDomain {
                observed: observed.clone(),
                reject,
                panic_verify,
            }),
        };
        (prepared, observed, marker)
    }
}

#[derive(Default)]
struct Observed {
    pid: AtomicI32,
    verified: AtomicBool,
    reaped: AtomicBool,
    dropped: AtomicBool,
}
struct TestDomain {
    observed: Arc<Observed>,
    reject: bool,
    panic_verify: bool,
}
impl ResourceDomain for TestDomain {
    fn verify_before_continue(&mut self, pid: libc::pid_t) -> Result<(), WorkerError> {
        self.observed.pid.store(pid, Ordering::SeqCst);
        assert_eq!(
            unsafe { libc::getsid(pid) },
            pid,
            "spawn creates a separate session"
        );
        if self.panic_verify {
            panic!("test-only verifier panic");
        }
        if self.reject {
            return Err(WorkerError::ResourceDomain);
        }
        self.observed.verified.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn terminate(&mut self, pid: libc::pid_t) {
        self.observed.pid.store(pid, Ordering::SeqCst);
    }
    fn reap_domain_blocking(&mut self) {
        let mut status = 0;
        let pid = self.observed.pid.load(Ordering::SeqCst);
        assert!(pid > 0);
        assert_eq!(
            unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD),
            "lease retained through actual waitpid"
        );
        self.observed.reaped.store(true, Ordering::SeqCst);
    }
}
impl Drop for TestDomain {
    fn drop(&mut self) {
        if self.observed.pid.load(Ordering::SeqCst) != 0 {
            assert!(self.observed.reaped.load(Ordering::SeqCst));
        }
        self.observed.dropped.store(true, Ordering::SeqCst);
    }
}
fn assert_released(observed: &Observed) {
    assert!(observed.reaped.load(Ordering::SeqCst));
    assert!(observed.dropped.load(Ordering::SeqCst));
}

#[test]
fn native_abi_and_process_lifecycle() {
    let fixture = Fixture::compile();
    // Intentionally create an ambient descriptor without CLOEXEC. It must not
    // reach native main, before the C launcher could filter it itself.
    let ambient = File::open("/dev/null").unwrap();
    let leaked = unsafe { libc::fcntl(ambient.as_raw_fd(), libc::F_DUPFD, 240) };
    assert_eq!(leaked, 240);
    let _leaked = unsafe { File::from_raw_fd(leaked) };
    let limits = WorkerLimits {
        startup_timeout: Duration::from_secs(2),
        ..Default::default()
    };
    let (prepared, observed, marker) = fixture.prepare("normal", false, false);
    let mut worker = WorkerProcess::spawn_blocking(prepared, limits).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut sdk = worker.take_sdk_channel().unwrap();
        assert!(matches!(sdk.receive(Duration::from_secs(1)).await.unwrap(), crate::wire::Message::Event { name, .. } if name == "fixture.ready"));
        assert!(matches!(worker.take_sdk_channel(), Err(WorkerError::SdkUnavailable)));
    });
    let result = worker.wait_blocking().unwrap();
    assert_eq!(result.code, Some(0));
    assert_eq!(result.failure, None);
    assert_eq!(result.stdout_tail, b"fixture complete\n");
    assert_eq!(result.stderr_tail, b"diagnostic\n");
    assert!(marker.exists());
    assert!(observed.verified.load(Ordering::SeqCst));
    assert_released(&observed);

    for (mode, reject, panic_verify, expected) in [
        ("wrong", false, false, WorkerError::Identity),
        ("normal", true, false, WorkerError::ResourceDomain),
        ("normal", false, true, WorkerError::Supervisor),
        ("early", false, false, WorkerError::EarlyExit),
        ("timeout", false, false, WorkerError::StartupTimeout),
    ] {
        let (prepared, observed, marker) = fixture.prepare(mode, reject, panic_verify);
        let limits = WorkerLimits {
            startup_timeout: Duration::from_millis(300),
            ..limits
        };
        let started = Instant::now();
        let error = WorkerProcess::spawn_blocking(prepared, limits)
            .err()
            .expect(mode);
        assert_eq!(error, expected, "{mode}");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{mode} cleanup deadline"
        );
        assert!(!marker.exists(), "{mode} must never continue");
        assert_released(&observed);
    }
    let (prepared, observed, _) = fixture.prepare("flood", false, false);
    let limits = WorkerLimits {
        log_tail_bytes: 1024,
        log_bytes_per_second: 8192,
        ..limits
    };
    let result = WorkerProcess::spawn_blocking(prepared, limits)
        .unwrap()
        .wait_blocking()
        .unwrap();
    assert_eq!(result.failure, Some(WorkerError::LogLimit));
    assert!(result.stdout_tail.len() <= 1024);
    assert_released(&observed);

    let (prepared, observed, _) = fixture.prepare("hang", false, false);
    let mut worker = WorkerProcess::spawn_blocking(prepared, limits).unwrap();
    let cancellation = worker.cancellation();
    let sdk = runtime.block_on(async { worker.take_sdk_channel().unwrap() });
    assert!(!observed.dropped.load(Ordering::SeqCst));
    let started = Instant::now();
    drop(worker);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_released(&observed);
    // Neither an outstanding SDK stream nor a cancellation clone owns/revives
    // the worker or prevents the lease from being released after actual reap.
    cancellation.cancel();
    drop(sdk);

    let (prepared, observed, _) = fixture.prepare("hang", false, false);
    let worker = WorkerProcess::spawn_blocking(prepared, limits).unwrap();
    worker.cancellation().cancel();
    let result = worker.wait_blocking().unwrap();
    assert_eq!(result.failure, Some(WorkerError::Cancelled));
    assert_released(&observed);
}

#[test]
fn preparation_rejects_invalid_native_identity_and_path_boundaries() {
    let scratch = Scratch::new();
    let mut record = LaunchRecord {
        generation: "7".into(),
        installation: "installation_1".into(),
        release_digest: format!("sha256:{}", "a".repeat(64)),
        roots: ["package", "data", "tmp", "runtime"]
            .map(|name| scratch.0.join(name).to_str().unwrap().to_owned()),
        bootstrap: "// trusted\n".into(),
        nofile: 128,
        cpu_seconds: 30,
        heap_mib: 64,
        v8_threads: 1,
        max_file_bytes: 1048576,
    };
    for generation in [
        "0",
        "00",
        "07",
        "+7",
        "-7",
        "9223372036854775808",
        "18446744073709551615",
    ] {
        record.generation = generation.into();
        assert_eq!(record.encode().unwrap_err(), WorkerError::Preparation);
    }
    record.generation = i64::MAX.to_string();
    assert!(record.encode().is_ok());
    record.roots[1] = format!("{}/child", record.roots[0]);
    assert!(record.encode().is_err());
    record.roots[1] = "/app/../data".into();
    assert!(record.encode().is_err());
    record.roots[1] = "/app/data".into();
    record.release_digest = format!("sha256:{}", "é".repeat(32));
    assert!(record.encode().is_err());
}
