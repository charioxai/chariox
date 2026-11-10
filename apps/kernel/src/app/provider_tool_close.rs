//! A provider tool row still open when its run ends abnormally would show as
//! running forever: the provider that owned it is gone and will not close it.
use crate::app::prompt_activity::{
    provider_tool_phase, provider_tool_record_status, ProviderToolPhase,
};
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::history::SessionHistoryEntry;
use crate::terminal::TerminalOutputKind;

/// How an abnormal run end reads on the tool rows it leaves open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderToolEnding {
    Cancelled,
    Failed,
}

impl ProviderToolEnding {
    fn status(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Failed => "error",
        }
    }
}

/// A row that ends one open tool: its merge key and JSON bytes.
pub(crate) type EndedProviderToolRow = (Option<String>, Vec<u8>);

/// The latest record with a known status of every tool in `entries` (one
/// run's tool rows, oldest first), with the merge key its rows carry.
fn latest_provider_tool_records(
    entries: Vec<SessionHistoryEntry>,
) -> std::collections::BTreeMap<String, (Option<String>, serde_json::Map<String, serde_json::Value>)>
{
    let mut latest = std::collections::BTreeMap::new();
    for entry in entries {
        let Ok(serde_json::Value::Object(record)) =
            serde_json::from_str::<serde_json::Value>(&entry.text)
        else {
            continue;
        };
        let Some(key) = entry.merge_key.clone().or_else(|| {
            record
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        }) else {
            continue;
        };
        if provider_tool_record_status(&record)
            .as_deref()
            .and_then(provider_tool_phase)
            .is_none()
        {
            continue;
        }
        latest.insert(key, (entry.merge_key, record));
    }
    latest
}

/// A command the provider left running in the background whose end nobody
/// has reported yet.
fn is_running_background_record(record: &serde_json::Map<String, serde_json::Value>) -> bool {
    record.get("background") == Some(&serde_json::Value::Bool(true))
        && record
            .get("finished_at_ms")
            .is_some_and(serde_json::Value::is_null)
}

/// What a run's background commands are doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackgroundCalls {
    /// The run launched none.
    None,
    /// At least one has not reported its end.
    Running,
    /// Every one ended; `last_ended_at_ms` is when the last of them did.
    Ended { last_ended_at_ms: u64 },
}

/// Whether output is the provider doing work, not the report that a background call ended.
pub(crate) fn is_work_of_a_turn(kind: &TerminalOutputKind, bytes: &[u8]) -> bool {
    if *kind != TerminalOutputKind::ProviderTool {
        return true;
    }
    serde_json::from_slice::<serde_json::Value>(bytes)
        .map_or(true, |tool| tool.get("background") != Some(&true.into()))
}

/// The state of every background command in `entries` (one run's tool rows, oldest first).
pub(crate) fn background_calls(entries: Vec<SessionHistoryEntry>) -> BackgroundCalls {
    let mut calls = BackgroundCalls::None;
    for (_, record) in latest_provider_tool_records(entries).into_values() {
        if record.get("background") != Some(&serde_json::Value::Bool(true)) {
            continue;
        }
        if is_running_background_record(&record) {
            return BackgroundCalls::Running;
        }
        let ended_at_ms = record
            .get("finished_at_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        calls = match calls {
            BackgroundCalls::Ended { last_ended_at_ms } => BackgroundCalls::Ended {
                last_ended_at_ms: last_ended_at_ms.max(ended_at_ms),
            },
            _ => BackgroundCalls::Ended {
                last_ended_at_ms: ended_at_ms,
            },
        };
    }
    calls
}

fn ended_row(
    merge_key: Option<String>,
    record: serde_json::Map<String, serde_json::Value>,
) -> EndedProviderToolRow {
    (
        merge_key,
        serde_json::Value::Object(record).to_string().into_bytes(),
    )
}

/// For every tool in `entries` (one run's tool rows, oldest first) whose latest
/// status is still open, the same row with the same id and merge key, ended.
/// A background command among them ended with it.
pub(crate) fn ended_open_provider_tool_rows(
    entries: Vec<SessionHistoryEntry>,
    ending: ProviderToolEnding,
) -> Vec<EndedProviderToolRow> {
    let ended_at_ms = crate::session::unix_epoch_ms();
    latest_provider_tool_records(entries)
        .into_values()
        .filter(|(_, record)| {
            provider_tool_record_status(record)
                .as_deref()
                .and_then(provider_tool_phase)
                == Some(ProviderToolPhase::Open)
        })
        .map(|(merge_key, mut record)| {
            record.insert("status".to_string(), ending.status().into());
            if let Some(serde_json::Value::Object(state)) = record.get_mut("state") {
                state.insert("status".to_string(), ending.status().into());
            }
            if is_running_background_record(&record) {
                record.insert("finished_at_ms".to_string(), ended_at_ms.into());
            }
            ended_row(merge_key, record)
        })
        .collect()
}

/// The run's provider process is gone, so every background command it still
/// ran ended no later than `ended_at_ms`, the moment the kernel learned it.
/// One still open reads as cancelled: nobody reported how it ended.
pub(crate) fn ended_background_provider_tool_rows(
    entries: Vec<SessionHistoryEntry>,
    ended_at_ms: u64,
) -> Vec<EndedProviderToolRow> {
    latest_provider_tool_records(entries)
        .into_values()
        .filter(|(_, record)| is_running_background_record(record))
        .map(|(merge_key, mut record)| {
            record.insert("finished_at_ms".to_string(), ended_at_ms.into());
            let open = provider_tool_record_status(&record)
                .as_deref()
                .and_then(provider_tool_phase)
                == Some(ProviderToolPhase::Open);
            if open {
                record.insert(
                    "status".to_string(),
                    ProviderToolEnding::Cancelled.status().into(),
                );
            }
            ended_row(merge_key, record)
        })
        .collect()
}

impl DaemonApp {
    pub(crate) fn end_open_provider_tools(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        ending: ProviderToolEnding,
    ) -> Result<(), DaemonError> {
        let rows = ended_open_provider_tool_rows(
            self.operational_history
                .load_provider_tool_entries_for_run(session_id, provider_run_id)?,
            ending,
        );
        let recipients = self.attachments.list_session_attachment_ids(session_id);
        for (merge_key, bytes) in rows {
            self.fan_out_output_for_agent(
                session_id,
                provider_run_id,
                Some(agent_id),
                TerminalOutputKind::ProviderTool,
                merge_key,
                recipients.clone(),
                &bytes,
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_rows(snapshots: &[serde_json::Value]) -> Vec<SessionHistoryEntry> {
        snapshots
            .iter()
            .map(|snapshot| {
                SessionHistoryEntry::provider_output(
                    "session",
                    "run",
                    Some("agent"),
                    TerminalOutputKind::ProviderTool,
                    snapshot["id"].as_str().map(str::to_string),
                    snapshot.to_string(),
                )
            })
            .collect()
    }

    fn ended(rows: Vec<EndedProviderToolRow>) -> Vec<serde_json::Value> {
        rows.into_iter()
            .map(|(_, bytes)| serde_json::from_slice(&bytes).unwrap())
            .collect()
    }

    #[test]
    fn background_calls_tell_running_from_ended_by_the_latest_row_of_each() {
        let launched = |id: &str, finished: serde_json::Value| serde_json::json!({"id":id,"tool":"Bash","status":"completed","started_at_ms":1,"background":true,"finished_at_ms":finished});
        let foreground =
            serde_json::json!({"id":"fg","tool":"Read","status":"completed","started_at_ms":1});
        assert_eq!(
            background_calls(tool_rows(std::slice::from_ref(&foreground))),
            BackgroundCalls::None
        );
        assert_eq!(
            background_calls(tool_rows(&[
                foreground.clone(),
                launched("a", serde_json::Value::Null)
            ])),
            BackgroundCalls::Running
        );
        // The end of the same call replaces its running row.
        assert_eq!(
            background_calls(tool_rows(&[
                launched("a", serde_json::Value::Null),
                launched("a", 9.into()),
                launched("b", 7.into()),
            ])),
            BackgroundCalls::Ended {
                last_ended_at_ms: 9
            }
        );
        // One that still runs outweighs the ones that ended.
        assert_eq!(
            background_calls(tool_rows(&[
                launched("a", 9.into()),
                launched("b", serde_json::Value::Null)
            ])),
            BackgroundCalls::Running
        );
    }

    #[test]
    fn a_gone_process_ends_only_the_background_commands_still_running() {
        let rows = tool_rows(&[
            serde_json::json!({"id":"fg","tool":"Read","status":"completed","started_at_ms":1}),
            serde_json::json!({"id":"claude","tool":"Bash","status":"running","started_at_ms":1,"background":true,"finished_at_ms":null}),
            serde_json::json!({"id":"claude","tool":"Bash","status":"completed","started_at_ms":1,"background":true,"finished_at_ms":null}),
            serde_json::json!({"id":"codex","tool":"bash","status":"running","started_at_ms":2,"background":true,"finished_at_ms":null}),
            serde_json::json!({"id":"done","tool":"Bash","status":"completed","started_at_ms":3,"background":true,"finished_at_ms":9}),
        ]);
        let ended = ended(ended_background_provider_tool_rows(rows, 50));
        assert_eq!(ended.len(), 2, "{ended:?}");
        assert_eq!(ended[0]["id"], "claude");
        assert_eq!(ended[0]["status"], "completed");
        assert_eq!(ended[0]["finished_at_ms"], 50);
        assert_eq!(ended[1]["id"], "codex");
        assert_eq!(ended[1]["status"], "cancelled");
        assert_eq!(ended[1]["finished_at_ms"], 50);
        assert_eq!(ended[1]["started_at_ms"], 2);
    }

    #[test]
    fn an_abnormal_run_end_also_ends_an_open_background_command() {
        let rows = tool_rows(&[
            serde_json::json!({"id":"codex","tool":"bash","status":"running","started_at_ms":2,"background":true,"finished_at_ms":null}),
            serde_json::json!({"id":"open","tool":"bash","status":"running","started_at_ms":3}),
        ]);
        let ended = ended(ended_open_provider_tool_rows(
            rows,
            ProviderToolEnding::Cancelled,
        ));
        assert_eq!(ended[0]["status"], "cancelled");
        assert!(ended[0]["finished_at_ms"].as_u64().unwrap() > 2);
        assert_eq!(ended[1]["status"], "cancelled");
        assert!(ended[1].get("finished_at_ms").is_none());
    }
    use crate::app::RemoteLeaseRuntime;
    use crate::config::DaemonConfig;
    use crate::git_observer::CompletedTurnSettlementStatus;

    struct RunningTool {
        app: DaemonApp,
        session_id: String,
        agent_id: String,
        run_id: String,
    }

    /// A turn with one tool still running and one already completed.
    fn turn_with_a_running_tool() -> RunningTool {
        let mut config = DaemonConfig::for_tests();
        config.accept_remote_leases = true;
        let mut app = DaemonApp::bootstrap(config).expect("daemon should bootstrap");
        let lease = RemoteLeaseRuntime::new(&mut app)
            .create_execution_lease("home-kernel", "session-1", "agent-home-1", false, "user")
            .expect("execution lease should be created");
        let leased = RemoteLeaseRuntime::new(&mut app)
            .create_leased_agent(
                &lease.id,
                "managed-dev-stub",
                "default",
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .expect("leased agent should be created");
        let (run_id, _) = RemoteLeaseRuntime::new(&mut app)
            .submit_leased_prompt(&leased.id, "run a long command\n", Vec::new())
            .expect("prompt should start");
        for (id, status) in [("call-1", "inProgress"), ("call-2", "completed")] {
            let row = serde_json::json!({
                "id": id, "type": "commandExecution", "command": "sleep 600", "status": status,
            });
            app.fan_out_output_for_agent(
                &leased.backing_session_id,
                &run_id,
                Some(&leased.backing_agent_id),
                TerminalOutputKind::ProviderTool,
                Some(id.to_string()),
                vec![leased.backing_attachment_id.clone()],
                row.to_string().as_bytes(),
            );
        }
        RunningTool {
            app,
            session_id: leased.backing_session_id,
            agent_id: leased.backing_agent_id,
            run_id,
        }
    }

    fn tool_statuses(turn: &RunningTool) -> Vec<(String, String)> {
        turn.app
            .operational_history
            .load_provider_tool_entries_for_run(&turn.session_id, &turn.run_id)
            .expect("tool rows should load")
            .into_iter()
            .map(|entry| {
                let row: serde_json::Value = serde_json::from_str(&entry.text).unwrap();
                (
                    row["id"].as_str().unwrap().to_string(),
                    row["status"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    fn end_turn(turn: &mut RunningTool, status: CompletedTurnSettlementStatus) {
        crate::app::KernelAgentService::new(&mut turn.app)
            .complete_active_prompt_for_kernel(
                &turn.session_id,
                &turn.agent_id,
                Some(&turn.run_id),
                None,
                status,
                None,
                None,
            )
            .expect("the turn should settle");
    }

    // MP-08 / MP-10 A10: a tool that was running when its run failed stayed
    // RUNNING on every client, because nothing closed its row.
    #[test]
    fn a_failed_run_ends_its_running_tool_rows() {
        let mut turn = turn_with_a_running_tool();
        end_turn(&mut turn, CompletedTurnSettlementStatus::Failed);
        assert_eq!(
            tool_statuses(&turn),
            [
                ("call-1".to_string(), "inProgress".to_string()),
                ("call-2".to_string(), "completed".to_string()),
                ("call-1".to_string(), "error".to_string()),
            ],
            "only the open tool gets an ending row"
        );
    }

    #[test]
    fn a_cancelled_run_ends_its_running_tool_rows_as_cancelled() {
        let mut turn = turn_with_a_running_tool();
        end_turn(&mut turn, CompletedTurnSettlementStatus::Cancelled);
        assert_eq!(
            tool_statuses(&turn).last(),
            Some(&("call-1".to_string(), "cancelled".to_string()))
        );
    }

    #[test]
    fn a_completed_run_leaves_its_tool_rows_alone() {
        let mut turn = turn_with_a_running_tool();
        end_turn(&mut turn, CompletedTurnSettlementStatus::Completed);
        assert_eq!(tool_statuses(&turn).len(), 2);
    }
}
