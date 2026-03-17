use arroba_daemon::{DaemonApp, DaemonConfig};

// Tokio is the M1 async runtime baseline for the daemon because upcoming PTY,
// process, and signal-handling work all need a shared async execution model.
#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), arroba_daemon::DaemonError> {
    let app = DaemonApp::bootstrap(DaemonConfig::load_from_env())?;

    println!("{}", app.startup_message());
    app.run().await
}
