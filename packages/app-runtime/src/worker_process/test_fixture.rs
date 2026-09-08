//! Fixed native ABI fixture for cross-crate tests. This is intentionally NOT a
//! sandbox or embedded Node worker. There is no caller-selected executable,
//! bootstrap, path, generation, limit or resource domain. The feature is absent
//! from production dependencies and rejected by release-profile compilation.
//! Only package identity comes from an already verified package; no package
//! payload is executed. Installation names come from fixed fixture modes.

use super::{
    record::LaunchRecord, PreparedWorker, ResourceDomain, WorkerError, WorkerLimits, WorkerProcess,
};
use std::{
    ffi::{CString, OsStr},
    fs::{self, File},
    io::{self, Read},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub enum Mode {
    Ready,
    WrongHandlers,
    NoReport,
    BrokerCall,
    ToolEcho,
    Files,
    Http,
    HttpPaused,
    Health,
    BadHealth,
    OtherInstallation,
}
impl Mode {
    fn argument(self) -> &'static str {
        match self {
            Self::Ready => "sdk_ready",
            Self::WrongHandlers => "sdk_wrong_handlers",
            Self::NoReport => "sdk_no_report",
            Self::BrokerCall => "sdk_broker_call",
            Self::ToolEcho => "sdk_tool",
            Self::Files => "sdk_files",
            Self::Http => "sdk_http",
            Self::HttpPaused => "sdk_http_paused",
            Self::Health => "sdk_health",
            Self::BadHealth => "sdk_bad_health",
            Self::OtherInstallation => "sdk_other_installation",
        }
    }
}

pub struct Fixture {
    scratch: Arc<Scratch>,
    executable: PathBuf,
    next: AtomicUsize,
}
pub struct Observation {
    reaped: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
    marker: PathBuf,
    _scratch: Arc<Scratch>,
}
impl Observation {
    pub fn was_reaped(&self) -> bool {
        self.reaped.load(Ordering::Acquire)
    }
    pub fn lease_was_dropped(&self) -> bool {
        self.dropped.load(Ordering::Acquire)
    }
    pub fn ready_was_acknowledged(&self) -> bool {
        self.marker.is_file()
    }
    pub fn tool_invocations(&self) -> u64 {
        self.marker
            .with_file_name("tool-effects")
            .metadata()
            .map(|m| m.len() / 2)
            .unwrap_or(0)
    }
    /// Observe only the fixed broker fixture output; no caller-selected path.
    pub fn private_file(&self) -> io::Result<Option<Vec<u8>>> {
        let file = match File::open(self.marker.with_file_name("fixture-file")) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.take(512 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 512 * 1024 {
            return Err(io::Error::other(
                "fixed private fixture output exceeded bound",
            ));
        }
        Ok(Some(bytes))
    }
    pub fn pending_file_replacements(&self) -> io::Result<usize> {
        let mut count = 0;
        for (index, entry) in fs::read_dir(self.marker.parent().unwrap())?.enumerate() {
            if index >= 1024 {
                return Err(io::Error::other("fixed fixture directory exceeded bound"));
            }
            if entry?
                .file_name()
                .as_encoded_bytes()
                .starts_with(b".chariox-replace-")
            {
                count += 1;
            }
        }
        Ok(count)
    }
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
impl Fixture {
    /// Compiles only the fixed libc fixture and production startup-record codec.
    /// Run on a bounded blocking test thread, never the async kernel coordinator.
    pub fn compile() -> io::Result<Self> {
        let mut pattern = CString::new("/tmp/chariox-worker-fixture-XXXXXX")
            .unwrap()
            .into_bytes_with_nul();
        if unsafe { libc::mkdtemp(pattern.as_mut_ptr().cast()) }.is_null() {
            return Err(io::Error::last_os_error());
        }
        let scratch = Arc::new(Scratch(fs::canonicalize(Path::new(OsStr::from_bytes(
            &pattern[..pattern.len() - 1],
        )))?));
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/app-worker/src");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/worker_process/fixture.c");
        let executable = scratch.0.join("fixed-worker-fixture");
        let diagnostic_path = scratch.0.join("compiler.log");
        let mut child = Command::new("/usr/bin/cc")
            .args([
                "-std=c11",
                "-D_POSIX_C_SOURCE=200809L",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-O1",
                "-I",
            ])
            .arg(source.as_os_str())
            .arg(source.join("launch_record.c"))
            .arg(fixture)
            .arg("-o")
            .arg(&executable)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(File::create(&diagnostic_path)?)
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "fixed worker compiler timed out",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        };
        if !status.success() {
            let mut diagnostic = String::new();
            File::open(diagnostic_path)?
                .take(32768)
                .read_to_string(&mut diagnostic)?;
            return Err(io::Error::other(format!(
                "fixed worker compiler failed: {diagnostic}"
            )));
        }
        Ok(Self {
            scratch,
            executable,
            next: AtomicUsize::new(0),
        })
    }

    pub fn spawn_blocking(
        &self,
        mode: Mode,
        package: &chariox_app_package::VerifiedPackage<'_>,
    ) -> Result<(WorkerProcess, Observation), WorkerError> {
        self.spawn_for_installation_blocking(
            mode,
            package,
            match mode {
                Mode::OtherInstallation => "other_installed",
                _ => "installed",
            },
        )
    }

    /// Fixed-fixture identity only. Available solely under worker-test-fixture;
    /// it never accepts a runtime path, process or production sandbox override.
    pub fn spawn_for_installation_blocking(
        &self,
        mode: Mode,
        package: &chariox_app_package::VerifiedPackage<'_>,
        installation: &str,
    ) -> Result<(WorkerProcess, Observation), WorkerError> {
        if installation.is_empty()
            || installation.len() > 128
            || !installation
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(WorkerError::Preparation);
        }
        let prepare = || -> io::Result<(PreparedWorker, Observation)> {
            let parent = self.scratch.0.join(format!(
                "instance-{}",
                self.next.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&parent)?;
            let mut roots = [String::new(), String::new(), String::new(), String::new()];
            let mut objects = Vec::new();
            for (index, name) in ["package", "data", "tmp", "runtime"].iter().enumerate() {
                let path = parent.join(name);
                fs::create_dir(&path)?;
                roots[index] = path
                    .to_str()
                    .ok_or_else(|| io::Error::other("fixture path encoding"))?
                    .into();
                objects.push(File::open(path)?);
            }
            let reaped = Arc::new(AtomicBool::new(false));
            let dropped = Arc::new(AtomicBool::new(false));
            let observation = Observation {
                reaped: reaped.clone(),
                dropped: dropped.clone(),
                marker: Path::new(&roots[1]).join("ready-ack"),
                _scratch: self.scratch.clone(),
            };
            let private_data = objects[1].try_clone()?;
            Ok((
                PreparedWorker {
                    program: CString::new(self.executable.as_os_str().as_bytes()).unwrap(),
                    arguments: vec![CString::new(mode.argument()).unwrap()],
                    record: LaunchRecord {
                        generation: "1".into(),
                        installation: installation.into(),
                        release_digest: package.package_digest().into(),
                        roots,
                        bootstrap: "// trusted fixture bootstrap\n".into(),
                        nofile: 128,
                        cpu_seconds: 30,
                        heap_mib: 64,
                        v8_threads: 1,
                        max_file_bytes: 1048576,
                    },
                    _objects: objects,
                    domain: Box::new(FixtureDomain {
                        private_data,
                        reaped,
                        dropped,
                        _scratch: self.scratch.clone(),
                    }),
                },
                observation,
            ))
        };
        let (prepared, observation) = prepare().map_err(|_| WorkerError::Preparation)?;
        let worker = WorkerProcess::spawn_blocking(
            prepared,
            WorkerLimits {
                startup_timeout: Duration::from_secs(2),
                ..WorkerLimits::default()
            },
        )?;
        Ok((worker, observation))
    }
}

struct FixtureDomain {
    private_data: File,
    reaped: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
    _scratch: Arc<Scratch>,
}
impl ResourceDomain for FixtureDomain {
    fn private_data_directory(&self) -> Result<File, WorkerError> {
        self.private_data
            .try_clone()
            .map_err(|_| WorkerError::Preparation)
    }
    fn verify_before_continue(&mut self, pid: libc::pid_t) -> Result<(), WorkerError> {
        if unsafe { libc::getsid(pid) } != pid {
            return Err(WorkerError::ResourceDomain);
        }
        Ok(())
    }
    fn terminate(&mut self, _pid: libc::pid_t) {}
    fn reap_domain_blocking(&mut self) {
        self.reaped.store(true, Ordering::Release);
    }
}
impl Drop for FixtureDomain {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::Release);
    }
}
