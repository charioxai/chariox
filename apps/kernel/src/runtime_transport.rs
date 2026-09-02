use std::collections::VecDeque;
use std::future::Future;
use std::io::{Read, Write};
use std::net::TcpListener as StdTcpListener;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use futures_util::{Sink, SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex, OwnedSemaphorePermit, Semaphore, TryAcquireError};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tokio_tungstenite::{
    accept_async, accept_hdr_async,
    tungstenite::{
        handshake::server::ErrorResponse,
        http::StatusCode,
        protocol::{frame::coding::CloseCode, CloseFrame, Message},
    },
};

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::runtime::command::{KernelCommand, KernelCommandPriority, KernelCommandSource};
use crate::runtime::event_log::EventLog;
use crate::runtime::projection::TransportHealthStore;
use crate::runtime::router::CommandRouter;
use crate::transport::kernel_protocol::{
    kernel_subscription_scope, map_kernel_error, serialize_frame, KernelEvent, KernelIncomingFrame,
    KernelOutgoingFrame, KernelSubscriptionScope, KernelTransportError,
    WAITING_ROOM_INVENTORY_SENTINEL_ID, WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE,
};

mod local_presence;

pub(crate) mod command_cache;
mod outgoing;
mod subscriptions;

pub(crate) use command_cache::COMMAND_RESULT_CACHE_LIMIT;
use command_cache::{
    request_is_cacheable, CommandFingerprint, CommandReservation, CommandResultCache,
};
use outgoing::{try_send_outgoing_frame, KernelOutgoingSender};
use subscriptions::{
    emit_replay_gap_snapshot, replay_recent_events, run_subscription_loop, ReplaySubscriptionResult,
};
pub(crate) use subscriptions::{watch_subscription_state, WatchResult};

pub(crate) const WATCH_INTERVAL_MS: u64 = 100;
pub(crate) const WAITING_ROOM_ROW_COALESCE_MS: u64 = 500;
const PUMP_INTERVAL_MS: u64 = 500;
const IDLE_PUMP_INTERVAL_MS: u64 = 5_000;
pub(crate) const SESSION_SNAPSHOT_RECONCILIATION_INTERVAL_TICKS: u64 = 300;
const HEARTBEAT_INTERVAL_TICKS: u64 = 50;
const RELAY_DISCOVERY_INTERVAL_TICKS: u64 = 150;
const WAITING_ROOM_INVENTORY_INTERVAL_TICKS: u64 = 100;
const DURABLE_SNAPSHOT_POLL_INTERVAL_MS: u64 = 5_000;
const WEBSOCKET_PING_INTERVAL_MS: u64 = 5_000;
const MAX_KERNEL_LOCAL_AUTH_TOKEN_BYTES: u64 = 8 * 1024;
pub const KERNEL_LOCAL_AUTH_TOKEN_ENV: &str = "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN";
pub const KERNEL_LOCAL_AUTH_TOKEN_FILE_ENV: &str = "CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE";
pub const KERNEL_RUNTIME_THREAD_STACK_SIZE: usize = 16 * 1024 * 1024;
pub(crate) const RECENT_EVENT_LIMIT: usize = 256;
const BACKPRESSURE_CLOSE_REASON: &str = "kernel transport overloaded; reconnecting";
pub const CONNECTION_INBOUND_REQUEST_LIMIT: usize = 32;
const MIN_PROCESS_INBOUND_REQUEST_LIMIT: usize = 32;
const MAX_PROCESS_INBOUND_REQUEST_LIMIT: usize = 256;
const PROCESS_INBOUND_REQUESTS_PER_CPU: usize = 8;
const RESERVED_INTERACTIVE_REQUESTS: usize = 8;
static KERNEL_LOCAL_AUTH_TOKEN: OnceLock<Option<Arc<str>>> = OnceLock::new();

pub fn initialize_kernel_local_auth_from_env() -> Result<(), DaemonError> {
    if KERNEL_LOCAL_AUTH_TOKEN.get().is_some() {
        std::env::remove_var(KERNEL_LOCAL_AUTH_TOKEN_ENV);
        std::env::remove_var(KERNEL_LOCAL_AUTH_TOKEN_FILE_ENV);
        return Ok(());
    }
    let environment_token = match std::env::var(KERNEL_LOCAL_AUTH_TOKEN_ENV) {
        Ok(value) if value.trim().is_empty() => {
            return Err(DaemonError::LocalTransport {
                operation: "configure kernel websocket auth",
                message: format!("{KERNEL_LOCAL_AUTH_TOKEN_ENV} must not be empty"),
            });
        }
        Ok(value) => Some(value.trim().to_string()),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(DaemonError::LocalTransport {
                operation: "configure kernel websocket auth",
                message: format!("{KERNEL_LOCAL_AUTH_TOKEN_ENV} must be valid UTF-8"),
            });
        }
    };
    let token_file = match std::env::var(KERNEL_LOCAL_AUTH_TOKEN_FILE_ENV) {
        Ok(value) if value.trim().is_empty() => {
            return Err(DaemonError::LocalTransport {
                operation: "configure kernel websocket auth",
                message: format!("{KERNEL_LOCAL_AUTH_TOKEN_FILE_ENV} must not be empty"),
            });
        }
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(DaemonError::LocalTransport {
                operation: "configure kernel websocket auth",
                message: format!("{KERNEL_LOCAL_AUTH_TOKEN_FILE_ENV} must be valid UTF-8"),
            });
        }
    };
    std::env::remove_var(KERNEL_LOCAL_AUTH_TOKEN_ENV);
    std::env::remove_var(KERNEL_LOCAL_AUTH_TOKEN_FILE_ENV);
    if environment_token.is_some() && token_file.is_some() {
        return Err(DaemonError::LocalTransport {
            operation: "configure kernel websocket auth",
            message: format!(
                "{KERNEL_LOCAL_AUTH_TOKEN_ENV} and {KERNEL_LOCAL_AUTH_TOKEN_FILE_ENV} cannot both be set"
            ),
        });
    }
    let token = match (environment_token, token_file) {
        (Some(value), None) => Some(Arc::<str>::from(value)),
        (None, Some(path)) => Some(Arc::<str>::from(read_kernel_local_auth_token_file(&path)?)),
        (None, None) => None,
        (Some(_), Some(_)) => unreachable!(),
    };
    if token.is_some() {
        disable_kernel_process_dumpability()?;
    }
    let _ = KERNEL_LOCAL_AUTH_TOKEN.set(token);
    Ok(())
}

fn read_kernel_local_auth_token_file(path: &str) -> Result<String, DaemonError> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read kernel websocket auth file",
            message: error.to_string(),
        })?;
    read_opened_kernel_local_auth_token_file(&mut file, path)
}

fn read_opened_kernel_local_auth_token_file(
    file: &mut std::fs::File,
    path: &str,
) -> Result<String, DaemonError> {
    use std::io::Read;

    let metadata = file
        .metadata()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read kernel websocket auth file",
            message: error.to_string(),
        })?;
    if !metadata.file_type().is_file() {
        return Err(DaemonError::LocalTransport {
            operation: "read kernel websocket auth file",
            message: "auth file must be a regular file".to_string(),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.permissions().mode() & 0o077 != 0
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.len() > MAX_KERNEL_LOCAL_AUTH_TOKEN_BYTES
        {
            return Err(DaemonError::LocalTransport {
                operation: "read kernel websocket auth file",
                message:
                    "auth file must be a bounded, single-link file owned by the kernel user with mode 0600"
                        .to_string(),
            });
        }
    }
    let mut value = String::new();
    let read_result = file.read_to_string(&mut value);
    consume_opened_kernel_local_auth_token_path(path, &metadata)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let remaining_links = file
            .metadata()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "consume kernel websocket auth file",
                message: error.to_string(),
            })?
            .nlink();
        if remaining_links != 0 {
            return Err(DaemonError::LocalTransport {
                operation: "consume kernel websocket auth file",
                message: "auth file was not consumed from its validated descriptor".to_string(),
            });
        }
    }
    read_result.map_err(|error| DaemonError::LocalTransport {
        operation: "read kernel websocket auth file",
        message: error.to_string(),
    })?;
    let token = value.trim();
    if token.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "configure kernel websocket auth",
            message: format!("{KERNEL_LOCAL_AUTH_TOKEN_FILE_ENV} must not contain an empty token"),
        });
    }
    Ok(token.to_string())
}

fn consume_opened_kernel_local_auth_token_path(
    path: &str,
    opened_metadata: &std::fs::Metadata,
) -> Result<(), DaemonError> {
    let path_metadata =
        std::fs::symlink_metadata(path).map_err(|error| DaemonError::LocalTransport {
            operation: "consume kernel websocket auth file",
            message: error.to_string(),
        })?;
    if !path_metadata.file_type().is_file() || path_metadata.file_type().is_symlink() {
        return Err(DaemonError::LocalTransport {
            operation: "consume kernel websocket auth file",
            message: "auth file changed while it was being consumed".to_string(),
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if path_metadata.dev() != opened_metadata.dev()
            || path_metadata.ino() != opened_metadata.ino()
        {
            return Err(DaemonError::LocalTransport {
                operation: "consume kernel websocket auth file",
                message: "auth file changed while it was being consumed".to_string(),
            });
        }
    }
    std::fs::remove_file(path).map_err(|error| DaemonError::LocalTransport {
        operation: "consume kernel websocket auth file",
        message: error.to_string(),
    })
}

fn configured_kernel_local_auth_token() -> Option<Arc<str>> {
    KERNEL_LOCAL_AUTH_TOKEN.get().cloned().flatten()
}

#[cfg(target_os = "linux")]
fn disable_kernel_process_dumpability() -> Result<(), DaemonError> {
    let result = unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0) };
    if result == 0 {
        return Ok(());
    }
    Err(DaemonError::LocalTransport {
        operation: "harden kernel websocket auth",
        message: std::io::Error::last_os_error().to_string(),
    })
}

#[cfg(not(target_os = "linux"))]
fn disable_kernel_process_dumpability() -> Result<(), DaemonError> {
    Ok(())
}

pub(crate) fn process_inbound_request_limit() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(4)
        .saturating_mul(PROCESS_INBOUND_REQUESTS_PER_CPU)
        .clamp(
            MIN_PROCESS_INBOUND_REQUEST_LIMIT,
            MAX_PROCESS_INBOUND_REQUEST_LIMIT,
        )
}

#[derive(Clone)]
struct InboundRequestAdmission {
    total: Arc<Semaphore>,
    non_interactive: Arc<Semaphore>,
}

struct InboundRequestPermit {
    _connection: OwnedSemaphorePermit,
    _total: OwnedSemaphorePermit,
    _non_interactive: Option<OwnedSemaphorePermit>,
}

impl InboundRequestAdmission {
    fn new(process_limit: usize) -> Self {
        Self {
            total: Arc::new(Semaphore::new(process_limit)),
            non_interactive: Arc::new(Semaphore::new(
                process_limit.saturating_sub(RESERVED_INTERACTIVE_REQUESTS),
            )),
        }
    }

    fn try_acquire(
        &self,
        connection: &Arc<Semaphore>,
        priority: &KernelCommandPriority,
    ) -> Result<InboundRequestPermit, TryAcquireError> {
        let connection = Arc::clone(connection).try_acquire_owned()?;
        let non_interactive = if *priority == KernelCommandPriority::Interactive {
            None
        } else {
            Some(Arc::clone(&self.non_interactive).try_acquire_owned()?)
        };
        let total = Arc::clone(&self.total).try_acquire_owned()?;
        Ok(InboundRequestPermit {
            _connection: connection,
            _total: total,
            _non_interactive: non_interactive,
        })
    }
}

#[derive(Debug, Clone)]
struct KernelSubscription {
    session_id: String,
    attachment_id: String,
    subscription_scope: KernelSubscriptionScope,
}

#[derive(Debug)]
struct KernelTransportRuntime {
    event_log: EventLog<KernelEvent>,
    command_result_cache: CommandResultCache,
    transport_health: TransportHealthStore,
}

impl Default for KernelTransportRuntime {
    fn default() -> Self {
        Self::new(TransportHealthStore::default())
    }
}

impl KernelTransportRuntime {
    fn new(transport_health: TransportHealthStore) -> Self {
        Self {
            event_log: EventLog::new(RECENT_EVENT_LIMIT),
            command_result_cache: CommandResultCache::default(),
            transport_health,
        }
    }

    fn new_with_persistent_event_ids(
        transport_health: TransportHealthStore,
        event_counter_path: impl Into<std::path::PathBuf>,
    ) -> Result<Self, DaemonError> {
        let event_counter_path = event_counter_path.into();
        let command_result_cache_path = event_counter_path.with_file_name("command-results.jsonl");
        Ok(Self {
            event_log: EventLog::new_with_persistent_event_store(
                RECENT_EVENT_LIMIT,
                event_counter_path.clone(),
                event_counter_path.with_file_name("events.jsonl"),
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "reserve kernel event ids",
                message: error.to_string(),
            })?,
            command_result_cache: CommandResultCache::new_with_persistent_path(
                command_result_cache_path,
            )
            .map_err(|error| DaemonError::LocalTransport {
                operation: "load kernel command result cache",
                message: error.to_string(),
            })?,
            transport_health,
        })
    }
}

#[derive(Debug)]
struct ConnectionState {
    subscription: Option<KernelSubscription>,
    watch_task: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone)]
struct ConnectionCloseCommand {
    reason: String,
}

struct TransportConnectionGuard {
    transport_health: TransportHealthStore,
}

impl Drop for TransportConnectionGuard {
    fn drop(&mut self) {
        self.transport_health.record_connection_closed();
    }
}

pub async fn run_kernel_websocket_server<F>(
    app: Arc<Mutex<DaemonApp>>,
    shutdown: F,
) -> Result<(), DaemonError>
where
    F: Future<Output = ()>,
{
    initialize_kernel_local_auth_from_env()?;
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        Arc::clone(&app),
        crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
    ));
    let (bind_host, bind_port) = router.kernel_websocket_bind_address();
    let bind_started = Instant::now();
    let listener = TcpListener::bind((bind_host.as_str(), bind_port))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "bind kernel websocket",
            message: error.to_string(),
        })?;
    crate::logging::info_with_fields(
        "daemon.startup",
        "kernel websocket listener bound",
        serde_json::json!({
            "bind_ms": bind_started.elapsed().as_millis(),
            "bind_host": bind_host,
            "bind_port": bind_port,
        }),
    );
    let _local_presence = local_presence::LocalKernelPresenceLease::start(&router, &listener).await;
    run_kernel_websocket_server_with_bound_listener(
        router,
        listener,
        configured_kernel_local_auth_token(),
        shutdown,
    )
    .await
}

pub(crate) async fn run_kernel_websocket_server_with_router<F>(
    router: Arc<CommandRouter>,
    shutdown: F,
) -> Result<(), DaemonError>
where
    F: Future<Output = ()>,
{
    initialize_kernel_local_auth_from_env()?;
    let (bind_host, bind_port) = router.kernel_websocket_bind_address();
    let bind_started = Instant::now();
    let listener = TcpListener::bind((bind_host.as_str(), bind_port))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "bind kernel websocket",
            message: error.to_string(),
        })?;
    crate::logging::info_with_fields(
        "daemon.startup",
        "kernel websocket listener bound",
        serde_json::json!({
            "bind_ms": bind_started.elapsed().as_millis(),
            "bind_host": bind_host,
            "bind_port": bind_port,
        }),
    );
    let _local_presence = local_presence::LocalKernelPresenceLease::start(&router, &listener).await;
    run_kernel_websocket_server_with_bound_listener(
        router,
        listener,
        configured_kernel_local_auth_token(),
        shutdown,
    )
    .await
}

pub async fn run_kernel_websocket_server_on_listener<F>(
    app: Arc<Mutex<DaemonApp>>,
    listener: StdTcpListener,
    shutdown: F,
) -> Result<(), DaemonError>
where
    F: Future<Output = ()>,
{
    initialize_kernel_local_auth_from_env()?;
    run_kernel_websocket_server_on_listener_with_auth(
        app,
        listener,
        configured_kernel_local_auth_token(),
        shutdown,
    )
    .await
}

pub(crate) async fn run_kernel_websocket_server_with_router_on_listener<F>(
    router: Arc<CommandRouter>,
    listener: StdTcpListener,
    shutdown: F,
) -> Result<(), DaemonError>
where
    F: Future<Output = ()>,
{
    initialize_kernel_local_auth_from_env()?;
    listener
        .set_nonblocking(true)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "configure kernel websocket listener",
            message: error.to_string(),
        })?;
    let listener =
        TcpListener::from_std(listener).map_err(|error| DaemonError::LocalTransport {
            operation: "adopt kernel websocket listener",
            message: error.to_string(),
        })?;
    let _local_presence = local_presence::LocalKernelPresenceLease::start(&router, &listener).await;
    run_kernel_websocket_server_with_bound_listener(
        router,
        listener,
        configured_kernel_local_auth_token(),
        shutdown,
    )
    .await
}

async fn run_kernel_websocket_server_on_listener_with_auth<F>(
    app: Arc<Mutex<DaemonApp>>,
    listener: StdTcpListener,
    local_auth_token: Option<Arc<str>>,
    shutdown: F,
) -> Result<(), DaemonError>
where
    F: Future<Output = ()>,
{
    let listener = adopt_std_listener(listener, "kernel websocket")?;
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        app,
        crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
    ));
    run_kernel_websocket_server_with_bound_listener(router, listener, local_auth_token, shutdown)
        .await
}

#[cfg(test)]
async fn run_kernel_websocket_server_on_listeners_with_auth<F>(
    app: Arc<Mutex<DaemonApp>>,
    listener: StdTcpListener,
    mcp_listener: StdTcpListener,
    local_auth_token: Option<Arc<str>>,
    shutdown: F,
) -> Result<(), DaemonError>
where
    F: Future<Output = ()>,
{
    let listener = adopt_std_listener(listener, "kernel websocket")?;
    let mcp_listener = adopt_std_listener(mcp_listener, "runtime mcp")?;
    let router = Arc::new(CommandRouter::with_interactive_capacity_from_app(
        app,
        crate::runtime::router::INTERACTIVE_COMMAND_QUEUE_LIMIT,
    ));
    run_kernel_websocket_server_with_bound_listeners(
        router,
        listener,
        mcp_listener,
        local_auth_token,
        shutdown,
    )
    .await
}

fn adopt_std_listener(
    listener: StdTcpListener,
    listener_name: &'static str,
) -> Result<TcpListener, DaemonError> {
    listener
        .set_nonblocking(true)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "configure pre-bound listener",
            message: format!("{listener_name}: {error}"),
        })?;
    TcpListener::from_std(listener).map_err(|error| DaemonError::LocalTransport {
        operation: "adopt pre-bound listener",
        message: format!("{listener_name}: {error}"),
    })
}

async fn run_kernel_websocket_server_with_bound_listener<F>(
    router: Arc<CommandRouter>,
    listener: TcpListener,
    local_auth_token: Option<Arc<str>>,
    shutdown: F,
) -> Result<(), DaemonError>
where
    F: Future<Output = ()>,
{
    let mcp_listener = crate::transport::mcp_server::bind_mcp_http_server(&router).await?;
    run_kernel_websocket_server_with_bound_listeners(
        router,
        listener,
        mcp_listener,
        local_auth_token,
        shutdown,
    )
    .await
}

async fn run_kernel_websocket_server_with_bound_listeners<F>(
    router: Arc<CommandRouter>,
    listener: TcpListener,
    mcp_listener: TcpListener,
    local_auth_token: Option<Arc<str>>,
    shutdown: F,
) -> Result<(), DaemonError>
where
    F: Future<Output = ()>,
{
    let transport_health = router.transport_health_store();
    let durable_snapshot_scheduler = router.durable_snapshot_scheduler();
    let event_counter_path = router.kernel_event_counter_path();
    let local_addr = listener.local_addr().ok().map(|addr| addr.to_string());
    let runtime = Arc::new(KernelTransportRuntime::new_with_persistent_event_ids(
        transport_health.clone(),
        event_counter_path,
    )?);
    let process_inbound_request_limit = process_inbound_request_limit();
    let inbound_request_admission = InboundRequestAdmission::new(process_inbound_request_limit);
    crate::logging::info_with_fields(
        "daemon.startup",
        "kernel ready for local command",
        serde_json::json!({
            "phase": "runtime_ready",
            "kernel_websocket_addr": local_addr,
            "kernel_local_auth_required": local_auth_token.is_some(),
            "recent_event_limit": RECENT_EVENT_LIMIT,
            "process_inbound_request_limit": process_inbound_request_limit,
            "connection_inbound_request_limit": CONNECTION_INBOUND_REQUEST_LIMIT,
            "reserved_interactive_requests": RESERVED_INTERACTIVE_REQUESTS,
        }),
    );

    tokio::pin!(shutdown);

    let pump_router = Arc::clone(&router);
    let pump_task = tokio::spawn(async move {
        loop {
            pump_router.pump_transport_runtime().await;
            let change_sequence = pump_router.transport_runtime_pump_change_sequence();
            let pty_output_sequence = pump_router.pty_output_change_sequence();
            let provider_actor_completion_sequence =
                pump_router.provider_run_actor_completion_sequence();
            let delay_ms = pump_router.transport_runtime_pump_interval_ms(
                PUMP_INTERVAL_MS,
                IDLE_PUMP_INTERVAL_MS,
                crate::session::unix_epoch_ms(),
            );
            tokio::select! {
                _ = sleep(Duration::from_millis(delay_ms)) => {}
                _ = pump_router.wait_for_transport_runtime_pump_change_after(change_sequence) => {}
                _ = pump_router.wait_for_pty_output_change_after(pty_output_sequence) => {}
                _ = pump_router.wait_for_provider_run_actor_completion_after(provider_actor_completion_sequence) => {}
            }
        }
    });
    let mut durable_snapshot_task = durable_snapshot_scheduler.map(|scheduler| {
        tokio::spawn(scheduler.run(Duration::from_millis(DURABLE_SNAPSHOT_POLL_INTERVAL_MS)))
    });

    // Bind the provider-facing MCP endpoint before allowing durable restart
    // recovery to launch any provider. Recovery can dispatch immediately after
    // a kernel restart; starting it before this listener is bound creates a
    // race where required MCP initialization fails and the provider run is
    // stranded before its first turn.
    let mcp_router = Arc::clone(&router);
    let mcp_task = tokio::spawn(async move {
        let _ =
            crate::transport::mcp_server::run_mcp_http_server_on_listener(mcp_router, mcp_listener)
                .await;
    });
    let _restart_recovery_task = router.runtime_state().spawn_durable_restart_recovery();

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                drop(_restart_recovery_task);
                pump_task.abort();
                if let Some(task) = durable_snapshot_task.take() {
                    task.abort();
                }
                mcp_task.abort();
                let _ = router.shutdown_cleanup().await;
                return Ok(());
            },
            accept_result = listener.accept() => {
                let (stream, _) = accept_result.map_err(|error| DaemonError::LocalTransport {
                    operation: "accept kernel websocket",
                    message: error.to_string(),
                })?;
                let runtime = Arc::clone(&runtime);
                let router = Arc::clone(&router);
                let inbound_request_admission = inbound_request_admission.clone();
                let local_auth_token = local_auth_token.clone();
                tokio::spawn(async move {
                    let _ = handle_kernel_connection(
                        runtime,
                        router,
                        inbound_request_admission,
                        local_auth_token,
                        stream,
                    )
                    .await;
                });
            }
        }
    }
}

#[cfg(test)]
mod tests;

#[derive(Debug)]
struct EventWriteCoalescer<T> {
    delay_ms: u64,
    frames: VecDeque<T>,
    ready_at: Option<tokio::time::Instant>,
}

impl<T> EventWriteCoalescer<T> {
    fn new(delay_ms: u64) -> Self {
        Self {
            delay_ms,
            frames: VecDeque::new(),
            ready_at: None,
        }
    }

    fn ready_at(&self) -> Option<tokio::time::Instant> {
        self.ready_at
    }

    fn push_event(&mut self, frame: T, now: tokio::time::Instant) -> Option<T> {
        if self.delay_ms == 0 {
            return Some(frame);
        }
        self.frames.push_back(frame);
        if self.ready_at.is_none() {
            self.ready_at = Some(now + Duration::from_millis(self.delay_ms));
        }
        None
    }

    fn drain_ready(&mut self) -> Vec<T> {
        self.ready_at = None;
        self.frames.drain(..).collect()
    }
}

fn kernel_local_authorization_matches(value: Option<&str>, expected_token: &str) -> bool {
    let Some(token) = value.and_then(|value| value.strip_prefix("Bearer ")) else {
        return false;
    };
    let expected = expected_token.as_bytes();
    let supplied = token.as_bytes();
    let mut difference = expected.len() ^ supplied.len();
    for (index, byte) in expected.iter().enumerate() {
        difference |= usize::from(*byte ^ supplied.get(index).copied().unwrap_or_default());
    }
    difference == 0
}

async fn handle_kernel_connection(
    runtime: Arc<KernelTransportRuntime>,
    router: Arc<CommandRouter>,
    inbound_request_admission: InboundRequestAdmission,
    local_auth_token: Option<Arc<str>>,
    stream: tokio::net::TcpStream,
) -> Result<(), DaemonError> {
    let socket = if let Some(expected_token) = local_auth_token {
        accept_hdr_async(
            stream,
            move |request: &tokio_tungstenite::tungstenite::handshake::server::Request,
                  response| {
                if kernel_local_authorization_matches(
                    request
                        .headers()
                        .get("authorization")
                        .and_then(|value| value.to_str().ok()),
                    &expected_token,
                ) {
                    return Ok(response);
                }
                let mut error = ErrorResponse::new(Some("Unauthorized".to_string()));
                *error.status_mut() = StatusCode::UNAUTHORIZED;
                Err(error)
            },
        )
        .await
    } else {
        accept_async(stream).await
    }
    .map_err(|error| DaemonError::LocalTransport {
        operation: "accept kernel websocket handshake",
        message: error.to_string(),
    })?;
    runtime.transport_health.record_connection_opened();
    let _connection_guard = TransportConnectionGuard {
        transport_health: runtime.transport_health.clone(),
    };
    let (queue_capacity, write_delay_ms) = router.kernel_websocket_connection_config();

    let (mut writer, mut reader) = socket.split();
    let (priority_tx, mut priority_rx) = mpsc::channel::<KernelOutgoingFrame>(queue_capacity);
    let (event_tx, mut event_rx) = mpsc::channel::<KernelOutgoingFrame>(queue_capacity);
    let outgoing_tx = KernelOutgoingSender::new(priority_tx, event_tx);
    let (close_tx, mut close_rx) = mpsc::unbounded_channel::<ConnectionCloseCommand>();
    let (pong_tx, mut pong_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let close_requested = Arc::new(AtomicBool::new(false));
    let connection_inbound_request_permits =
        Arc::new(Semaphore::new(CONNECTION_INBOUND_REQUEST_LIMIT));
    let connection_state = Arc::new(Mutex::new(ConnectionState {
        subscription: None,
        watch_task: None,
    }));

    let writer_task = tokio::spawn(async move {
        let mut transport_ping =
            tokio::time::interval(Duration::from_millis(WEBSOCKET_PING_INTERVAL_MS));
        transport_ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut event_write_coalescer = EventWriteCoalescer::new(write_delay_ms);
        'writer_loop: loop {
            if let Some(ready_at) = event_write_coalescer.ready_at() {
                tokio::select! {
                    biased;
                    Some(command) = close_rx.recv() => {
                        let _ = writer.send(Message::Close(Some(CloseFrame {
                            code: CloseCode::Policy,
                            reason: command.reason.into(),
                        }))).await;
                        break;
                    }
                    Some(payload) = pong_rx.recv() => {
                        if writer.send(Message::Pong(payload.into())).await.is_err() {
                            break;
                        }
                    }
                    _ = transport_ping.tick() => {
                        if writer.send(Message::Ping(Vec::new().into())).await.is_err() {
                            break;
                        }
                    }
                    Some(frame) = priority_rx.recv() => {
                        if !send_kernel_frame(&mut writer, frame).await {
                            break;
                        }
                    }
                    Some(frame) = event_rx.recv() => {
                        if let Some(frame) = event_write_coalescer.push_event(frame, tokio::time::Instant::now()) {
                            if !send_kernel_frame(&mut writer, frame).await {
                                break;
                            }
                        }
                    }
                    _ = tokio::time::sleep_until(ready_at) => {
                        for frame in event_write_coalescer.drain_ready() {
                            if !send_kernel_frame(&mut writer, frame).await {
                                break 'writer_loop;
                            }
                        }
                    }
                    else => break,
                }
                continue;
            }

            tokio::select! {
                biased;
                Some(command) = close_rx.recv() => {
                    let _ = writer.send(Message::Close(Some(CloseFrame {
                        code: CloseCode::Policy,
                        reason: command.reason.into(),
                    }))).await;
                    break;
                }
                Some(payload) = pong_rx.recv() => {
                    if writer.send(Message::Pong(payload.into())).await.is_err() {
                        break;
                    }
                }
                _ = transport_ping.tick() => {
                    if writer.send(Message::Ping(Vec::new().into())).await.is_err() {
                        break;
                    }
                }
                Some(frame) = priority_rx.recv() => {
                    if !send_kernel_frame(&mut writer, frame).await {
                        break;
                    }
                }
                Some(frame) = event_rx.recv() => {
                    let Some(frame) = event_write_coalescer.push_event(frame, tokio::time::Instant::now()) else {
                        continue;
                    };
                    if !send_kernel_frame(&mut writer, frame).await {
                        break;
                    }
                }
                else => break,
            }
        }
    });

    let mut read_error = None;
    while let Some(message_result) = reader.next().await {
        let message = match message_result {
            Ok(message) => message,
            Err(error) => {
                read_error = Some(DaemonError::LocalTransport {
                    operation: "read kernel websocket frame",
                    message: error.to_string(),
                });
                break;
            }
        };

        match message {
            Message::Text(payload) => {
                handle_incoming_payload(
                    &runtime,
                    &router,
                    &connection_state,
                    &inbound_request_admission,
                    &connection_inbound_request_permits,
                    &outgoing_tx,
                    &close_tx,
                    &close_requested,
                    payload.as_bytes(),
                )
                .await;
            }
            Message::Binary(payload) => {
                handle_incoming_payload(
                    &runtime,
                    &router,
                    &connection_state,
                    &inbound_request_admission,
                    &connection_inbound_request_permits,
                    &outgoing_tx,
                    &close_tx,
                    &close_requested,
                    &payload,
                )
                .await;
            }
            Message::Ping(payload) => {
                let _ = pong_tx.send(payload.to_vec());
            }
            Message::Close(_) => break,
            Message::Pong(_) => {
                record_connection_subscription_heartbeat(&router, &connection_state).await;
            }
            Message::Frame(_) => {}
        }
    }

    {
        let mut state = connection_state.lock().await;
        if let Some(task) = state.watch_task.take() {
            task.abort();
        }
        if let Some(subscription) = state.subscription.take() {
            runtime.transport_health.record_subscription_closed();
            drop(state);
            detach_connection_subscription(&router, subscription).await;
        }
    }
    writer_task.abort();

    if let Some(error) = read_error {
        return Err(error);
    }

    Ok(())
}

async fn detach_connection_subscription(
    router: &Arc<CommandRouter>,
    subscription: KernelSubscription,
) {
    if subscription.subscription_scope == KernelSubscriptionScope::WaitingRoomInventory {
        return;
    }
    match router
        .detach_terminal_attachment(&subscription.attachment_id)
        .await
    {
        Ok(attachment) => {
            crate::logging::info_with_fields(
                "daemon.runtime_transport",
                "detached terminal attachment after websocket close",
                serde_json::json!({
                    "session_id": attachment.session_id(),
                    "attachment_id": attachment.id(),
                }),
            );
        }
        Err(DaemonError::AttachmentNotFound { .. }) => {}
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.runtime_transport",
                "failed detaching terminal attachment after websocket close",
                serde_json::json!({
                    "session_id": subscription.session_id,
                    "attachment_id": subscription.attachment_id,
                    "error": error.to_string(),
                }),
            );
        }
    }
}

async fn record_connection_subscription_heartbeat(
    router: &Arc<CommandRouter>,
    connection_state: &Arc<Mutex<ConnectionState>>,
) {
    let subscription = {
        let state = connection_state.lock().await;
        state.subscription.clone()
    };
    let Some(subscription) = subscription else {
        return;
    };
    if subscription.subscription_scope == KernelSubscriptionScope::WaitingRoomInventory {
        return;
    }
    if let Err(error) = router
        .record_terminal_attachment_heartbeat(
            &subscription.session_id,
            &subscription.attachment_id,
            crate::session::unix_epoch_ms(),
        )
        .await
    {
        crate::logging::warn_with_fields(
            "daemon.runtime_transport",
            "failed recording terminal attachment heartbeat",
            serde_json::json!({
                "session_id": subscription.session_id,
                "attachment_id": subscription.attachment_id,
                "error": error.to_string(),
            }),
        );
    }
}

async fn send_kernel_frame<S>(writer: &mut S, frame: KernelOutgoingFrame) -> bool
where
    S: Sink<Message> + Unpin,
{
    let payload = match serialize_frame(&frame) {
        Ok(payload) => payload,
        Err(_) => return false,
    };
    writer.send(Message::Text(payload.into())).await.is_ok()
}

async fn handle_incoming_payload(
    runtime: &Arc<KernelTransportRuntime>,
    router: &Arc<CommandRouter>,
    connection_state: &Arc<Mutex<ConnectionState>>,
    inbound_request_admission: &InboundRequestAdmission,
    connection_inbound_request_permits: &Arc<Semaphore>,
    outgoing_tx: &KernelOutgoingSender,
    close_tx: &mpsc::UnboundedSender<ConnectionCloseCommand>,
    close_requested: &Arc<AtomicBool>,
    payload: &[u8],
) {
    let frame = match serde_json::from_slice::<KernelIncomingFrame>(payload) {
        Ok(frame) => frame,
        Err(error) => {
            let _ = try_send_outgoing_frame(
                outgoing_tx,
                close_tx,
                close_requested,
                &runtime.transport_health,
                KernelOutgoingFrame::Response {
                    request_id: "unknown".to_string(),
                    response: Box::new(None),
                    error: Some(KernelTransportError {
                        code: "invalid_frame".to_string(),
                        message: format!("invalid kernel transport payload: {error}"),
                        retryable: false,
                    }),
                },
                None,
                None,
            );
            return;
        }
    };

    match frame {
        KernelIncomingFrame::Request {
            request_id,
            command_id,
            causation_id,
            correlation_id,
            request,
        } => {
            runtime.transport_health.record_incoming_request();
            let caller = router
                .local_command_caller(KernelCommandSource::LocalCli)
                .await;
            let command = KernelCommand::from_local_request_with_caller(
                command_id.unwrap_or_else(|| request_id.clone()),
                KernelCommandSource::LocalCli,
                caller,
                correlation_id.clone(),
                causation_id.clone(),
                &request,
            );
            let fingerprint = request_is_cacheable(&request)
                .then(|| CommandFingerprint::from_command_and_request(&command, &request));
            if let Some(fingerprint) = fingerprint.as_ref() {
                match runtime
                    .command_result_cache
                    .reserve(&command.command_id, fingerprint)
                    .await
                {
                    CommandReservation::Wait(wait_rx) => {
                        let outgoing_tx = outgoing_tx.clone();
                        let close_tx = close_tx.clone();
                        let close_requested = Arc::clone(close_requested);
                        let transport_health = runtime.transport_health.clone();
                        let session_id = command.session_id.clone();
                        let attachment_id = command.attachment_id.clone();
                        tokio::spawn(async move {
                            let Ok(cached) = wait_rx.await else {
                                let _ = try_send_outgoing_frame(
                                    &outgoing_tx,
                                    &close_tx,
                                    &close_requested,
                                    &transport_health,
                                    KernelOutgoingFrame::Response {
                                        request_id,
                                        response: Box::new(None),
                                        error: Some(KernelTransportError {
                                            code: "duplicate_command_unavailable".to_string(),
                                            message:
                                                "original duplicate command result was unavailable"
                                                    .to_string(),
                                            retryable: true,
                                        }),
                                    },
                                    session_id.as_deref(),
                                    attachment_id.as_deref(),
                                );
                                return;
                            };
                            let _ = try_send_outgoing_frame(
                                &outgoing_tx,
                                &close_tx,
                                &close_requested,
                                &transport_health,
                                KernelOutgoingFrame::Response {
                                    request_id,
                                    response: cached.response,
                                    error: cached.error,
                                },
                                session_id.as_deref(),
                                attachment_id.as_deref(),
                            );
                        });
                        return;
                    }
                    CommandReservation::Conflict => {
                        runtime.transport_health.record_duplicate_command_conflict();
                        let _ = try_send_outgoing_frame(
                            outgoing_tx,
                            close_tx,
                            close_requested,
                            &runtime.transport_health,
                            KernelOutgoingFrame::Response {
                                request_id,
                                response: Box::new(None),
                                error: Some(KernelTransportError {
                                    code: "duplicate_command_conflict".to_string(),
                                    message: format!(
                                        "command_id `{}` was already used for a different request",
                                        command.command_id
                                    ),
                                    retryable: false,
                                }),
                            },
                            command.session_id.as_deref(),
                            command.attachment_id.as_deref(),
                        );
                        return;
                    }
                    CommandReservation::Dispatch => {}
                }
            }
            let permit = match inbound_request_admission
                .try_acquire(connection_inbound_request_permits, &command.priority)
            {
                Ok(permit) => permit,
                Err(error) => {
                    if fingerprint.is_some() {
                        runtime
                            .command_result_cache
                            .forget_pending(&command.command_id)
                            .await;
                    }
                    runtime.transport_health.record_inbound_overload_rejection();
                    let _ = try_send_outgoing_frame(
                        outgoing_tx,
                        close_tx,
                        close_requested,
                        &runtime.transport_health,
                        KernelOutgoingFrame::Response {
                            request_id,
                            response: Box::new(None),
                            error: Some(KernelTransportError {
                                code: "kernel_request_overloaded".to_string(),
                                message: format!(
                                    "kernel request admission queue overloaded: {error}"
                                ),
                                retryable: true,
                            }),
                        },
                        command.session_id.as_deref(),
                        command.attachment_id.as_deref(),
                    );
                    return;
                }
            };
            if !crate::runtime::command_latency::is_quiet_success_command_type(
                &command.command_type,
            ) {
                crate::logging::info_with_fields(
                    "daemon.runtime_transport",
                    "kernel command accepted",
                    serde_json::json!({
                        "request_id": request_id,
                        "command_id": command.command_id,
                        "command_type": command.command_type,
                        "correlation_id": command.correlation_id,
                        "priority": format!("{:?}", command.priority),
                        "session_id": command.session_id,
                        "attachment_id": command.attachment_id,
                        "agent_id": command.agent_id,
                    }),
                );
            }
            let runtime = Arc::clone(runtime);
            let router = Arc::clone(router);
            let outgoing_tx = outgoing_tx.clone();
            let close_tx = close_tx.clone();
            let close_requested = Arc::clone(close_requested);
            tokio::spawn(async move {
                let _permit = permit;
                let command_id = command.command_id.clone();
                let session_id = command.session_id.clone();
                let attachment_id = command.attachment_id.clone();
                let response = router.dispatch(command, request).await;
                let outgoing = match response {
                    Ok(response) => KernelOutgoingFrame::Response {
                        request_id,
                        response: Box::new(Some(
                            serde_json::to_value(response).unwrap_or(Value::Null),
                        )),
                        error: None,
                    },
                    Err(error) => KernelOutgoingFrame::Response {
                        request_id,
                        response: Box::new(None),
                        error: Some(map_kernel_error(&error)),
                    },
                };
                if let Some(fingerprint) = fingerprint {
                    runtime
                        .command_result_cache
                        .complete(command_id, fingerprint, &outgoing)
                        .await;
                }
                let _ = try_send_outgoing_frame(
                    &outgoing_tx,
                    &close_tx,
                    &close_requested,
                    &runtime.transport_health,
                    outgoing,
                    session_id.as_deref(),
                    attachment_id.as_deref(),
                );
            });
        }
        KernelIncomingFrame::Subscribe {
            request_id,
            session_id,
            attachment_id,
            subscription_scope,
            resume_from_event_id,
        } => {
            let scope = kernel_subscription_scope(subscription_scope.as_deref());
            crate::logging::info_with_fields(
                "daemon.runtime_transport",
                "kernel websocket subscribed",
                serde_json::json!({
                    "session_id": session_id,
                    "attachment_id": attachment_id,
                    "subscription_scope": subscription_scope,
                    "resume_from_event_id": resume_from_event_id,
                }),
            );
            if scope != KernelSubscriptionScope::WaitingRoomInventory
                && (session_id == WAITING_ROOM_INVENTORY_SENTINEL_ID
                    || attachment_id == WAITING_ROOM_INVENTORY_SENTINEL_ID)
            {
                crate::logging::warn_with_fields(
                    "daemon.runtime_transport",
                    "waiting-room inventory sentinel arrived without subscription scope",
                    serde_json::json!({
                        "session_id": session_id,
                        "attachment_id": attachment_id,
                        "subscription_scope": subscription_scope,
                        "diagnosis": "client likely dropped subscription_scope=waiting_room_inventory",
                    }),
                );
            }
            let replay_gap = if scope == KernelSubscriptionScope::WaitingRoomInventory {
                None
            } else {
                let replay_result = replay_recent_events(
                    runtime,
                    outgoing_tx,
                    close_tx,
                    close_requested,
                    &session_id,
                    &attachment_id,
                    resume_from_event_id,
                )
                .await;
                match replay_result {
                    ReplaySubscriptionResult::Gap(gap) => {
                        emit_replay_gap_snapshot(
                            router,
                            runtime,
                            outgoing_tx,
                            close_tx,
                            close_requested,
                            &session_id,
                            &attachment_id,
                        )
                        .await;
                        Some(gap)
                    }
                    ReplaySubscriptionResult::Overflow => return,
                    ReplaySubscriptionResult::Complete | ReplaySubscriptionResult::NoCursor => None,
                }
            };
            if !try_send_outgoing_frame(
                outgoing_tx,
                close_tx,
                close_requested,
                &runtime.transport_health,
                KernelOutgoingFrame::Response {
                    request_id,
                    response: Box::new(Some(serde_json::json!({
                        "ok": true,
                        "resumed_from_event_id": resume_from_event_id,
                        "replay_gap": replay_gap.as_ref().map(|gap| serde_json::json!({
                            "requested_from_event_id": gap.requested_from_event_id,
                            "first_retained_event_id": gap.first_retained_event_id,
                            "latest_event_id": gap.latest_event_id,
                        })),
                    }))),
                    error: None,
                },
                if subscription_scope.as_deref() == Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE)
                {
                    None
                } else {
                    Some(&session_id)
                },
                if subscription_scope.as_deref() == Some(WAITING_ROOM_INVENTORY_SUBSCRIPTION_SCOPE)
                {
                    None
                } else {
                    Some(&attachment_id)
                },
            ) {
                return;
            }
            {
                let mut state = connection_state.lock().await;
                if let Some(task) = state.watch_task.take() {
                    task.abort();
                }
                if state.subscription.is_none() {
                    runtime.transport_health.record_subscription_opened();
                }
                state.subscription = Some(KernelSubscription {
                    session_id: session_id.clone(),
                    attachment_id: attachment_id.clone(),
                    subscription_scope: scope.clone(),
                });
                state.watch_task = Some(tokio::spawn(run_subscription_loop(
                    Arc::clone(router),
                    Arc::clone(runtime),
                    outgoing_tx.clone(),
                    close_tx.clone(),
                    Arc::clone(close_requested),
                    KernelSubscription {
                        session_id: session_id.clone(),
                        attachment_id: attachment_id.clone(),
                        subscription_scope: scope,
                    },
                )));
            }
            record_connection_subscription_heartbeat(router, connection_state).await;
        }
        KernelIncomingFrame::Unsubscribe { request_id } => {
            crate::logging::info_with_fields(
                "daemon.runtime_transport",
                "kernel websocket unsubscribed",
                serde_json::json!({}),
            );
            {
                let mut state = connection_state.lock().await;
                if state.subscription.take().is_some() {
                    runtime.transport_health.record_subscription_closed();
                }
                if let Some(task) = state.watch_task.take() {
                    task.abort();
                }
            }
            let _ = try_send_outgoing_frame(
                outgoing_tx,
                close_tx,
                close_requested,
                &runtime.transport_health,
                KernelOutgoingFrame::Response {
                    request_id,
                    response: Box::new(Some(serde_json::json!({ "ok": true }))),
                    error: None,
                },
                None,
                None,
            );
        }
    }
}
pub struct KernelBootListener {
    listener: Option<StdTcpListener>,
    shutdown: Option<std::sync::mpsc::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl KernelBootListener {
    pub fn bind(host: &str, port: u16) -> Result<Self, DaemonError> {
        let listener =
            StdTcpListener::bind((host, port)).map_err(|error| DaemonError::LocalTransport {
                operation: "bind booting kernel websocket",
                message: error.to_string(),
            })?;
        let boot_listener = listener
            .try_clone()
            .map_err(|error| DaemonError::LocalTransport {
                operation: "clone booting kernel websocket listener",
                message: error.to_string(),
            })?;
        boot_listener
            .set_nonblocking(true)
            .map_err(|error| DaemonError::LocalTransport {
                operation: "configure booting kernel websocket listener",
                message: error.to_string(),
            })?;
        let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("chariox-kernel-boot-listener".to_string())
            .spawn(move || {
                let body = br#"{"phase":"booting","runtime_ready":false,"event_ready":false}"#;
                loop {
                    if shutdown_rx.try_recv().is_ok() {
                        break;
                    }
                    match boot_listener.accept() {
                        Ok((mut stream, _)) => {
                            consume_boot_request(&mut stream);
                            let response = format!(
                                "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nRetry-After: 1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            );
                            let _ = stream.write_all(response.as_bytes());
                            let _ = stream.write_all(body);
                            let _ = stream.flush();
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|error| DaemonError::LocalTransport {
                operation: "start booting kernel websocket listener",
                message: error.to_string(),
            })?;
        Ok(Self {
            listener: Some(listener),
            shutdown: Some(shutdown_tx),
            worker: Some(worker),
        })
    }

    pub fn into_listener(mut self) -> Result<StdTcpListener, DaemonError> {
        self.stop_worker();
        self.listener
            .take()
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "adopt booting kernel websocket listener",
                message: "boot listener was already consumed".to_string(),
            })
    }

    pub fn local_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        self.listener
            .as_ref()
            .ok_or_else(|| std::io::Error::other("boot listener was already consumed"))?
            .local_addr()
    }

    fn stop_worker(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn consume_boot_request(stream: &mut std::net::TcpStream) {
    const MAX_BOOT_REQUEST_HEADER_BYTES: usize = 8 * 1024;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let mut request = [0_u8; MAX_BOOT_REQUEST_HEADER_BYTES];
    let mut consumed = 0;
    while consumed < request.len() {
        match stream.read(&mut request[consumed..]) {
            Ok(0) => break,
            Ok(read) => {
                consumed += read;
                if request[..consumed]
                    .windows(4)
                    .any(|window| window == b"\r\n\r\n")
                {
                    break;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(_) => break,
        }
    }
}

impl Drop for KernelBootListener {
    fn drop(&mut self) {
        self.stop_worker();
    }
}

#[cfg(test)]
mod boot_listener_tests {
    use super::KernelBootListener;
    use std::io::{Read, Write};

    #[test]
    fn boot_listener_reports_booting_then_hands_off_the_same_socket() {
        let boot = KernelBootListener::bind("127.0.0.1", 0).expect("boot listener should bind");
        let address = boot.local_addr().expect("boot address should resolve");
        let mut client = std::net::TcpStream::connect(address).expect("boot client should connect");
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .expect("read timeout should configure");
        client.write_all(b"GET /kernel HTTP/1.1\r\n\r\n").unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 503"));
        assert!(response.contains(r#""phase":"booting""#));

        let listener = boot.into_listener().expect("listener should hand off");
        let second = std::thread::spawn(move || {
            std::net::TcpStream::connect(address).expect("runtime client should connect")
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            match listener.accept() {
                Ok(_) => break,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "handed-off listener did not accept before deadline"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(error) => panic!("handed-off listener should accept: {error}"),
            }
        }
        second.join().expect("runtime client should join");
    }
}
