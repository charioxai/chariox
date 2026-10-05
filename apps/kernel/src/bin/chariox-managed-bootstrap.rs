#[cfg(all(not(test), not(debug_assertions)))]
const _: () = chariox_app_runtime::assert_production_build();

fn main() -> Result<(), chariox_kernel::DaemonError> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let command = args.first().and_then(|arg| arg.to_str());
    let disposable_worker = match (command, args.len()) {
        (None, 0) => false,
        (Some("--disposable-worker"), 1) => true,
        (Some("--help" | "-h"), 1) => {
            println!("usage: chariox-managed-bootstrap [--disposable-worker | --help | --version]\n\nRun the managed home supervisor with no arguments, or bootstrap a disposable\nworker with --disposable-worker. Configuration is supplied through the environment.\n  -h, --help  Print usage and exit\n  --version   Print the package version and exit");
            return Ok(());
        }
        (Some("--version"), 1) => {
            println!("chariox-managed-bootstrap {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        _ => {
            eprintln!("error: unknown or additional managed-bootstrap argument\nRun chariox-managed-bootstrap --help for usage.");
            std::process::exit(2);
        }
    };
    if let Ok(log_path) = chariox_kernel::logging::init_process_logger("managed-bootstrap") {
        chariox_kernel::logging::info_with_fields(
            "managed_bootstrap.start",
            "managed kernel bootstrap supervisor starting",
            serde_json::json!({ "log_path": log_path.display().to_string() }),
        );
    }
    if disposable_worker {
        chariox_kernel::managed_bootstrap::worker::run_from_env()
    } else {
        chariox_kernel::managed_bootstrap::run_from_env()
    }
}
