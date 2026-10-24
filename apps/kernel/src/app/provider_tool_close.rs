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

/// For every tool in `entries` (one run's tool rows, oldest first) whose latest
/// status is still open, the same row with the same id and merge key, ended.
pub(crate) fn ended_open_provider_tool_rows(
    entries: Vec<SessionHistoryEntry>,
    ending: ProviderToolEnding,
) -> Vec<EndedProviderToolRow> {
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
        let Some(phase) = provider_tool_record_status(&record)
            .as_deref()
            .and_then(provider_tool_phase)
        else {
            continue;
        };
        latest.insert(key, (entry.merge_key, record, phase));
    }
    latest
        .into_values()
        .filter(|(_, _, phase)| *phase == ProviderToolPhase::Open)
        .map(|(merge_key, mut record, _)| {
            record.insert("status".to_string(), ending.status().into());
            if let Some(serde_json::Value::Object(state)) = record.get_mut("state") {
                state.insert("status".to_string(), ending.status().into());
            }
            let bytes = serde_json::Value::Object(record).to_string().into_bytes();
            (merge_key, bytes)
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
