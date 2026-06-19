use std::env;
use std::process::ExitCode;

use arroba_kernel::artifacts::OperationalArtifactStore;
use arroba_kernel::config::DaemonConfig;
use arroba_kernel::config::HistoryArchiveMode;
use arroba_kernel::history::OperationalHistoryStore;
use arroba_kernel::history_archive::{
    ArtifactArchiveExporter, HistoryArchiveClient, HistoryArchiveExporter,
};

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
    let artifact_outcome =
        if config.user_config.artifacts.archive.mode == HistoryArchiveMode::External {
            let artifact_store = OperationalArtifactStore::open(
                config.operational_artifact_root(),
                config.operational_artifact_index_path(),
            )?;
            let artifact_client =
                HistoryArchiveClient::from_artifact_config(&config.user_config.artifacts.archive)?;
            let artifact_exporter = ArtifactArchiveExporter::new(artifact_store, artifact_client);
            artifact_exporter.flush_pending_once(limit)?
        } else {
            arroba_kernel::history_archive::ArtifactArchiveFlushOutcome {
                attempted_artifact_ids: Vec::new(),
                accepted_artifact_ids: Vec::new(),
                rejected_artifacts: Vec::new(),
            }
        };

    let history_store = OperationalHistoryStore::open_with_read_delay_and_max_size(
        config.operational_history_path(),
        config.operational_history_read_delay_ms,
        config.operational_history_max_size_bytes(),
    )?;
    let history_client = HistoryArchiveClient::from_config(&config.user_config.history.archive)?;
    let history_exporter = HistoryArchiveExporter::new(history_store, history_client);
    let history_outcome = history_exporter.flush_pending_once(limit)?;
    println!(
        "{}",
        serde_json::json!({
            "artifacts": {
                "attempted_artifact_ids": artifact_outcome.attempted_artifact_ids,
                "accepted_artifact_ids": artifact_outcome.accepted_artifact_ids,
                "rejected_artifacts": artifact_outcome.rejected_artifacts,
            },
            "history": {
                "attempted_event_ids": history_outcome.attempted_event_ids,
                "accepted_event_ids": history_outcome.accepted_event_ids,
                "rejected_events": history_outcome.rejected_events,
            },
        })
    );
    Ok(())
}
