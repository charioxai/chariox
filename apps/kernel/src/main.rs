use arroba_kernel::{DaemonApp, DaemonConfig};
use std::time::Instant;

// Tokio is the M1 async runtime baseline for the daemon because upcoming PTY,
// process, and signal-handling work all need a shared async execution model.
fn main() -> Result<(), arroba_kernel::DaemonError> {
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new(
            "--print-local-daemon-protocol-version",
        ))
    {
        println!("{}", arroba_kernel::local::LOCAL_DAEMON_PROTOCOL_VERSION);
        return Ok(());
    }
    arroba_kernel::runtime_transport::initialize_kernel_local_auth_from_env()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(arroba_kernel::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
        .build()
        .map_err(|error| arroba_kernel::DaemonError::LocalTransport {
            operation: "daemon runtime",
            message: format!("failed to start Tokio runtime: {error}"),
        })?;
    runtime.block_on(async_main())
}

async fn async_main() -> Result<(), arroba_kernel::DaemonError> {
    let process_started = Instant::now();
    if let Ok(log_path) = arroba_kernel::logging::init_process_logger("daemon") {
        arroba_kernel::logging::info_with_fields(
            "daemon.main",
            "daemon process starting",
            serde_json::json!({
                "log_path": log_path.display().to_string(),
            }),
        );
    }
    let config_started = Instant::now();
    let config = DaemonConfig::load_from_env();
    arroba_kernel::logging::info_with_fields(
        "daemon.startup",
        "daemon config loaded",
        serde_json::json!({
            "config_load_ms": config_started.elapsed().as_millis(),
            "process_elapsed_ms": process_started.elapsed().as_millis(),
            "user_config_path": config.user_config_path().display().to_string(),
            "relay_configured": config.relay_url.is_some() && config.relay_token.is_some(),
            "cloud_profile_present": config.cloud_relay.is_some(),
            "kernel_websocket_url": config.kernel_websocket_url(),
            "local_socket_path": config.local_socket_path.display().to_string(),
        }),
    );
    let bootstrap_started = Instant::now();
    let app = DaemonApp::bootstrap(config)?;
    arroba_kernel::logging::info_with_fields(
        "daemon.startup",
        "daemon app bootstrapped",
        serde_json::json!({
            "bootstrap_ms": bootstrap_started.elapsed().as_millis(),
            "process_elapsed_ms": process_started.elapsed().as_millis(),
        }),
    );

    arroba_kernel::logging::info("daemon.main", app.startup_message());
    app.run().await
}
