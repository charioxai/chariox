use super::*;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProcessControlTarget {
    Agent { agent_id: String },
    Process { process_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessControlOutcome {
    OutsideEnd,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProcessControlResult {
    pub outcome: ProcessControlOutcome,
    pub process_ids: Vec<String>,
}

pub(super) enum WatchCommand {
    Shutdown,
    Stop {
        process_id: Option<String>,
        reply: mpsc::SyncSender<Result<Vec<String>, String>>,
    },
}

pub(crate) fn stop_for_key(key: &str, process_id: Option<String>) -> Result<Vec<String>, String> {
    let state = registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .and_then(std::sync::Weak::upgrade)
        .ok_or_else(|| "the process already ended; refresh Health".to_string())?;
    let sender = state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .control
        .clone()
        .ok_or_else(|| "process monitoring ended; refresh Health".to_string())?;
    let (reply, response) = mpsc::sync_channel(1);
    sender
        .send(WatchCommand::Stop { process_id, reply })
        .map_err(|_| "process monitoring ended; refresh Health".to_string())?;
    response.recv_timeout(Duration::from_secs(3)).map_err(|_| {
        "process stop did not finish within its deadline; refresh Health".to_string()
    })?
}

pub(crate) fn outside_ended_for_key(key: &str) -> bool {
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(key)
        .and_then(std::sync::Weak::upgrade)
        .is_some_and(|state| {
            state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .outside_ended
        })
}
