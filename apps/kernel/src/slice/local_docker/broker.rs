use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::process::{Command, ExitStatus, Output, Stdio};
use zeroize::{Zeroize, Zeroizing};

#[cfg(unix)]
use std::collections::BTreeMap;
#[cfg(unix)]
use std::io::{BufReader, Read};
#[cfg(unix)]
use std::os::fd::{FromRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
use std::sync::{Mutex, OnceLock};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
const BROKER_SOCKET_ENV: &str = "CHARIOX_SLICE_DOCKER_BROKER_SOCKET";
#[cfg(unix)]
const BROKER_FD_ENV: &str = "CHARIOX_SLICE_DOCKER_BROKER_FD";
#[cfg(unix)]
const BROKER_REQUIRED_ENV: &str = "CHARIOX_SLICE_DOCKER_BROKER_REQUIRED";
#[cfg(unix)]
const MAX_BROKER_RESPONSE_BYTES: usize = 12 * 1024 * 1024;
#[cfg(unix)]
const MAX_BROKER_REQUEST_BYTES: usize = 12 * 1024 * 1024;
#[cfg(unix)]
const BROKER_IO_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(unix)]
const LOCAL_BROKER_RETRY_INTERVAL: Duration = Duration::from_secs(30);

#[cfg(unix)]
struct BrokerConnection {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

#[cfg(unix)]
static BROKER: OnceLock<Mutex<Option<BrokerConnection>>> = OnceLock::new();
#[cfg(unix)]
static BROKER_CONFIGURED: OnceLock<bool> = OnceLock::new();

/// Why the local DEV broker never connected; a later broker use retries it.
#[cfg(unix)]
struct LocalBrokerOutage {
    reason: String,
    last_attempt: Instant,
}

#[cfg(unix)]
static LOCAL_BROKER_OUTAGE: Mutex<Option<LocalBrokerOutage>> = Mutex::new(None);

#[cfg(unix)]
#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum BrokerRequest<'a> {
    Docker {
        args: &'a [String],
    },
    Provisioner {
        action: &'a str,
        environment: &'a BTreeMap<String, String>,
        files: &'a [BrokerProvisionerFile],
    },
    HomeArchiveCapture {
        container: &'a str,
        scope: &'a str,
        id: &'a str,
    },
    HomeArchiveRemove {
        scope: &'a str,
        id: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<&'a str>,
    },
    HomeArchiveVerify {
        scope: &'a str,
        id: &'a str,
        path: &'a str,
    },
    HomeRestoreResolve {
        container: &'a str,
        path: &'a str,
    },
    ProviderAuthLayout {
        container: &'a str,
    },
    CapturePreflight {
        container: &'a str,
    },
}

#[cfg(unix)]
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BrokerProvisionerFile {
    environment: String,
    name: String,
    contents_base64: String,
}

#[cfg(unix)]
impl Drop for BrokerProvisionerFile {
    fn drop(&mut self) {
        self.contents_base64.zeroize();
    }
}

pub(super) struct ProvisionerInput {
    pub(super) environment: &'static str,
    pub(super) name: &'static str,
    pub(super) contents: Zeroizing<Vec<u8>>,
}

#[cfg(unix)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrokerResponse {
    status: i32,
    stdout_base64: String,
    stderr_base64: String,
    #[serde(default)]
    disk_quota_evidence: Option<super::super::disk_quota_policy::SliceDiskQuotaEvidence>,
}

pub(super) struct BrokerExecution {
    pub(super) output: Output,
    pub(super) disk_quota_evidence: Option<super::super::disk_quota_policy::SliceDiskQuotaEvidence>,
}

#[cfg(unix)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HomeArchiveCaptureResponse {
    path: String,
    size_bytes: u64,
    sha256: String,
}

pub fn initialize() {
    #[cfg(unix)]
    {
        let mut socket_path = std::env::var_os(BROKER_SOCKET_ENV);
        let inherited_fd = std::env::var(BROKER_FD_ENV)
            .ok()
            .and_then(|value| value.parse::<RawFd>().ok());
        let local = if socket_path.is_none()
            && inherited_fd.is_none()
            && std::env::var_os(BROKER_REQUIRED_ENV).is_none()
        {
            super::local_authority::start()
        } else {
            None
        };
        let local_configured = local.is_some();
        let mut local_failure = None;
        match local {
            Some(Ok(path)) => socket_path = Some(path.into_os_string()),
            Some(Err(error)) => local_failure = Some(error.to_string()),
            None => {}
        }
        let configured = local_configured
            || socket_path.is_some()
            || inherited_fd.is_some()
            || std::env::var_os(BROKER_REQUIRED_ENV).is_some();
        let _ = BROKER_CONFIGURED.set(configured);
        std::env::remove_var(BROKER_SOCKET_ENV);
        std::env::remove_var(BROKER_FD_ENV);
        std::env::remove_var(BROKER_REQUIRED_ENV);
        let inherited_fd = inherited_fd.and_then(|raw_fd| {
            if set_close_on_exec(raw_fd).is_ok() {
                Some(raw_fd)
            } else {
                unsafe {
                    libc::close(raw_fd);
                }
                None
            }
        });
        // Every kernel owns relay/provider control memory, including ordinary hosts.
        // Exec resets dumpability for normal provider diagnostics on Linux.
        if !make_process_nondumpable() {
            if let Some(raw_fd) = inherited_fd {
                unsafe {
                    libc::close(raw_fd);
                }
            }
            panic!("failed to protect kernel control memory from process dumps");
        }
        if let Some(raw_fd) = inherited_fd {
            let writer = unsafe { UnixStream::from_raw_fd(raw_fd) };
            if configure_stream_deadlines(&writer).is_err() {
                return;
            }
            if let Ok(reader) = writer.try_clone() {
                let state = BROKER.get_or_init(|| Mutex::new(None));
                *state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(BrokerConnection {
                    reader: BufReader::new(reader),
                    writer,
                });
                return;
            }
        }
        if let Some(reason) = local_failure {
            record_local_broker_outage(reason);
            return;
        }
        let Some(socket_path) = socket_path else {
            return;
        };
        let state = BROKER.get_or_init(|| Mutex::new(None));
        let mut state = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.is_some() {
            return;
        }
        match connect(Path::new(&socket_path)) {
            Ok(connection) => *state = Some(connection),
            Err(error) if local_configured => record_local_broker_outage(error.to_string()),
            Err(_) => {}
        }
    }
}

#[cfg(unix)]
fn connect(socket_path: &Path) -> io::Result<BrokerConnection> {
    let mut refused = None;
    for _ in 0..50 {
        match UnixStream::connect(socket_path) {
            Ok(writer) => {
                configure_stream_deadlines(&writer)?;
                let reader = writer.try_clone()?;
                return Ok(BrokerConnection {
                    reader: BufReader::new(reader),
                    writer,
                });
            }
            Err(error) => {
                refused = Some(error);
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    Err(io::Error::other(format!(
        "managed slice Docker broker transport refused the connection: {}",
        refused.map_or_else(String::new, |error| error.to_string())
    )))
}

#[cfg(unix)]
fn record_local_broker_outage(reason: String) {
    // The process logger starts after the broker, so stderr carries this one.
    eprintln!(
        "chariox-kernel: managed slice Docker broker is unavailable: {reason}; a later slice operation retries it"
    );
    *LOCAL_BROKER_OUTAGE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(LocalBrokerOutage {
        reason,
        last_attempt: Instant::now(),
    });
}

#[cfg(unix)]
fn start_local_broker() -> io::Result<BrokerConnection> {
    let path = super::local_authority::start().unwrap_or_else(|| {
        Err(io::Error::other(
            "local DEV broker enrollment is no longer present",
        ))
    })?;
    connect(&path)
}

/// Retries a local DEV broker that never connected, at most once per interval,
/// and logs every outcome. Established connections are never restarted here.
#[cfg(unix)]
fn retry_local_broker(
    outage: &mut Option<LocalBrokerOutage>,
    now: Instant,
    start: impl FnOnce() -> io::Result<BrokerConnection>,
) -> Option<BrokerConnection> {
    let current = outage.as_mut()?;
    if now.saturating_duration_since(current.last_attempt) < LOCAL_BROKER_RETRY_INTERVAL {
        return None;
    }
    current.last_attempt = now;
    match start() {
        Ok(connection) => {
            *outage = None;
            crate::logging::info(
                "slice.local_docker.broker",
                "managed slice Docker broker started on retry",
            );
            Some(connection)
        }
        Err(error) => {
            current.reason = error.to_string();
            crate::logging::warn_with_fields(
                "slice.local_docker.broker",
                "managed slice Docker broker start failed; a later slice operation retries it",
                serde_json::json!({ "reason": current.reason }),
            );
            None
        }
    }
}

#[cfg(unix)]
fn broker_unavailable() -> io::Error {
    unavailable(
        &LOCAL_BROKER_OUTAGE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}

#[cfg(unix)]
fn unavailable(outage: &Option<LocalBrokerOutage>) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotConnected,
        match outage {
            Some(outage) => format!(
                "managed slice Docker broker is unavailable: {}; a later slice operation retries it",
                outage.reason
            ),
            None => "managed slice Docker broker is unavailable".to_string(),
        },
    )
}

#[cfg(unix)]
fn configure_stream_deadlines(stream: &UnixStream) -> io::Result<()> {
    stream.set_read_timeout(None)?;
    stream.set_write_timeout(Some(BROKER_IO_TIMEOUT))
}

#[cfg(target_os = "linux")]
fn make_process_nondumpable() -> bool {
    unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) == 0 }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn make_process_nondumpable() -> bool {
    true
}

#[cfg(unix)]
fn set_close_on_exec(raw_fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(raw_fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(raw_fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn broker_is_configured() -> bool {
    BROKER_CONFIGURED.get().copied().unwrap_or_else(|| {
        std::env::var_os(BROKER_SOCKET_ENV).is_some()
            || std::env::var_os(BROKER_FD_ENV).is_some()
            || std::env::var_os(BROKER_REQUIRED_ENV).is_some()
    })
}

pub(super) fn configured() -> bool {
    broker_is_configured()
}

pub(super) fn provider_auth_protected(container: &str) -> io::Result<bool> {
    if !broker_is_configured() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Layout {
            protected_layout: bool,
        }
        let output = execute(&BrokerRequest::ProviderAuthLayout { container })?;
        if output.status.success() {
            return serde_json::from_slice::<Layout>(&output.stdout)
                .map(|layout| layout.protected_layout)
                .map_err(io::Error::other);
        }
    }
    Err(io::Error::other(
        "provider auth layout verification refused",
    ))
}

pub(super) fn require_capture_preflight(container: &str) -> io::Result<()> {
    if !broker_is_configured() {
        return Err(io::Error::other("protected capture broker is unavailable"));
    }
    #[cfg(unix)]
    {
        let output = execute(&BrokerRequest::CapturePreflight { container })?;
        if output.status.success() {
            if String::from_utf8_lossy(&output.stderr).starts_with("Legacy release F slice:") {
                tracing::warn!(slice_container = container, "Legacy release F slice capture includes the original mixed home and image; migrate to a protected slice for credential separation");
            }
            return Ok(());
        }
    }
    Err(io::Error::other("protected capture preflight refused"))
}

pub(super) fn resolve_home_restore(container: &str, path: &str) -> io::Result<()> {
    if !broker_is_configured() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        let output = execute(&BrokerRequest::HomeRestoreResolve { container, path })?;
        if !output.status.success() {
            return Err(io::Error::other(
                "protected home restore resolution remains pending",
            ));
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn broker_is_configured() -> bool {
    false
}

#[cfg(unix)]
fn execute_with_disk_evidence(request: &BrokerRequest<'_>) -> io::Result<BrokerExecution> {
    let request = Zeroizing::new(
        serde_json::to_vec(request)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?,
    );
    if request.len() > MAX_BROKER_REQUEST_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "managed slice Docker broker request is too large",
        ));
    }
    let state = BROKER.get_or_init(|| Mutex::new(None));
    let mut state = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.is_none() {
        *state = retry_local_broker(
            &mut LOCAL_BROKER_OUTAGE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            Instant::now(),
            start_local_broker,
        );
    }
    let result = (|| {
        let connection = state.as_mut().ok_or_else(broker_unavailable)?;
        let previous_read_timeout = connection.writer.read_timeout()?;
        connection.writer.set_read_timeout(None)?;
        let result = (|| {
            connection
                .writer
                .write_all(&(request.len() as u32).to_be_bytes())?;
            connection.writer.write_all(&request)?;
            connection.writer.flush()?;
            let mut header = [0_u8; 4];
            connection.reader.read_exact(&mut header)?;
            let response_len = u32::from_be_bytes(header) as usize;
            if response_len == 0 || response_len > MAX_BROKER_RESPONSE_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "managed slice Docker broker response is invalid",
                ));
            }
            let mut response = vec![0_u8; response_len];
            connection.reader.read_exact(&mut response)?;
            let response: BrokerResponse = serde_json::from_slice(&response)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            let decode = |value: &str| {
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, value)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
            };
            Ok(BrokerExecution {
                output: Output {
                    status: ExitStatus::from_raw(response.status.clamp(0, 255) << 8),
                    stdout: decode(&response.stdout_base64)?,
                    stderr: decode(&response.stderr_base64)?,
                },
                disk_quota_evidence: response.disk_quota_evidence,
            })
        })();
        // MP-08/MP-10/MP-11: all operations wait under broker lease ownership.
        // Preserve the caller write deadline and restore read state on every result.
        connection.writer.set_read_timeout(previous_read_timeout)?;
        result
    })();
    if result.is_err() {
        *state = None;
    }
    result
}

#[cfg(unix)]
fn execute(request: &BrokerRequest<'_>) -> io::Result<Output> {
    execute_with_disk_evidence(request).map(|response| response.output)
}

#[cfg(unix)]
pub(super) fn provisioner_environment(command: &Command) -> BTreeMap<String, String> {
    command
        .get_envs()
        .filter_map(|(name, value)| {
            let name = name.to_str()?;
            if !name.starts_with("CHARIOX_SLICE_")
                && !matches!(
                    name,
                    "CHARIOX_ROOM_ENVIRONMENT_HOME_KERNEL_ID"
                        | "CHARIOX_ROOM_ENVIRONMENT_HOME_PUBLIC_KEY"
                        | "CHARIOX_ROOM_ENVIRONMENT_SESSION_ID"
                        | "CHARIOX_ROOM_ENVIRONMENT_SLICE_ID"
                        | "CHARIOX_MANAGED_PROVIDER_ISOLATION_PROBE"
                )
            {
                return None;
            }
            Some((name.to_string(), value?.to_str()?.to_string()))
        })
        .collect()
}

pub(super) fn run_provisioner(
    command: &Command,
    action: &str,
    inputs: &[ProvisionerInput],
) -> Option<io::Result<BrokerExecution>> {
    if !broker_is_configured() {
        return None;
    }
    #[cfg(unix)]
    {
        let environment = provisioner_environment(command);
        let files = inputs
            .iter()
            .map(|input| BrokerProvisionerFile {
                environment: input.environment.to_string(),
                name: input.name.to_string(),
                contents_base64: base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &input.contents,
                ),
            })
            .collect::<Vec<_>>();
        Some(execute_with_disk_evidence(&BrokerRequest::Provisioner {
            action,
            environment: &environment,
            files: &files,
        }))
    }
    #[cfg(not(unix))]
    unreachable!()
}

pub(super) fn capture_home_archive(
    container: &str,
    scope: &str,
    id: &str,
) -> io::Result<Option<(std::path::PathBuf, u64, String)>> {
    if !broker_is_configured() {
        return Ok(None);
    }
    #[cfg(unix)]
    {
        let output = execute(&BrokerRequest::HomeArchiveCapture {
            container,
            scope,
            id,
        })?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "managed home archive capture failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let captured: HomeArchiveCaptureResponse = serde_json::from_slice(&output.stdout)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if captured.size_bytes == 0
            || captured.size_bytes > 9_007_199_254_740_991
            || captured.sha256.len() != 64
            || !captured
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "managed home archive digest is invalid",
            ));
        }
        Ok(Some((
            captured.path.into(),
            captured.size_bytes,
            captured.sha256,
        )))
    }
    #[cfg(not(unix))]
    unreachable!()
}

pub(super) fn verify_home_archive(
    scope: &str,
    id: &str,
    path: &std::path::Path,
) -> io::Result<Option<(u64, String)>> {
    if !broker_is_configured() {
        return Ok(None);
    }
    #[cfg(unix)]
    {
        let path = path.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "managed home archive path is not UTF-8",
            )
        })?;
        let output = execute(&BrokerRequest::HomeArchiveVerify { scope, id, path })?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "managed home archive verification failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let verified: HomeArchiveCaptureResponse = serde_json::from_slice(&output.stdout)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if verified.path != path
            || verified.size_bytes == 0
            || verified.size_bytes > 9_007_199_254_740_991
            || verified.sha256.len() != 64
            || !verified
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "managed home archive verification response is invalid",
            ));
        }
        Ok(Some((verified.size_bytes, verified.sha256)))
    }
    #[cfg(not(unix))]
    unreachable!()
}

pub(super) fn remove_home_archive(scope: &str, id: &str) -> io::Result<bool> {
    if !broker_is_configured() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        let output = execute(&BrokerRequest::HomeArchiveRemove {
            scope,
            id,
            path: None,
        })?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "managed home archive removal failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(true)
    }
    #[cfg(not(unix))]
    unreachable!()
}

pub(super) fn remove_home_archive_path(
    scope: &str,
    id: &str,
    path: &std::path::Path,
) -> io::Result<bool> {
    if !broker_is_configured() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        let path = path.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "managed home archive path is not UTF-8",
            )
        })?;
        let output = execute(&BrokerRequest::HomeArchiveRemove {
            scope,
            id,
            path: Some(path),
        })?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "managed home archive generation removal failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(true)
    }
    #[cfg(not(unix))]
    unreachable!()
}

pub(super) fn docker_command() -> DockerCommand {
    DockerCommand::default()
}

#[derive(Default)]
pub(super) struct DockerCommand {
    args: Vec<OsString>,
    quiet_stdout: bool,
    quiet_stderr: bool,
}

impl DockerCommand {
    pub(super) fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    pub(super) fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|arg| arg.as_ref().to_os_string()));
        self
    }

    pub(super) fn stdout(&mut self, _stdio: Stdio) -> &mut Self {
        self.quiet_stdout = true;
        self
    }

    pub(super) fn stderr(&mut self, _stdio: Stdio) -> &mut Self {
        self.quiet_stderr = true;
        self
    }

    pub(super) fn output(&mut self) -> io::Result<Output> {
        if broker_is_configured() {
            #[cfg(unix)]
            {
                let args = self
                    .args
                    .iter()
                    .map(|arg| {
                        arg.to_str().map(str::to_string).ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "Docker argument is not UTF-8",
                            )
                        })
                    })
                    .collect::<io::Result<Vec<_>>>()?;
                return execute(&BrokerRequest::Docker { args: &args });
            }
        }
        self.local_command().output()
    }

    pub(super) fn status(&mut self) -> io::Result<ExitStatus> {
        if broker_is_configured() {
            let output = self.output()?;
            if !self.quiet_stdout {
                io::stdout().write_all(&output.stdout)?;
            }
            if !self.quiet_stderr {
                io::stderr().write_all(&output.stderr)?;
            }
            return Ok(output.status);
        }
        self.local_command().status()
    }

    fn local_command(&self) -> Command {
        let mut command = Command::new("docker");
        command.args(&self.args);
        // MP-08/MP-11: use the same explicit engine as image production.
        if std::env::var_os("DOCKER_HOST").is_some_and(|host| !host.is_empty()) {
            command.env_remove("DOCKER_CONTEXT");
        }
        if self.quiet_stdout {
            command.stdout(Stdio::null());
        }
        if self.quiet_stderr {
            command.stderr(Stdio::null());
        }
        command
    }
}

#[cfg(all(test, unix))]
pub(super) fn broker_stream_is_close_on_exec(stream: &UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    let flags = unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFD) };
    flags >= 0 && flags & libc::FD_CLOEXEC != 0
}

#[cfg(all(test, unix))]
pub(super) fn mark_broker_stream_close_on_exec(stream: &UnixStream) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    set_close_on_exec(stream.as_raw_fd())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn outage(reason: &str, last_attempt: Instant) -> Option<LocalBrokerOutage> {
        Some(LocalBrokerOutage {
            reason: reason.to_string(),
            last_attempt,
        })
    }

    fn connection() -> BrokerConnection {
        let (writer, _peer) = UnixStream::pair().expect("socket pair should open");
        BrokerConnection {
            reader: BufReader::new(writer.try_clone().expect("socket should clone")),
            writer,
        }
    }

    #[test]
    fn a_failed_local_broker_start_is_retried_later_and_recovers() {
        let failed_at = Instant::now();
        let mut state = outage("transport refused: socket owner 0 mode 0755", failed_at);
        let too_soon = failed_at + Duration::from_secs(1);
        assert!(retry_local_broker(&mut state, too_soon, || panic!("retry waits")).is_none());

        let later = failed_at + LOCAL_BROKER_RETRY_INTERVAL;
        let still_failing = retry_local_broker(&mut state, later, || {
            Err(io::Error::other(
                "helper exited before publishing the transport",
            ))
        });
        assert!(still_failing.is_none());
        let error = unavailable(&state).to_string();
        assert!(
            error.contains("helper exited before publishing the transport")
                && error.contains("a later slice operation retries it"),
            "callers must see why the broker is missing: {error}"
        );

        let recovered = retry_local_broker(&mut state, later + LOCAL_BROKER_RETRY_INTERVAL, || {
            Ok(connection())
        });
        assert!(recovered.is_some());
        assert!(state.is_none());
    }

    #[test]
    fn only_a_local_broker_that_never_connected_is_retried() {
        let mut state = None;
        assert!(retry_local_broker(&mut state, Instant::now(), || panic!("no retry")).is_none());
        assert_eq!(
            unavailable(&state).to_string(),
            "managed slice Docker broker is unavailable"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mp08_mp11_kernel_control_memory_policy_is_common_and_exec_resets_it() {
        const MODE: &str = "CHARIOX_TEST_DUMPABILITY_PLACEMENT";
        if let Ok(mode) = std::env::var(MODE) {
            unsafe {
                assert_eq!(libc::prctl(libc::PR_SET_DUMPABLE, 1, 0, 0, 0), 0);
            }
            std::env::remove_var(BROKER_SOCKET_ENV);
            std::env::remove_var(BROKER_FD_ENV);
            std::env::remove_var(BROKER_REQUIRED_ENV);
            if mode == "broker" {
                std::env::set_var(BROKER_REQUIRED_ENV, "1");
            }
            initialize();
            assert_eq!(unsafe { libc::prctl(libc::PR_GET_DUMPABLE, 0, 0, 0, 0) }, 0);
            let provider = Command::new("/usr/bin/python3")
                .args([
                    "-I",
                    "-S",
                    "-c",
                    "import ctypes; print(ctypes.CDLL(None).prctl(3,0,0,0,0))",
                ])
                .output()
                .unwrap();
            assert!(provider.status.success());
            assert_eq!(
                provider.stdout, b"1\n",
                "ordinary exec must retain provider diagnostics"
            );
            return;
        }
        for mode in ["ordinary", "broker"] {
            let result = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "slice::local_docker::broker::tests::mp08_mp11_kernel_control_memory_policy_is_common_and_exec_resets_it", "--test-threads=1"])
                .env(MODE, mode).output().unwrap();
            assert!(
                result.status.success(),
                "{mode}: {}",
                String::from_utf8_lossy(&result.stdout)
            );
        }
    }

    #[test]
    fn mp08_mp11_raw_controls_use_the_same_explicit_engine_as_builds() {
        let _lock = crate::env_lock::lock();
        let previous = ["DOCKER_HOST", "DOCKER_CONTEXT"].map(|name| (name, std::env::var_os(name)));
        std::env::set_var("DOCKER_HOST", "unix:///synthetic-slice.sock");
        std::env::set_var("DOCKER_CONTEXT", "foreign-builder");
        let command = DockerCommand::default().local_command();
        for (name, value) in previous {
            if let Some(value) = value {
                std::env::set_var(name, value);
            } else {
                std::env::remove_var(name);
            }
        }
        assert_eq!(
            command.get_envs().collect::<Vec<_>>(),
            vec![(OsStr::new("DOCKER_CONTEXT"), None)]
        );
    }

    #[test]
    fn managed_provider_isolation_probe_survives_broker_filter() {
        let mut command = Command::new("unused-provisioner");
        command.env("CHARIOX_MANAGED_PROVIDER_ISOLATION_PROBE", "1");
        command.env("CHARIOX_MANAGED_UNKNOWN", "do-not-forward");
        assert_eq!(
            provisioner_environment(&command),
            BTreeMap::from([(
                "CHARIOX_MANAGED_PROVIDER_ISOLATION_PROBE".to_string(),
                "1".to_string()
            ),])
        );
    }

    #[test]
    fn managed_room_binding_survives_provisioner_request_serialization() {
        let binding = [
            ("CHARIOX_ROOM_ENVIRONMENT_HOME_KERNEL_ID", "kernel-home"),
            (
                "CHARIOX_ROOM_ENVIRONMENT_HOME_PUBLIC_KEY",
                "test-public-key",
            ),
            ("CHARIOX_ROOM_ENVIRONMENT_SESSION_ID", "room-1"),
            ("CHARIOX_ROOM_ENVIRONMENT_SLICE_ID", "slice-1"),
        ];
        let mut command = Command::new("unused-provisioner");
        command.envs(binding).env("CHARIOX_SLICE_ID", "slice-1");
        command.env("CHARIOX_ROOM_ENVIRONMENT_UNKNOWN", "do-not-forward");
        command.env("UNRELATED_SECRET", "do-not-forward");
        let environment = provisioner_environment(&command);
        let request = serde_json::to_value(BrokerRequest::Provisioner {
            action: "provision",
            environment: &environment,
            files: &[],
        })
        .unwrap();
        for (name, value) in binding {
            assert_eq!(request["environment"][name], value, "missing {name}");
        }
        assert_eq!(environment.len(), 5);
        for (name, _) in binding {
            command.env_remove(name);
        }
        assert_eq!(
            provisioner_environment(&command),
            BTreeMap::from([("CHARIOX_SLICE_ID".to_string(), "slice-1".to_string()),])
        );
    }
}

#[cfg(all(test, unix))]
#[path = "broker_archive_policy_tests.rs"]
mod archive_policy_tests;
