use std::env;
use std::process::ExitCode;

use arroba_kernel::config::DaemonConfig;
use arroba_kernel::history::OperationalHistoryStore;
use arroba_kernel::history_archive::{HistoryArchiveClient, HistoryArchiveExporter};

fn main() -> ExitCode {
    let _ = arroba_kernel::logging::init_process_logger("history-archive-flush");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), arroba_kernel::DaemonError> {
    let mut limit = 100usize;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--limit" => {
                let Some(value) = args.next() else {
                    return Err(arroba_kernel::DaemonError::InvalidConfig {
                        field: "history.archive.flush.limit",
                        message: "missing value for --limit",
                    });
                };
                limit = value.parse::<usize>().map_err(|error| {
                    arroba_kernel::DaemonError::LocalTransport {
                        operation: "history.archive.flush",
                        message: format!("--limit must be a positive integer: {error}"),
                    }
                })?;
                if limit == 0 {
                    return Err(arroba_kernel::DaemonError::InvalidConfig {
                        field: "history.archive.flush.limit",
                        message: "value must be greater than zero",
                    });
                }
            }
            "--help" | "-h" => {
                println!("Usage: arroba-history-archive-flush [--limit N]");
                return Ok(());
            }
            _ => {
                return Err(arroba_kernel::DaemonError::LocalTransport {
                    operation: "history.archive.flush",
                    message: format!("unknown argument `{arg}`"),
                });
            }
        }
    }

    let config = DaemonConfig::load_from_env();
    let store = OperationalHistoryStore::open(config.operational_history_path())?;
    let client = HistoryArchiveClient::from_config(&config.user_config.history.archive)?;
    let exporter = HistoryArchiveExporter::new(store, client);
    let outcome = exporter.flush_pending_once(limit)?;
    println!(
        "{}",
        serde_json::json!({
            "attempted_event_ids": outcome.attempted_event_ids,
            "accepted_event_ids": outcome.accepted_event_ids,
            "rejected_events": outcome.rejected_events,
        })
    );
    Ok(())
}
