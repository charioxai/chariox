use chariox_kernel::{DaemonApp, DaemonConfig};
use std::time::Instant;

mod kernel_arguments;

#[cfg(all(not(test), not(debug_assertions)))]
const _: () = chariox_app_runtime::assert_production_build();

// The kernel clones sessions, snapshots and history pages on every pump and
// request. The system allocators keep that freed memory in per-thread arenas or
// zones, so a home kernel serving leased agents grew far past its live heap.
// mimalloc returns freed pages to the OS and keeps RSS near the live heap.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

// Tokio is the M1 async runtime baseline for the daemon because upcoming PTY,
// process, and signal-handling work all need a shared async execution model.
fn main() -> Result<(), chariox_kernel::DaemonError> {
    #[cfg(all(feature = "native-display", target_os = "linux"))]
    if let Some(arg) = std::env::args_os().nth(1).filter(|arg| {
        arg == "--display-native-worker"
            || arg == "--display-native-codec-probe"
            || arg == "--display-native-windows"
    }) {
        let result = if arg == "--display-native-codec-probe" {
            chariox_kernel::display_native::probe_codec()
        } else if arg == "--display-native-windows" {
            chariox_kernel::display_native::inventory()
        } else {
            chariox_kernel::display_native::run()
        };
        if let Err(error) = result {
            // MP-08/MP-10: fixed-label stage for the host's refusal reason.
            let stage: String = error
                .rsplit(':')
                .next()
                .unwrap_or_default()
                .trim()
                .chars()
                .filter(|c| c.is_ascii_alphabetic() || *c == ' ')
                .map(|c| {
                    if c == ' ' {
                        '_'
                    } else {
                        c.to_ascii_lowercase()
                    }
                })
                .take(48)
                .collect();
            eprintln!("MD-DISPLAY: native stage {stage}");
            std::process::exit(1);
        }
        if arg == "--display-native-codec-probe" {
            println!(
                "MP-08/MP-10: native decoder {}",
                chariox_kernel::display_native::codec_decoder_library()
            );
        }
        return Ok(());
    }
    let _process_logs = chariox_kernel::logging::ProcessLogGuard;
    let command =
        kernel_arguments::parse(std::env::args_os().skip(1).collect()).unwrap_or_else(|message| {
            eprintln!("error: {message}\nRun chariox-kernel --help for usage.");
            std::process::exit(2);
        });
    match command {
        kernel_arguments::Command::Help => {
            println!("{}", kernel_arguments::USAGE);
            return Ok(());
        }
        kernel_arguments::Command::Version => {
            println!("chariox-kernel {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        kernel_arguments::Command::ProtocolVersion => {
            println!("{}", chariox_kernel::local::LOCAL_DAEMON_PROTOCOL_VERSION);
            return Ok(());
        }
        kernel_arguments::Command::HandoverFormat => {
            #[cfg(unix)]
            println!("{}", chariox_kernel::hot_reload::HANDOVER_FORMAT);
            return Ok(());
        }
        kernel_arguments::Command::PrepareProtectedSliceIdentity(port) => {
            let proof =
                chariox_kernel::config::prepare_protected_slice_identity("127.0.0.1", port)?;
            println!("{proof}");
            return Ok(());
        }
        mode @ (kernel_arguments::Command::OwnerManagedEnroll
        | kernel_arguments::Command::OwnerManagedReady
        | kernel_arguments::Command::OwnerManagedSelfEnroll
        | kernel_arguments::Command::OwnerManagedDeviceEnroll) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| chariox_kernel::DaemonError::LocalTransport {
                    operation: "owner-managed bootstrap",
                    message: "runtime unavailable".into(),
                })?;
            return runtime.block_on(async {
                match mode {
                    kernel_arguments::Command::OwnerManagedEnroll => {
                        chariox_kernel::owner_managed_bootstrap::enroll_from_stdin(true).await
                    }
                    kernel_arguments::Command::OwnerManagedSelfEnroll => {
                        chariox_kernel::owner_managed_bootstrap::enroll_from_stdin(false).await
                    }
                    kernel_arguments::Command::OwnerManagedDeviceEnroll => {
                        chariox_kernel::owner_managed_bootstrap::enroll_device_from_stdin().await
                    }
                    kernel_arguments::Command::OwnerManagedReady => {
                        chariox_kernel::owner_managed_bootstrap::wait_ready().await
                    }
                    _ => unreachable!(),
                }
            });
        }
        kernel_arguments::Command::Run => {}
    }
    #[cfg(unix)]
    chariox_kernel::hot_reload::remember_start();
    chariox_kernel::slice::initialize_managed_docker_broker();
    chariox_kernel::runtime_transport::initialize_kernel_local_auth_from_env()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(chariox_kernel::runtime_transport::KERNEL_RUNTIME_THREAD_STACK_SIZE)
        .build()
        .map_err(|error| chariox_kernel::DaemonError::LocalTransport {
            operation: "daemon runtime",
            message: format!("failed to start Tokio runtime: {error}"),
        })?;
    runtime.block_on(async_main())
}

async fn async_main() -> Result<(), chariox_kernel::DaemonError> {
    let process_started = Instant::now();
    if let Ok(log_path) = chariox_kernel::logging::init_process_logger("daemon") {
        chariox_kernel::logging::info_with_fields(
            "daemon.main",
            "daemon process starting",
            serde_json::json!({
                "log_path": log_path.display().to_string(),
            }),
        );
    }
    #[cfg(unix)]
    chariox_kernel::hot_reload::receive();
    chariox_kernel::managed_context::kernel::scavenge_source_snapshots();
    let config_started = Instant::now();
    let config = DaemonConfig::load_from_env();
    for name in [
        "CHARIOX_RELAY_TOKEN",
        "CHARIOX_CLOUD_RELAY_CONFIG_JSON",
        "CHARIOX_CLOUD_RELAY_CONFIG_PATH",
    ] {
        std::env::remove_var(name);
    }
    chariox_kernel::logging::info_with_fields(
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
    let bind_started = Instant::now();
    let boot_listener = chariox_kernel::runtime_transport::KernelBootListener::bind(
        &config.kernel_websocket_host,
        config.kernel_websocket_port,
    )?;
    chariox_kernel::logging::info_with_fields(
        "daemon.startup",
        "kernel boot listener bound",
        serde_json::json!({
            "phase": "booting",
            "bind_ms": bind_started.elapsed().as_millis(),
            "process_elapsed_ms": process_started.elapsed().as_millis(),
            "bind_host": config.kernel_websocket_host,
            "bind_port": config.kernel_websocket_port,
            "runtime_ready": false,
            "event_ready": false,
        }),
    );
    let bootstrap_started = Instant::now();
    let app = DaemonApp::bootstrap(config)?;
    chariox_kernel::logging::info_with_fields(
        "daemon.startup",
        "daemon app bootstrapped",
        serde_json::json!({
            "bootstrap_ms": bootstrap_started.elapsed().as_millis(),
            "process_elapsed_ms": process_started.elapsed().as_millis(),
        }),
    );

    chariox_kernel::logging::info("daemon.main", app.startup_message());
    let result = app.run_on_listener(boot_listener.into_listener()?).await;
    #[cfg(unix)]
    if let Some(error) = chariox_kernel::hot_reload::finish() {
        // Everything already stopped as for a restart: exit as failed, so the
        // supervisor starts the installed program as after any restart.
        chariox_kernel::logging::error_with_fields(
            "hot_reload",
            "the new kernel program could not start; exiting for a restart",
            serde_json::json!({ "error": error }),
        );
        std::process::exit(1);
    }
    result
}
