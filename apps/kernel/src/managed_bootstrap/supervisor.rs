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
#[cfg(test)]
use super::state::BootstrapReceiptStatus;
use super::state::{BootstrapConfig, BootstrapReceipt};
use super::{
    jittered, managed_provider_topology, ManagedProviderTopology, PendingConfirmation,
    MANAGED_PROVIDER_TOPOLOGY_ENV, PATH1_SHARED_HOST_SELECTOR_ENVS,
};

const MIN_RESTART_DELAY: Duration = Duration::from_secs(1);
const MAX_RESTART_DELAY: Duration = Duration::from_secs(30);
const MIN_CONFIRM_RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_CONFIRM_RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_CONFIRMATION_WAIT: Duration = Duration::from_secs(10 * 60);
const STABLE_RUNTIME: Duration = Duration::from_secs(30);
const BROKER_SOCKET_ENV: &str = "CHARIOX_SLICE_DOCKER_BROKER_SOCKET";
#[cfg(unix)]
const BROKER_FD_ENV: &str = "CHARIOX_SLICE_DOCKER_BROKER_FD";
#[cfg(unix)]
const BROKER_REQUIRED_ENV: &str = "CHARIOX_SLICE_DOCKER_BROKER_REQUIRED";
const DEFAULT_MANAGED_SLICE_SERVICE_ROOT: &str = "/var/lib/chariox-slice-share";
const DEFAULT_MANAGED_SLICE_PUBLICATION_ROOT: &str = "/var/lib/chariox-slice-share/slices";
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
                    if broker_lease_is_safe && configure_broker_stream_deadlines(&stream).is_ok() {
                        let Ok(reader) = stream.try_clone() else {
                            return;
                        };
                        monitor_broker_lease(stream.try_clone().ok());
                        *lease = Some(BrokerLease {
                            reader: BufReader::new(reader),
                            writer: stream,
                        });
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
    topology: ManagedProviderTopology,
) -> Result<(), DaemonError> {
    let mut restart_delay = MIN_RESTART_DELAY;
    loop {
        let started_at = Instant::now();
        match run_kernel_once(config, release, &mut confirmation, cloud, topology) {
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
    topology: ManagedProviderTopology,
) -> Result<KernelRun, DaemonError> {
    let started_at = Instant::now();
    let mut child = spawn_kernel(config, release, topology)?;
    if confirmation.is_some() {
        await_relay_ready_confirmation(config, &mut child, confirmation, cloud)?;
        terminate_child(&mut child)?;
        child = spawn_kernel(config, release, topology)?;
    }
    let status = child
        .wait()
        .map_err(|error| supervisor_error(&format!("wait for managed kernel: {error}")))?;
    Ok(KernelRun {
        status,
        runtime: started_at.elapsed(),
    })
}

fn spawn_kernel(
    config: &BootstrapConfig,
    release: &VerifiedRelease,
    topology: ManagedProviderTopology,
) -> Result<Child, DaemonError> {
    spawn_kernel_with_handoff(config, release, topology).map(|(child, _)| child)
}

fn spawn_kernel_with_handoff(
    config: &BootstrapConfig,
    release: &VerifiedRelease,
    topology: ManagedProviderTopology,
) -> Result<(Child, Option<i32>), DaemonError> {
    let managed_repository_root = BootstrapReceipt::read(&config.receipt_path)?
        .ok_or_else(|| {
            supervisor_error("managed bootstrap receipt is missing before kernel launch")
        })?
        .managed_repository_root()?;
    let (isolation_root, service_root, publication_root) = match topology {
        ManagedProviderTopology::Path1 => (None, None, None),
        ManagedProviderTopology::SharedHost => {
            let (service_root, publication_root) = managed_slice_boundaries_for_kernel()?;
            (
                Some(
                    std::env::var_os("CHARIOX_CAPABILITY_ISOLATION_ROOT")
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| {
                            config.chariox_home.join("managed-context").join("kernel")
                        }),
                ),
                service_root,
                publication_root,
            )
        }
    };
    let provider_home = prepare_managed_provider_home(config)?;
    let local_auth_path = prepare_kernel_local_auth_file(config)?;
    let mut command = Command::new(&release.kernel_binary);
    command
        .current_dir(&config.process_home)
        .env("HOME", &config.process_home)
        .env("CHARIOX_HOME", &config.chariox_home)
        .env(super::MANAGED_REPOSITORY_ROOT_ENV, managed_repository_root)
        .env("CHARIOX_MANAGED_PROVIDER_HOME", provider_home)
        .env(
            crate::runtime_transport::KERNEL_LOCAL_AUTH_TOKEN_FILE_ENV,
            &local_auth_path,
        )
        .env(
            "CHARIOX_MANAGED_VAULT_PATH",
            config.chariox_home.join("vault").join("vault.json"),
        )
        .env("CHARIOX_MANAGED_BOOTSTRAP_RECEIPT", &config.receipt_path)
        .env("CHARIOX_KERNEL_HOST", &config.kernel_host)
        .env("CHARIOX_KERNEL_PORT", config.kernel_port.to_string())
        .env(MANAGED_PROVIDER_TOPOLOGY_ENV, topology.as_str())
        .env_remove("CHARIOX_DAEMON_ID")
        .env_remove("CHARIOX_MACHINE_ID")
        .env_remove("CHARIOX_RELAY_TOKEN")
        .env_remove(super::worker::ACTIVITY_RECEIPT_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Some(isolation_root) = isolation_root {
        command
            .env("CHARIOX_CAPABILITY_ISOLATION_ROOT", isolation_root)
            .env("CHARIOX_MANAGED_PROVIDER_ISOLATION", "1");
    } else {
        for name in PATH1_SHARED_HOST_SELECTOR_ENVS {
            command.env_remove(name);
        }
    }
    if let Some(service_root) = service_root.as_ref() {
        command.env(
            crate::provider::MANAGED_SLICE_SERVICE_ROOT_ENV,
            service_root,
        );
    } else {
        command.env_remove(crate::provider::MANAGED_SLICE_SERVICE_ROOT_ENV);
    }
    if let Some(publication_root) = publication_root {
        command.env(
            crate::provider::MANAGED_SLICE_PUBLICATION_ROOT_ENV,
            publication_root,
        );
    } else {
        command.env_remove(crate::provider::MANAGED_SLICE_PUBLICATION_ROOT_ENV);
    }
    let spawn_result = match topology {
        ManagedProviderTopology::Path1 => command.spawn().map(|child| (child, None)),
        ManagedProviderTopology::SharedHost => spawn_with_broker_lease(&mut command),
    };
    let (mut child, handed_off_fd) = match spawn_result {
        Ok(child) => child,
        Err(error) => {
            let _ = std::fs::remove_file(&local_auth_path);
            return Err(supervisor_error(&format!("start managed kernel: {error}")));
        }
    };
    wait_for_local_auth_consumption(&mut child, &local_auth_path)?;
    Ok((child, handed_off_fd))
}

fn configured_managed_slice_boundary(
    name: &str,
) -> Result<Option<std::path::PathBuf>, DaemonError> {
    let Some(raw) = std::env::var_os(name).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let path = std::path::PathBuf::from(raw);
    if !path.is_absolute() || path == std::path::Path::new("/") {
        return Err(supervisor_error(&format!(
            "{name} must be an absolute non-root path"
        )));
    }
    Ok(Some(path))
}

fn broker_share_root_from_socket() -> Option<std::path::PathBuf> {
    let socket = std::env::var_os(BROKER_SOCKET_ENV)
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)?;
    if !socket.is_absolute() {
        return None;
    }
    socket
        .parent()?
        .parent()?
        .parent()
        .map(std::path::Path::to_path_buf)
}

fn managed_slice_boundaries_for_kernel(
) -> Result<(Option<std::path::PathBuf>, Option<std::path::PathBuf>), DaemonError> {
    let mut service_root =
        configured_managed_slice_boundary(crate::provider::MANAGED_SLICE_SERVICE_ROOT_ENV)?;
    let mut publication_root =
        configured_managed_slice_boundary(crate::provider::MANAGED_SLICE_PUBLICATION_ROOT_ENV)?;
    let slice_root = std::env::var_os("CHARIOX_SLICE_ROOT")
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from);
    let broker_share_root = broker_share_root_from_socket();

    // This compatibility derivation happens in the supervisor while the
    // service still has its socket configuration. The kernel isolation policy
    // receives the resulting invariant and never derives publication scope
    // from transport availability after the handoff.
    if service_root.is_none() {
        service_root = broker_share_root.clone();
    }
    if publication_root.is_none() {
        if let (Some(slice_root), Some(broker_share_root)) =
            (slice_root.as_ref(), broker_share_root.as_ref())
        {
            if slice_root.parent() == Some(broker_share_root.as_path()) {
                publication_root = Some(slice_root.clone());
            }
        }
    }

    // Safe compatibility for the original host layout when no broker socket
    // is available (the required-fallback path).
    if publication_root.is_none()
        && slice_root.as_deref()
            == Some(std::path::Path::new(DEFAULT_MANAGED_SLICE_PUBLICATION_ROOT))
    {
        publication_root = Some(std::path::PathBuf::from(
            DEFAULT_MANAGED_SLICE_PUBLICATION_ROOT,
        ));
    }
    if service_root.is_none()
        && publication_root.as_deref()
            == Some(std::path::Path::new(DEFAULT_MANAGED_SLICE_PUBLICATION_ROOT))
    {
        service_root = Some(std::path::PathBuf::from(DEFAULT_MANAGED_SLICE_SERVICE_ROOT));
    }
    Ok((service_root, publication_root))
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
    if !path.is_absolute()
        || path == std::path::Path::new("/")
        || path == config.chariox_home
        || path.starts_with(&config.chariox_home)
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
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

fn spawn_with_broker_lease(command: &mut Command) -> std::io::Result<(Child, Option<i32>)> {
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
            return spawned.map(|child| (child, Some(fd)));
        }
    }
    command
        .env_remove(BROKER_SOCKET_ENV)
        .env_remove(BROKER_FD_ENV)
        .env(BROKER_REQUIRED_ENV, "1");
    command.spawn().map(|child| (child, None))
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

    fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }

    #[test]
    fn managed_provider_topology_is_explicit_and_fail_closed() {
        let _env = crate::env_lock::lock();
        let previous = std::env::var_os(MANAGED_PROVIDER_TOPOLOGY_ENV);

        std::env::remove_var(MANAGED_PROVIDER_TOPOLOGY_ENV);
        assert!(managed_provider_topology()
            .expect_err("missing topology must fail closed")
            .to_string()
            .contains("explicitly set"));
        std::env::set_var(MANAGED_PROVIDER_TOPOLOGY_ENV, "unknown");
        assert!(managed_provider_topology()
            .expect_err("unknown topology must fail closed")
            .to_string()
            .contains("must be path1 or shared_host"));
        std::env::set_var(MANAGED_PROVIDER_TOPOLOGY_ENV, "path1");
        assert_eq!(
            managed_provider_topology().unwrap(),
            ManagedProviderTopology::Path1
        );
        std::env::set_var(MANAGED_PROVIDER_TOPOLOGY_ENV, "shared_host");
        assert_eq!(
            managed_provider_topology().unwrap(),
            ManagedProviderTopology::SharedHost
        );

        restore_env(MANAGED_PROVIDER_TOPOLOGY_ENV, previous);
    }

    #[test]
    fn managed_and_path1_kernel_children_keep_their_launch_boundaries_separate() {
        let _env = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-managed-supervisor-boundary-contract-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let home = root.join("home");
        let kernel = root.join("bin/kernel");
        let fallback_record = root.join("fallback.env");
        let fd_record = root.join("fd.env");
        let path1_record = root.join("path1.env");
        let service_root = root.join("relocated-share");
        let publication_root = service_root.join("configured-publications");
        std::fs::create_dir_all(kernel.parent().expect("kernel parent"))
            .expect("kernel parent should exist");
        std::fs::create_dir_all(&home).expect("kernel home should exist");
        std::fs::create_dir_all(&publication_root).expect("publication root should exist");
        let script = r##"#!/bin/sh
{
  printf 'home=%s\n' "${HOME-}"
  printf 'chariox_home=%s\n' "${CHARIOX_HOME-}"
  printf 'repository_root=%s\n' "${CHARIOX_MANAGED_REPOSITORY_ROOT-}"
  printf 'cwd=%s\n' "$(pwd)"
  printf 'topology=%s\n' "${CHARIOX_MANAGED_PROVIDER_TOPOLOGY-<unset>}"
  printf 'capability_root=%s\n' "${CHARIOX_CAPABILITY_ISOLATION_ROOT-<unset>}"
  printf 'provider_isolation=%s\n' "${CHARIOX_MANAGED_PROVIDER_ISOLATION-<unset>}"
  printf 'provider_isolation_active=%s\n' "${CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE-<unset>}"
  printf 'provider_bwrap=%s\n' "${CHARIOX_MANAGED_PROVIDER_BWRAP-<unset>}"
  printf 'vault=%s\n' "${CHARIOX_MANAGED_VAULT_PATH-}"
  printf 'service=%s\n' "${CHARIOX_MANAGED_SLICE_SERVICE_ROOT-<unset>}"
  printf 'publication=%s\n' "${CHARIOX_MANAGED_SLICE_PUBLICATION_ROOT-<unset>}"
  printf 'socket=%s\n' "${CHARIOX_SLICE_DOCKER_BROKER_SOCKET-<unset>}"
  printf 'fd=%s\n' "${CHARIOX_SLICE_DOCKER_BROKER_FD-<unset>}"
  printf 'required=%s\n' "${CHARIOX_SLICE_DOCKER_BROKER_REQUIRED-<unset>}"
} > "$CHARIOX_ENV_RECORD"
rm -f -- "$CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE"
"##;
        std::fs::write(&kernel, script).expect("kernel fixture should write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&kernel, std::fs::Permissions::from_mode(0o755))
                .expect("kernel fixture should be executable");
        }

        let previous_service = std::env::var_os(crate::provider::MANAGED_SLICE_SERVICE_ROOT_ENV);
        let previous_publication =
            std::env::var_os(crate::provider::MANAGED_SLICE_PUBLICATION_ROOT_ENV);
        let previous_slice_root = std::env::var_os("CHARIOX_SLICE_ROOT");
        let previous_socket = std::env::var_os(BROKER_SOCKET_ENV);
        let previous_fd = std::env::var_os(BROKER_FD_ENV);
        let previous_required = std::env::var_os(BROKER_REQUIRED_ENV);
        let previous_topology = std::env::var_os(MANAGED_PROVIDER_TOPOLOGY_ENV);
        let previous_capability_root = std::env::var_os("CHARIOX_CAPABILITY_ISOLATION_ROOT");
        let previous_provider_isolation = std::env::var_os("CHARIOX_MANAGED_PROVIDER_ISOLATION");
        let previous_provider_isolation_active =
            std::env::var_os("CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE");
        let previous_provider_bwrap = std::env::var_os("CHARIOX_MANAGED_PROVIDER_BWRAP");
        let previous_record = std::env::var_os("CHARIOX_ENV_RECORD");
        std::env::set_var(MANAGED_PROVIDER_TOPOLOGY_ENV, "shared_host");
        std::env::set_var(
            crate::provider::MANAGED_SLICE_SERVICE_ROOT_ENV,
            &service_root,
        );
        std::env::set_var(
            crate::provider::MANAGED_SLICE_PUBLICATION_ROOT_ENV,
            &publication_root,
        );
        std::env::set_var("CHARIOX_SLICE_ROOT", &publication_root);
        std::env::set_var(
            BROKER_SOCKET_ENV,
            service_root.join(".private-control/control/control.sock"),
        );
        std::env::remove_var(BROKER_FD_ENV);
        std::env::remove_var(BROKER_REQUIRED_ENV);
        std::env::set_var("CHARIOX_MANAGED_PROVIDER_ISOLATION", "1");

        let config = BootstrapConfig {
            process_home: home.clone(),
            chariox_home: home.join(".chariox"),
            envelope_path: root.join("managed-bootstrap.json"),
            receipt_path: root.join("bootstrap-receipt.json"),
            manifest_path: root.join("release-manifest.json"),
            signature_path: root.join("release-manifest.sig"),
            public_key_path: root.join("release-public-key"),
            kernel_binary: kernel.clone(),
            kernel_host: "127.0.0.1".to_string(),
            kernel_port: 43118,
        };
        let release = VerifiedRelease {
            digest: "test".to_string(),
            kernel_binary: kernel,
        };
        BootstrapReceipt {
            schema_version: 2,
            status: BootstrapReceiptStatus::Confirmed,
            environment_id: "managed-env-1".to_string(),
            machine_id: "managed-machine-1".to_string(),
            kernel_id: "managed-kernel-1".to_string(),
            generation: 1,
            relay_public_key: "managed-public-key".to_string(),
            runtime_release_digest: format!("sha256:{}", "a".repeat(64)),
            managed_repository_root: Some("/srv/managed workspaces".to_string()),
            confirmed_at: Some("2026-09-20T20:00:00Z".to_string()),
            context_plan: None,
            provider_rebuild_action_id: None,
            freshness_evidence: None,
        }
        .persist(&config.receipt_path)
        .expect("managed receipt should persist");

        // No broker lease exercises the required-fallback path. The
        // supervisor strips transport variables, but the explicit service
        // boundary must still be present in the child environment.
        let lease = BROKER_LEASE.get_or_init(|| Mutex::new(None));
        assert!(lease.lock().expect("broker lease").is_none());
        std::env::set_var("CHARIOX_ENV_RECORD", &fallback_record);
        let mut child = spawn_kernel(&config, &release, ManagedProviderTopology::SharedHost)
            .expect("fallback kernel should spawn");
        child.wait().expect("fallback kernel should exit");
        let fallback = std::fs::read_to_string(&fallback_record).expect("fallback env record");
        let canonical_home = home
            .canonicalize()
            .expect("kernel home should canonicalize");
        assert!(fallback.contains(&format!("home={}\n", home.display())));
        assert!(fallback.contains(&format!(
            "chariox_home={}\n",
            home.join(".chariox").display()
        )));
        assert!(fallback.contains(&format!("cwd={}\n", canonical_home.display())));
        assert!(fallback.contains("repository_root=/srv/managed workspaces\n"));
        assert!(fallback.contains(&format!(
            "vault={}\n",
            home.join(".chariox/vault/vault.json").display()
        )));
        assert!(fallback.contains("topology=shared_host\n"));
        assert!(fallback.contains("provider_isolation=1\n"));
        assert!(fallback.contains(&format!("service={}\n", service_root.display())));
        assert!(fallback.contains(&format!("publication={}\n", publication_root.display())));
        assert!(fallback.contains("socket=<unset>\n"));
        assert!(fallback.contains("fd=<unset>\n"));
        assert!(fallback.contains("required=1\n"));

        // Install a test-only broker lease and run the same real supervisor
        // spawn path. This is the production FD handoff shape: the socket is
        // removed, an inherited FD is supplied, and policy remains separate.
        let (backend, _backend_peer) = UnixStream::pair().expect("broker lease pair");
        let backend_reader = backend.try_clone().expect("broker reader clone");
        *lease.lock().expect("broker lease") = Some(BrokerLease {
            reader: BufReader::new(backend_reader),
            writer: backend,
        });
        std::env::set_var("CHARIOX_ENV_RECORD", &fd_record);
        let handed_off_fd = {
            let (mut child, handed_off_fd) =
                spawn_kernel_with_handoff(&config, &release, ManagedProviderTopology::SharedHost)
                    .expect("FD kernel should spawn");
            child.wait().expect("FD kernel should exit");
            handed_off_fd.expect("broker lease should hand off an FD")
        };
        let fd = std::fs::read_to_string(&fd_record).expect("FD env record");
        assert!(fd.contains(&format!("service={}\n", service_root.display())));
        assert!(fd.contains(&format!("publication={}\n", publication_root.display())));
        assert!(fd.contains("socket=<unset>\n"));
        assert!(fd.contains(&format!("fd={handed_off_fd}\n")));
        assert!(fd.contains("required=<unset>\n"));
        *lease.lock().expect("broker lease") = None;

        // A Path-1 launch must ignore inherited shared-host selectors and must
        // not turn a stale broker lease/configuration into a child requirement.
        std::env::set_var(MANAGED_PROVIDER_TOPOLOGY_ENV, "path1");
        std::env::set_var(
            "CHARIOX_CAPABILITY_ISOLATION_ROOT",
            root.join("stale-capability-root"),
        );
        std::env::set_var("CHARIOX_MANAGED_PROVIDER_ISOLATION", "1");
        std::env::set_var("CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE", "1");
        std::env::set_var("CHARIOX_MANAGED_PROVIDER_BWRAP", "/usr/bin/bwrap");
        std::env::set_var(
            crate::provider::MANAGED_SLICE_SERVICE_ROOT_ENV,
            &service_root,
        );
        std::env::set_var(
            crate::provider::MANAGED_SLICE_PUBLICATION_ROOT_ENV,
            &publication_root,
        );
        std::env::set_var("CHARIOX_SLICE_ROOT", &publication_root);
        std::env::set_var(
            BROKER_SOCKET_ENV,
            service_root.join(".private-control/control/control.sock"),
        );
        std::env::set_var(BROKER_FD_ENV, "99");
        std::env::set_var(BROKER_REQUIRED_ENV, "1");
        std::env::set_var("CHARIOX_ENV_RECORD", &path1_record);
        let mut child = spawn_kernel(&config, &release, ManagedProviderTopology::Path1)
            .expect("Path-1 kernel should spawn");
        child.wait().expect("Path-1 kernel should exit");
        let path1 = std::fs::read_to_string(&path1_record).expect("Path-1 env record");
        assert!(path1.contains(&format!("home={}\n", home.display())));
        assert!(path1.contains(&format!(
            "chariox_home={}\n",
            home.join(".chariox").display()
        )));
        assert!(path1.contains(&format!("cwd={}\n", canonical_home.display())));
        assert!(path1.contains("repository_root=/srv/managed workspaces\n"));
        assert!(path1.contains("topology=path1\n"));
        assert!(path1.contains("capability_root=<unset>\n"));
        assert!(path1.contains("provider_isolation=<unset>\n"));
        assert!(path1.contains("provider_isolation_active=<unset>\n"));
        assert!(path1.contains("provider_bwrap=<unset>\n"));
        assert!(path1.contains("service=<unset>\n"));
        assert!(path1.contains("publication=<unset>\n"));
        assert!(path1.contains("socket=<unset>\n"));
        assert!(path1.contains("fd=<unset>\n"));
        assert!(path1.contains("required=<unset>\n"));

        restore_env(
            crate::provider::MANAGED_SLICE_SERVICE_ROOT_ENV,
            previous_service,
        );
        restore_env(
            crate::provider::MANAGED_SLICE_PUBLICATION_ROOT_ENV,
            previous_publication,
        );
        restore_env("CHARIOX_SLICE_ROOT", previous_slice_root);
        restore_env(BROKER_SOCKET_ENV, previous_socket);
        restore_env(BROKER_FD_ENV, previous_fd);
        restore_env(BROKER_REQUIRED_ENV, previous_required);
        restore_env(MANAGED_PROVIDER_TOPOLOGY_ENV, previous_topology);
        restore_env(
            "CHARIOX_CAPABILITY_ISOLATION_ROOT",
            previous_capability_root,
        );
        restore_env(
            "CHARIOX_MANAGED_PROVIDER_ISOLATION",
            previous_provider_isolation,
        );
        restore_env(
            "CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE",
            previous_provider_isolation_active,
        );
        restore_env("CHARIOX_MANAGED_PROVIDER_BWRAP", previous_provider_bwrap);
        restore_env("CHARIOX_ENV_RECORD", previous_record);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(target_os = "linux")]
    struct Path1ConfirmationCloud;

    #[cfg(target_os = "linux")]
    impl BootstrapCloudClient for Path1ConfirmationCloud {
        fn exchange(
            &self,
            _: &str,
            _: &super::super::cloud::ExchangeRequest,
        ) -> Result<super::super::cloud::ExchangeResponse, DaemonError> {
            panic!("confirmation restart must not exchange credentials")
        }

        fn confirm(
            &self,
            _: &str,
            _: &super::super::cloud::ConfirmRequest,
        ) -> Result<super::super::cloud::ConfirmResponse, DaemonError> {
            Ok(super::super::cloud::ConfirmResponse {
                confirmed: true,
                observed_state: "awaiting_context".to_string(),
                managed_repository_root: None,
            })
        }

        fn report_runtime_identity(
            &self,
            _: &str,
            _: &super::super::freshness::ManagedKernelRuntimeIdentityReport,
        ) -> Result<super::super::cloud::RuntimeIdentityReportResponse, DaemonError> {
            panic!("confirmation restart must not report reimage identity")
        }
    }

    #[cfg(target_os = "linux")]
    fn path1_confirmation_capture_script() -> String {
        let mut script = String::from(
            "#!/bin/sh\n\
             set -eu\n\
             capture=\"${CHARIOX_TEST_PATH1_CONFIRM_CAPTURE:?}\"\n\
             if /bin/mkdir \"$capture.first\" 2>/dev/null; then generation=1; else generation=2; fi\n\
             record=\"$capture.$generation\"\n\
             {\n\
               printf 'home=%s\\n' \"${HOME-<unset>}\"\n\
               printf 'path=%s\\n' \"${PATH-<unset>}\"\n\
               printf 'cwd=%s\\n' \"$(pwd)\"\n\
               printf 'chariox_home=%s\\n' \"${CHARIOX_HOME-<unset>}\"\n\
               printf 'repository_root=%s\\n' \"${CHARIOX_MANAGED_REPOSITORY_ROOT-<unset>}\"\n\
               printf 'topology=%s\\n' \"${CHARIOX_MANAGED_PROVIDER_TOPOLOGY-<unset>}\"\n\
               printf 'provider_home=%s\\n' \"${CHARIOX_MANAGED_PROVIDER_HOME-<unset>}\"\n\
               printf 'vault=%s\\n' \"${CHARIOX_MANAGED_VAULT_PATH-<unset>}\"\n",
        );
        for name in PATH1_SHARED_HOST_SELECTOR_ENVS {
            script.push_str(&format!(
                "  printf '{name}=%s\\n' \"${{{name}-<unset>}}\"\n"
            ));
        }
        script.push_str(
            "} > \"$record.tmp\"\n\
             /bin/mv \"$record.tmp\" \"$record\"\n\
             rm -f -- \"$CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE\"\n\
             if [ \"$generation\" = 1 ]; then exec /bin/sleep 30; fi\n",
        );
        script
    }

    #[cfg(target_os = "linux")]
    fn assert_path1_confirmation_capture(
        path: &std::path::Path,
        process_home: &std::path::Path,
        chariox_home: &std::path::Path,
        provider_home: &std::path::Path,
        path_value: &str,
    ) {
        let observed =
            std::fs::read_to_string(path).expect("confirmation child should record its boundary");
        for expected in [
            format!("home={}\n", process_home.display()),
            format!("path={path_value}\n"),
            format!("cwd={}\n", process_home.display()),
            format!("chariox_home={}\n", chariox_home.display()),
            "repository_root=/home/chariox\n".to_string(),
            "topology=path1\n".to_string(),
            format!("provider_home={}\n", provider_home.display()),
            format!(
                "vault={}\n",
                chariox_home.join("vault/vault.json").display()
            ),
        ] {
            assert!(
                observed.contains(&expected),
                "missing `{}` from {}: {observed}",
                expected.trim(),
                path.display()
            );
        }
        for name in PATH1_SHARED_HOST_SELECTOR_ENVS {
            assert!(
                observed.contains(&format!("{name}=<unset>\n")),
                "confirmation child inherited {name}: {observed}"
            );
        }
    }

    #[cfg(target_os = "linux")]
    struct ConfirmationFixtureRoot(std::path::PathBuf);

    #[cfg(target_os = "linux")]
    impl Drop for ConfirmationFixtureRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(target_os = "linux")]
    struct Path1TestEnvironment(Vec<(&'static str, Option<std::ffi::OsString>)>);

    #[cfg(target_os = "linux")]
    impl Path1TestEnvironment {
        fn capture(names: &[&'static str]) -> Self {
            Self(
                names
                    .iter()
                    .map(|name| (*name, std::env::var_os(name)))
                    .collect(),
            )
        }
    }

    #[cfg(target_os = "linux")]
    impl Drop for Path1TestEnvironment {
        fn drop(&mut self) {
            for (name, value) in self.0.drain(..) {
                restore_env(name, value);
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn path1_confirmation_restart_reapplies_ordinary_boundary() {
        use std::os::unix::fs::PermissionsExt;

        let _env = crate::env_lock::lock();
        let root = std::env::temp_dir().join(format!(
            "chariox-path1-confirmation-boundary-{}-{}",
            std::process::id(),
            crate::session::unix_epoch_ms()
        ));
        let _fixture_root = ConfirmationFixtureRoot(root.clone());
        let process_home = root.join("home");
        let chariox_home = process_home.join(".chariox");
        let provider_home = root.join("provider-home");
        let kernel_binary = root.join("bin/chariox-kernel");
        let capture = root.join("kernel-boundary");
        std::fs::create_dir_all(&chariox_home).expect("Path-1 test HOME should exist");
        std::fs::create_dir_all(kernel_binary.parent().expect("kernel parent should exist"))
            .expect("kernel parent should be created");
        std::fs::write(&kernel_binary, path1_confirmation_capture_script())
            .expect("confirmation probe kernel should be written");
        std::fs::set_permissions(&kernel_binary, std::fs::Permissions::from_mode(0o755))
            .expect("confirmation probe kernel should be executable");

        let config = BootstrapConfig {
            process_home: process_home.clone(),
            chariox_home: chariox_home.clone(),
            envelope_path: root.join("managed-bootstrap.json"),
            receipt_path: chariox_home.join("managed/bootstrap-receipt.json"),
            manifest_path: root.join("release-manifest.json"),
            signature_path: root.join("release-manifest.sig"),
            public_key_path: root.join("release-public-key"),
            kernel_binary: kernel_binary.clone(),
            kernel_host: "127.0.0.1".to_string(),
            kernel_port: 43118,
        };
        let runtime_release_digest = format!("sha256:{}", "c".repeat(64));
        let receipt = BootstrapReceipt {
            schema_version: 1,
            status: BootstrapReceiptStatus::Exchanged,
            environment_id: "environment-1".to_string(),
            machine_id: "machine-1".to_string(),
            kernel_id: "kernel-1".to_string(),
            generation: 1,
            relay_public_key: "relay-public-key".to_string(),
            runtime_release_digest: runtime_release_digest.clone(),
            managed_repository_root: None,
            confirmed_at: None,
            context_plan: None,
            provider_rebuild_action_id: None,
            freshness_evidence: None,
        };
        receipt
            .persist(&config.receipt_path)
            .expect("exchanged receipt should persist");
        let receipt = BootstrapReceipt::read(&config.receipt_path)
            .expect("exchanged receipt should be valid")
            .expect("exchanged receipt should exist");
        std::fs::write(&config.envelope_path, b"pending confirmation")
            .expect("confirmation envelope should exist");
        let mut profile = crate::config::PersistedCloudRelayProfile::default();
        profile.machine_credential = Some(format!("mcred_{}", "a".repeat(40)));
        let mut confirmation = Some(PendingConfirmation {
            envelope: super::super::state::ManagedBootstrapEnvelope {
                schema_version: 1,
                cloud_api_url: "https://cloud.example.test".to_string(),
                environment_id: receipt.environment_id.clone(),
                token: format!("mkboot_{}", "b".repeat(40)),
                expires_at: "2026-09-23T00:00:00Z".to_string(),
                runtime_release_digest: receipt.runtime_release_digest.clone(),
                managed_repository_root: None,
                provider_rebuild_action_id: None,
            },
            receipt,
            profile,
        });
        let release = VerifiedRelease {
            digest: runtime_release_digest,
            kernel_binary,
        };
        let path_value = format!(
            "{}/.local/bin:/usr/local/bin:/usr/bin:/bin",
            process_home.display()
        );
        let mut environment_names = PATH1_SHARED_HOST_SELECTOR_ENVS.to_vec();
        environment_names.extend([
            "HOME",
            "PATH",
            "CHARIOX_TEST_PATH1_CONFIRM_CAPTURE",
            "CHARIOX_MANAGED_PROVIDER_HOME",
            "CHARIOX_MANAGED_VAULT_PATH",
            MANAGED_PROVIDER_TOPOLOGY_ENV,
        ]);
        let environment = Path1TestEnvironment::capture(&environment_names);
        for name in PATH1_SHARED_HOST_SELECTOR_ENVS {
            std::env::set_var(name, "contaminated-shared-host-selector");
        }
        std::env::set_var("HOME", root.join("stale-home"));
        std::env::set_var("PATH", &path_value);
        std::env::set_var("CHARIOX_TEST_PATH1_CONFIRM_CAPTURE", &capture);
        std::env::set_var("CHARIOX_MANAGED_PROVIDER_HOME", &provider_home);
        std::env::set_var("CHARIOX_MANAGED_VAULT_PATH", "/stale/managed-vault.json");
        std::env::set_var(MANAGED_PROVIDER_TOPOLOGY_ENV, "path1");

        let run = run_kernel_once(
            &config,
            &release,
            &mut confirmation,
            &Path1ConfirmationCloud,
            ManagedProviderTopology::Path1,
        );

        drop(environment);
        let run = run.expect("Path-1 confirmation restart should complete");
        assert!(
            run.status.success(),
            "replacement kernel failed: {}",
            run.status
        );
        assert!(confirmation.is_none());
        for generation in [1, 2] {
            assert_path1_confirmation_capture(
                &std::path::PathBuf::from(format!("{}.{generation}", capture.display())),
                &process_home,
                &chariox_home,
                &provider_home,
                &path_value,
            );
        }
    }

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
