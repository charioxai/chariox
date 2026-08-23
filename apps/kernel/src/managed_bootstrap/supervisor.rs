#[cfg(unix)]
use std::io::{self, BufReader, Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::fd::{AsRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use rand::distributions::{Alphanumeric, DistString};

use crate::error::DaemonError;

use super::cloud::BootstrapCloudClient;
use super::release::VerifiedRelease;
use super::state::BootstrapConfig;
use super::{jittered, PendingConfirmation};

const MIN_RESTART_DELAY: Duration = Duration::from_secs(1);
const MAX_RESTART_DELAY: Duration = Duration::from_secs(30);
const MIN_CONFIRM_RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_CONFIRM_RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_CONFIRMATION_WAIT: Duration = Duration::from_secs(10 * 60);
const STABLE_RUNTIME: Duration = Duration::from_secs(30);
#[cfg(unix)]
const BROKER_SOCKET_ENV: &str = "CHARIOX_SLICE_DOCKER_BROKER_SOCKET";
#[cfg(unix)]
const BROKER_FD_ENV: &str = "CHARIOX_SLICE_DOCKER_BROKER_FD";
#[cfg(unix)]
const BROKER_REQUIRED_ENV: &str = "CHARIOX_SLICE_DOCKER_BROKER_REQUIRED";
#[cfg(unix)]
const MAX_BROKER_FRAME_BYTES: usize = 12 * 1024 * 1024;
#[cfg(unix)]
const BROKER_IO_TIMEOUT: Duration = Duration::from_secs(21 * 60);
#[cfg(unix)]
struct BrokerLease {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}
#[cfg(unix)]
static BROKER_LEASE: OnceLock<Mutex<Option<BrokerLease>>> = OnceLock::new();

pub(super) fn initialize_managed_docker_broker() {
    #[cfg(unix)]
    {
        let Some(socket) = std::env::var_os(BROKER_SOCKET_ENV) else {
            return;
        };
        let broker_lease_is_safe = make_process_nondumpable();
        let lease = BROKER_LEASE.get_or_init(|| Mutex::new(None));
        let mut lease = lease
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // The broker removes its unclaimed listener after six seconds. Waiting
        // longer ensures no endpoint can appear after bootstrap starts a
        // provider-capable kernel.
        for _ in 0..80 {
            match UnixStream::connect(&socket) {
                Ok(stream) => {
                    if broker_lease_is_safe {
                        if configure_broker_stream_deadlines(&stream).is_ok() {
                            let Ok(reader) = stream.try_clone() else {
                                return;
                            };
                            monitor_broker_lease(stream.try_clone().ok());
                            *lease = Some(BrokerLease {
                                reader: BufReader::new(reader),
                                writer: stream,
                            });
                        }
                    }
                    return;
                }
                Err(_) => thread::sleep(Duration::from_millis(100)),
            }
        }
    }
}

#[cfg(unix)]
fn configure_broker_stream_deadlines(stream: &UnixStream) -> io::Result<()> {
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

pub(super) struct KernelRun {
    status: ExitStatus,
    runtime: Duration,
}

pub(super) fn supervise_kernel(
    config: &BootstrapConfig,
    release: &VerifiedRelease,
    mut confirmation: Option<PendingConfirmation>,
    cloud: &impl BootstrapCloudClient,
) -> Result<(), DaemonError> {
    let mut restart_delay = MIN_RESTART_DELAY;
    loop {
        let started_at = Instant::now();
        match run_kernel_once(config, release, &mut confirmation, cloud) {
            Ok(run) => {
                if run.runtime >= STABLE_RUNTIME {
                    restart_delay = MIN_RESTART_DELAY;
                }
                crate::logging::warn_with_fields(
                    "managed_bootstrap.kernel_exit",
                    "managed kernel exited; supervisor will restart it",
                    serde_json::json!({
                        "status": run.status.code(),
                        "restart_delay_ms": restart_delay.as_millis(),
                    }),
                );
            }
            Err(error) => crate::logging::warn_with_fields(
                "managed_bootstrap.kernel_spawn_failed",
                "managed kernel run failed; supervisor will retry",
                serde_json::json!({
                    "error": error.to_string(),
                    "restart_delay_ms": restart_delay.as_millis(),
                }),
            ),
        }
        if started_at.elapsed() >= STABLE_RUNTIME {
            restart_delay = MIN_RESTART_DELAY;
        }
        thread::sleep(jittered(restart_delay));
        restart_delay = restart_delay.saturating_mul(2).min(MAX_RESTART_DELAY);
    }
}

pub(super) fn run_kernel_once(
    config: &BootstrapConfig,
    release: &VerifiedRelease,
    confirmation: &mut Option<PendingConfirmation>,
    cloud: &impl BootstrapCloudClient,
) -> Result<KernelRun, DaemonError> {
    let started_at = Instant::now();
    let mut child = spawn_kernel(config, release)?;
    if confirmation.is_some() {
        await_relay_ready_confirmation(config, &mut child, confirmation, cloud)?;
        terminate_child(&mut child)?;
        child = spawn_kernel(config, release)?;
    }
    let status = child
        .wait()
        .map_err(|error| supervisor_error(&format!("wait for managed kernel: {error}")))?;
    Ok(KernelRun {
        status,
        runtime: started_at.elapsed(),
    })
}

fn spawn_kernel(config: &BootstrapConfig, release: &VerifiedRelease) -> Result<Child, DaemonError> {
    let isolation_root = std::env::var_os("CHARIOX_CAPABILITY_ISOLATION_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| config.chariox_home.join("managed-context").join("kernel"));
    let provider_home = prepare_managed_provider_home(config)?;
    let local_auth_path = prepare_kernel_local_auth_file(config)?;
    let mut command = Command::new(&release.kernel_binary);
    command
        .current_dir(&config.chariox_home)
        .env("CHARIOX_HOME", &config.chariox_home)
        .env("CHARIOX_CAPABILITY_ISOLATION_ROOT", isolation_root)
        .env("CHARIOX_MANAGED_PROVIDER_ISOLATION", "1")
        .env("CHARIOX_MANAGED_PROVIDER_HOME", provider_home)
        .env(
            crate::runtime_transport::KERNEL_LOCAL_AUTH_TOKEN_FILE_ENV,
            &local_auth_path,
        )
        .env(
            "CHARIOX_MANAGED_VAULT_PATH",
            config
                .chariox_home
                .join(".chariox")
                .join("vault")
                .join("vault.json"),
        )
        .env("CHARIOX_KERNEL_HOST", &config.kernel_host)
        .env("CHARIOX_KERNEL_PORT", config.kernel_port.to_string())
        .env_remove("CHARIOX_DAEMON_ID")
        .env_remove("CHARIOX_MACHINE_ID")
        .env_remove("CHARIOX_RELAY_TOKEN")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let mut child = match spawn_with_broker_lease(&mut command) {
        Ok(child) => child,
        Err(error) => {
            let _ = std::fs::remove_file(&local_auth_path);
            return Err(supervisor_error(&format!("start managed kernel: {error}")));
        }
    };
    wait_for_local_auth_consumption(&mut child, &local_auth_path)?;
    Ok(child)
}

fn prepare_managed_provider_home(
    config: &BootstrapConfig,
) -> Result<std::path::PathBuf, DaemonError> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let path = std::env::var_os("CHARIOX_MANAGED_PROVIDER_HOME")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            config
                .chariox_home
                .parent()
                .unwrap_or(&config.chariox_home)
                .join("provider-home")
        });
    if !path.is_absolute() || path.starts_with(config.chariox_home.join(".chariox")) {
        return Err(supervisor_error(
            "managed provider HOME must be absolute and separate from kernel state",
        ));
    }
    std::fs::create_dir_all(&path)
        .map_err(|error| supervisor_error(&format!("create managed provider HOME: {error}")))?;
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|error| supervisor_error(&format!("inspect managed provider HOME: {error}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(supervisor_error(
            "managed provider HOME must be a real directory",
        ));
    }
    #[cfg(unix)]
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| supervisor_error(&format!("protect managed provider HOME: {error}")))?;
    Ok(path)
}

fn prepare_kernel_local_auth_file(
    config: &BootstrapConfig,
) -> Result<std::path::PathBuf, DaemonError> {
    let root = config.chariox_home.join("managed-runtime-auth");
    let token = Alphanumeric.sample_string(&mut rand::thread_rng(), 64);
    let path = root.join(format!(
        "kernel-{}-{}.token",
        std::process::id(),
        Alphanumeric.sample_string(&mut rand::thread_rng(), 16)
    ));
    crate::config::write_private_file(&path, token.as_bytes()).map_err(|error| {
        supervisor_error(&format!("write managed kernel local auth token: {error}"))
    })?;
    Ok(path)
}

fn wait_for_local_auth_consumption(
    child: &mut Child,
    path: &std::path::Path,
) -> Result<(), DaemonError> {
    for _ in 0..200 {
        if !path.exists() {
            return Ok(());
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| supervisor_error(&format!("poll managed kernel startup: {error}")))?
        {
            let _ = std::fs::remove_file(path);
            return Err(supervisor_error(&format!(
                "managed kernel exited before consuming local auth: {status}"
            )));
        }
        thread::sleep(Duration::from_millis(10));
    }
    let _ = terminate_child(child);
    let _ = std::fs::remove_file(path);
    Err(supervisor_error(
        "managed kernel did not consume its local auth token",
    ))
}

fn spawn_with_broker_lease(command: &mut Command) -> std::io::Result<Child> {
    #[cfg(unix)]
    {
        let lease = BROKER_LEASE.get_or_init(|| Mutex::new(None));
        if lease
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
        {
            let (kernel_stream, proxy_stream) = UnixStream::pair()?;
            let fd: RawFd = kernel_stream.as_raw_fd();
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            if flags < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0 {
                return Err(std::io::Error::last_os_error());
            }
            command
                .env_remove(BROKER_SOCKET_ENV)
                .env_remove(BROKER_REQUIRED_ENV)
                .env(BROKER_FD_ENV, fd.to_string());
            let proxy = thread::spawn(move || {
                if let Err(error) = proxy_kernel_broker(proxy_stream, lease) {
                    crate::logging::warn_with_fields(
                        "managed_bootstrap.slice_broker_lost",
                        "managed slice broker lease failed; bootstrap will restart",
                        serde_json::json!({ "error": error.to_string() }),
                    );
                    std::process::exit(1);
                }
            });
            let spawned = command.spawn();
            unsafe {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
            drop(kernel_stream);
            if spawned.is_err() {
                let _ = proxy.join();
            }
            return spawned;
        }
    }
    command
        .env_remove(BROKER_SOCKET_ENV)
        .env_remove(BROKER_FD_ENV)
        .env(BROKER_REQUIRED_ENV, "1");
    command.spawn()
}

#[cfg(unix)]
fn monitor_broker_lease(stream: Option<UnixStream>) {
    let Some(stream) = stream else { return };
    thread::spawn(move || {
        let mut descriptor = libc::pollfd {
            fd: stream.as_raw_fd(),
            events: libc::POLLHUP | libc::POLLERR | libc::POLLNVAL,
            revents: 0,
        };
        loop {
            let result = unsafe { libc::poll(&mut descriptor, 1, -1) };
            if result > 0
                && descriptor.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0
            {
                crate::logging::warn_with_fields(
                    "managed_bootstrap.slice_broker_lost",
                    "managed slice broker lease closed; bootstrap will restart",
                    serde_json::json!({}),
                );
                std::process::exit(1);
            }
            if result < 0 && std::io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                std::process::exit(1);
            }
        }
    });
}

#[cfg(unix)]
fn proxy_kernel_broker(
    local: UnixStream,
    backend: &'static Mutex<Option<BrokerLease>>,
) -> io::Result<()> {
    let mut local_reader = BufReader::new(local.try_clone()?);
    let mut local_writer = local;
    loop {
        let request = match read_broker_frame(&mut local_reader) {
            Ok(Some(request)) => request,
            Ok(None) | Err(_) => return Ok(()),
        };
        let response = {
            let mut backend = backend
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(connection) = backend.as_mut() else {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "slice broker unavailable",
                ));
            };
            let response = (|| {
                connection.writer.write_all(&request)?;
                connection.writer.flush()?;
                read_broker_frame(&mut connection.reader)?.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::UnexpectedEof, "slice broker closed")
                })
            })();
            if response.is_err() {
                *backend = None;
            }
            response?
        };
        // The backend response is always consumed before observing a dead kernel.
        // This prevents a later kernel generation from receiving a stale response.
        if local_writer.write_all(&response).is_err() || local_writer.flush().is_err() {
            return Ok(());
        }
    }
}

#[cfg(unix)]
fn read_broker_frame(reader: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut header = [0_u8; 4];
    if reader.read(&mut header[..1])? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut header[1..])?;
    let payload_len = u32::from_be_bytes(header) as usize;
    if payload_len == 0 || payload_len > MAX_BROKER_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "broker frame is invalid",
        ));
    }
    let mut frame = Vec::with_capacity(4 + payload_len);
    frame.extend_from_slice(&header);
    frame.resize(4 + payload_len, 0);
    reader.read_exact(&mut frame[4..])?;
    Ok(Some(frame))
}

#[cfg(all(test, unix))]
mod broker_proxy_tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::mpsc;

    fn bounded_stream(stream: &UnixStream) {
        let timeout = Some(Duration::from_secs(2));
        stream.set_read_timeout(timeout).unwrap();
        stream.set_write_timeout(timeout).unwrap();
    }

    fn spawn_bounded<T: Send + 'static>(
        run: impl FnOnce() -> T + Send + 'static,
    ) -> (mpsc::Receiver<T>, thread::JoinHandle<()>) {
        let (sender, receiver) = mpsc::sync_channel(1);
        let handle = thread::spawn(move || {
            let result = run();
            let _ = sender.send(result);
        });
        (receiver, handle)
    }

    fn finish_bounded<T>(receiver: mpsc::Receiver<T>, handle: thread::JoinHandle<()>) -> T {
        let result = receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("broker proxy test thread timed out");
        handle.join().expect("broker proxy test thread panicked");
        result
    }

    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut frame = (payload.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(payload);
        frame
    }

    #[test]
    fn framed_reader_accepts_split_headers_and_payloads_and_rejects_limits() {
        let expected = frame(br#"{"kind":"docker"}"#);
        for capacity in 1..=expected.len() {
            let cursor = Cursor::new(expected.clone());
            let mut reader = BufReader::with_capacity(capacity, cursor);
            assert_eq!(
                read_broker_frame(&mut reader).unwrap(),
                Some(expected.clone())
            );
            assert_eq!(read_broker_frame(&mut reader).unwrap(), None);
        }
        let mut zero = Cursor::new(0_u32.to_be_bytes());
        assert_eq!(
            read_broker_frame(&mut zero).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let mut large = Cursor::new(((MAX_BROKER_FRAME_BYTES + 1) as u32).to_be_bytes());
        assert_eq!(
            read_broker_frame(&mut large).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let mut partial = Cursor::new([0_u8, 0, 0, 4, b'a']);
        assert_eq!(
            read_broker_frame(&mut partial).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn proxy_discards_partial_generation_and_drains_lost_response() {
        let (backend_client, broker) = UnixStream::pair().unwrap();
        bounded_stream(&backend_client);
        bounded_stream(&broker);
        let backend_reader = backend_client.try_clone().unwrap();
        let backend: &'static Mutex<Option<BrokerLease>> =
            Box::leak(Box::new(Mutex::new(Some(BrokerLease {
                reader: BufReader::new(backend_reader),
                writer: backend_client,
            }))));
        let (broker_result, broker_thread) = spawn_bounded(move || {
            let mut reader = BufReader::new(broker.try_clone().unwrap());
            let mut writer = broker;
            for response in [b"first".as_slice(), b"second".as_slice()] {
                let request = read_broker_frame(&mut reader).unwrap().unwrap();
                assert!(request.len() > 4);
                writer.write_all(&frame(response)).unwrap();
                writer.flush().unwrap();
            }
        });

        let (mut killed_kernel, first_proxy) = UnixStream::pair().unwrap();
        bounded_stream(&killed_kernel);
        bounded_stream(&first_proxy);
        let (first_result, first) =
            spawn_bounded(move || proxy_kernel_broker(first_proxy, backend));
        killed_kernel.write_all(&frame(b"request-one")).unwrap();
        drop(killed_kernel);
        assert!(finish_bounded(first_result, first).is_ok());

        let (mut partial_kernel, partial_proxy) = UnixStream::pair().unwrap();
        bounded_stream(&partial_kernel);
        bounded_stream(&partial_proxy);
        let (partial_result, partial) =
            spawn_bounded(move || proxy_kernel_broker(partial_proxy, backend));
        partial_kernel.write_all(&[0, 0, 0, 8, b'x']).unwrap();
        drop(partial_kernel);
        assert!(finish_bounded(partial_result, partial).is_ok());

        let (mut next_kernel, next_proxy) = UnixStream::pair().unwrap();
        bounded_stream(&next_kernel);
        bounded_stream(&next_proxy);
        let (next_result, next) = spawn_bounded(move || proxy_kernel_broker(next_proxy, backend));
        next_kernel.write_all(&frame(b"request-two")).unwrap();
        let mut response_reader = BufReader::new(next_kernel.try_clone().unwrap());
        assert_eq!(
            read_broker_frame(&mut response_reader).unwrap(),
            Some(frame(b"second"))
        );
        drop(response_reader);
        drop(next_kernel);
        assert!(finish_bounded(next_result, next).is_ok());
        finish_bounded(broker_result, broker_thread);
    }

    #[test]
    fn proxy_drops_a_stalled_backend_lease_after_its_transport_deadline() {
        let (backend_client, stalled_broker) = UnixStream::pair().unwrap();
        backend_client
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        backend_client
            .set_write_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let backend_reader = backend_client.try_clone().unwrap();
        let backend: &'static Mutex<Option<BrokerLease>> =
            Box::leak(Box::new(Mutex::new(Some(BrokerLease {
                reader: BufReader::new(backend_reader),
                writer: backend_client,
            }))));
        let (mut kernel, proxy) = UnixStream::pair().unwrap();
        bounded_stream(&kernel);
        let (proxy_result, proxy_thread) =
            spawn_bounded(move || proxy_kernel_broker(proxy, backend));

        let started = Instant::now();
        kernel
            .write_all(&frame(b"request-without-response"))
            .unwrap();
        let error = finish_bounded(proxy_result, proxy_thread)
            .expect_err("stalled broker response must fail the proxy generation");
        assert!(matches!(
            error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(backend
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_none());

        drop(stalled_broker);
    }
}

fn await_relay_ready_confirmation(
    config: &BootstrapConfig,
    child: &mut Child,
    confirmation: &mut Option<PendingConfirmation>,
    cloud: &impl BootstrapCloudClient,
) -> Result<(), DaemonError> {
    let started_at = Instant::now();
    let mut retry_delay = MIN_CONFIRM_RETRY_DELAY;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| supervisor_error(&format!("inspect managed kernel: {error}")))?
        {
            return Err(supervisor_error(&format!(
                "managed kernel exited before relay-ready confirmation with status {status}"
            )));
        }
        let pending = confirmation
            .as_ref()
            .ok_or_else(|| supervisor_error("managed confirmation state disappeared"))?;
        match pending.confirm(config, cloud, Utc::now()) {
            Ok(()) => {
                confirmation.take();
                return Ok(());
            }
            Err(error) => {
                crate::logging::warn_with_fields(
                    "managed_bootstrap.confirm_pending",
                    "managed kernel is not relay-ready; confirmation will retry",
                    serde_json::json!({
                        "error": error.to_string(),
                        "retry_delay_ms": retry_delay.as_millis(),
                    }),
                );
            }
        }
        if started_at.elapsed() >= MAX_CONFIRMATION_WAIT {
            terminate_child(child)?;
            return Err(supervisor_error(
                "managed kernel did not establish relay presence before the confirmation deadline",
            ));
        }
        thread::sleep(jittered(retry_delay));
        retry_delay = retry_delay.saturating_mul(2).min(MAX_CONFIRM_RETRY_DELAY);
    }
}

fn terminate_child(child: &mut Child) -> Result<(), DaemonError> {
    if child
        .try_wait()
        .map_err(|error| supervisor_error(&format!("inspect managed kernel before stop: {error}")))?
        .is_some()
    {
        return Ok(());
    }
    if let Err(kill_error) = child.kill() {
        if child
            .try_wait()
            .map_err(|error| {
                supervisor_error(&format!(
                    "inspect managed kernel after stop failure ({kill_error}): {error}"
                ))
            })?
            .is_some()
        {
            return Ok(());
        }
        return Err(supervisor_error(&format!(
            "stop managed kernel before operational restart: {kill_error}"
        )));
    }
    child
        .wait()
        .map_err(|error| supervisor_error(&format!("reap stopped managed kernel: {error}")))?;
    Ok(())
}

fn supervisor_error(message: &str) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "supervise managed kernel",
        message: message.to_string(),
    }
}
