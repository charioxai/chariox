#[cfg(all(not(test), not(debug_assertions)))]
const _: () = chariox_app_runtime::assert_production_build();

fn main() -> Result<(), chariox_kernel::DaemonError> {
    let _process_logs = chariox_kernel::logging::ProcessLogGuard;
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let command = args.first().and_then(|arg| arg.to_str());
    let step = match (command, args.len()) {
        (None, 0) => Step::Supervise,
        (Some("--disposable-worker"), 1) => Step::DisposableWorker,
        (Some("--redeem-grant"), 1) => Step::RedeemGrant,
        (Some("--help" | "-h"), 1) => {
            println!("usage: chariox-managed-bootstrap [--disposable-worker | --redeem-grant | --help | --version]\n\nRun the managed home supervisor with no arguments, or bootstrap a disposable\nworker with --disposable-worker. --redeem-grant is the root step that blanks the\nbootstrap token once Cloud confirmed the kernel. Configuration is supplied through\nthe environment.\n  -h, --help  Print usage and exit\n  --version   Print the package version and exit");
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
    if matches!(step, Step::RedeemGrant) {
        // Root must not create files in the supervisor's log directory.
        return chariox_kernel::managed_bootstrap::grant_redemption::run_from_env();
    }
    if let Ok(log_path) = chariox_kernel::logging::init_process_logger("managed-bootstrap") {
        chariox_kernel::logging::info_with_fields(
            "managed_bootstrap.start",
            "managed kernel bootstrap supervisor starting",
            serde_json::json!({ "log_path": log_path.display().to_string() }),
        );
    }
    match step {
        Step::DisposableWorker => chariox_kernel::managed_bootstrap::worker::run_from_env(),
        Step::RedeemGrant => unreachable!("handled before logging starts"),
        Step::Supervise => chariox_kernel::managed_bootstrap::run_from_env(),
    }
}

enum Step {
    Supervise,
    DisposableWorker,
    RedeemGrant,
}
