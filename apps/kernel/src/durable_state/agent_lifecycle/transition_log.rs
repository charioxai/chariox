//! One info line for every ledger step that can end a turn's task or raise a card for it, whoever
//! wrote it: a settle, a correction, a block, a failed turn's end, a sweep. Each names the task,
//! its prompt, the state it reached, why, and the work still open, so a card can be traced to the
//! step that raised it from the kernel log alone.
use super::*;

/// The step a ledger operation takes on a task's turn, if it is one this log follows.
pub(super) fn followed_step(operation: &Operation) -> Option<&'static str> {
    Some(match operation {
        Operation::Settle { .. } => "settle",
        Operation::EndFailedTurn { .. } => "end failed turn",
        Operation::Block { .. } => "block",
        Operation::BlockForBrowserAccess { .. } => "block for browser access",
        Operation::BlockForQuestion { .. } => "block for question",
        Operation::Sweep { .. } => "sweep",
        Operation::RetireRestartFailure { .. } => "retire restart failure",
        _ => return None,
    })
}

pub(super) fn log_step(step: &str, outcome: &Outcome) {
    match outcome {
        Outcome::Settled { task, correction } => log_task(step, task, *correction),
        Outcome::Task(task) => log_task(step, task, false),
        Outcome::Swept(tasks) => {
            for task in tasks {
                log_task(step, task, false);
            }
        }
        _ => {}
    }
}

fn log_task(step: &str, task: &AgentTaskExecution, correction: bool) {
    let open: Vec<_> = task
        .obligations
        .iter()
        .filter(|obligation| obligation.requires_resolution())
        .map(|obligation| {
            serde_json::json!({
                "id": obligation.id, "kind": obligation.kind, "status": obligation.status,
                "outlives_turn": obligation.outlives_turn(),
            })
        })
        .collect();
    crate::logging::info_with_fields(
        "durable.agent_lifecycle",
        "an agent task's turn changed",
        serde_json::json!({
            "step": step, "room_id": task.room_id, "agent_id": task.agent_id,
            "task_id": task.task_id, "prompt_id": task.prompt_id,
            "provider_run_id": task.provider_run_id, "state": task.state,
            "reason": crate::secret_redaction::redact_secrets(&task.reason),
            "correction": correction, "pending_prompt_id": task.pending_prompt_id,
            "blocked_revision": task.blocked_revision, "open_obligations": open,
            "waiting": task.wait.is_some(),
        }),
    );
}
