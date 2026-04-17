use arroba_kernel::{DaemonApp, DaemonConfig};

// Tokio is the M1 async runtime baseline for the daemon because upcoming PTY,
// process, and signal-handling work all need a shared async execution model.
#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), arroba_kernel::DaemonError> {
    if let Ok(log_path) = arroba_kernel::logging::init_process_logger("daemon") {
        arroba_kernel::logging::info_with_fields(
            "daemon.main",
            "daemon process starting",
            serde_json::json!({
                "log_path": log_path.display().to_string(),
            }),
        );
    }
    let app = DaemonApp::bootstrap(DaemonConfig::load_from_env())?;

    arroba_kernel::logging::info("daemon.main", app.startup_message());
    app.run().await
}
