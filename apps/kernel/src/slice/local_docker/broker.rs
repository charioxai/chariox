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
use std::time::Duration;

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
const BROKER_IO_TIMEOUT: Duration = Duration::from_secs(21 * 60);

#[cfg(unix)]
struct BrokerConnection {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

#[cfg(unix)]
static BROKER: OnceLock<Mutex<Option<BrokerConnection>>> = OnceLock::new();
#[cfg(unix)]
static BROKER_CONFIGURED: OnceLock<bool> = OnceLock::new();

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
        let socket_path = std::env::var_os(BROKER_SOCKET_ENV);
        let inherited_fd = std::env::var(BROKER_FD_ENV)
            .ok()
            .and_then(|value| value.parse::<RawFd>().ok());
        let configured = socket_path.is_some()
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
        if configured && !make_process_nondumpable() {
            if let Some(raw_fd) = inherited_fd {
                unsafe {
                    libc::close(raw_fd);
                }
            }
            return;
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
        for _ in 0..50 {
            match UnixStream::connect(Path::new(&socket_path)) {
                Ok(writer) => {
                    if configure_stream_deadlines(&writer).is_err() {
                        return;
                    }
                    let reader = match writer.try_clone() {
                        Ok(reader) => reader,
                        Err(_) => return,
                    };
                    *state = Some(BrokerConnection {
                        reader: BufReader::new(reader),
                        writer,
                    });
                    return;
                }
                Err(_) => thread::sleep(Duration::from_millis(100)),
            }
        }
    }
}

#[cfg(unix)]
fn configure_stream_deadlines(stream: &UnixStream) -> io::Result<()> {
    stream.set_read_timeout(Some(BROKER_IO_TIMEOUT))?;
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

#[cfg(not(unix))]
fn broker_is_configured() -> bool {
    false
}

#[cfg(unix)]
fn execute(request: &BrokerRequest<'_>) -> io::Result<Output> {
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
    let result = (|| {
        let connection = state.as_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "managed slice Docker broker is unavailable",
            )
        })?;
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
        Ok(Output {
            status: ExitStatus::from_raw(response.status.clamp(0, 255) << 8),
            stdout: decode(&response.stdout_base64)?,
            stderr: decode(&response.stderr_base64)?,
        })
    })();
    if result.is_err() {
        *state = None;
    }
    result
}

pub(super) fn run_provisioner(
    command: &Command,
    action: &str,
    inputs: &[ProvisionerInput],
) -> Option<io::Result<Output>> {
    if !broker_is_configured() {
        return None;
    }
    #[cfg(unix)]
    {
        let environment = command
            .get_envs()
            .filter_map(|(name, value)| {
                let name = name.to_str()?;
                if !name.starts_with("CHARIOX_SLICE_") {
                    return None;
                }
                Some((name.to_string(), value?.to_str()?.to_string()))
            })
            .collect::<BTreeMap<_, _>>();
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
        Some(execute(&BrokerRequest::Provisioner {
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
) -> io::Result<Option<(std::path::PathBuf, u64)>> {
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
        if captured.sha256.len() != 64
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
        return Ok(Some((captured.path.into(), captured.size_bytes)));
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
        return Ok(true);
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
        return Ok(true);
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
