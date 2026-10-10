//! MP-08 / MP-09 / MP-10 / MP-11 A12: bounded plain completion confirmation.
//! The original request stays in public history; this receipt never grants goal mode.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCompletionCheck {
    pub schema_version: u32,
    pub original_prompt_id: String,
    pub check_count: u64,
    pub evaluation: Option<goals::GoalEvaluation>,
    pub final_answer: Option<String>,
    pub evidence: Vec<String>,
}

pub(super) fn admit(tx: &Transaction<'_>, task: String) -> Result<Outcome, DaemonError> {
    let mut t = load(tx, &task)?;
    if t.completion_check.is_some() {
        return Ok(Outcome::Task(t));
    }
    if t.goal.is_some()
        || t.state != ExecutionState::Working
        || t.task_id != t.prompt_id
        || t.revision != 1
        || !t.obligations.is_empty()
    {
        return Err(error(
            "completion confirmation must bind before ordinary work",
        ));
    }
    t.completion_check = Some(AgentCompletionCheck {
        schema_version: 1,
        original_prompt_id: t.task_id.clone(),
        check_count: 0,
        evaluation: None,
        final_answer: None,
        evidence: vec![],
    });
    t.revision += 1;
    save(tx, &t)?;
    Ok(Outcome::Task(t))
}

pub(crate) fn evaluation(t: &AgentTaskExecution) -> Option<&goals::GoalEvaluation> {
    t.goal
        .as_ref()
        .and_then(|g| g.evaluation.as_ref())
        .or_else(|| {
            t.completion_check
                .as_ref()
                .and_then(|c| c.evaluation.as_ref())
        })
}
pub(super) fn evaluation_mut(t: &mut AgentTaskExecution) -> Option<&mut goals::GoalEvaluation> {
    if let Some(g) = &mut t.goal {
        g.evaluation.as_mut()
    } else {
        t.completion_check
            .as_mut()
            .and_then(|c| c.evaluation.as_mut())
    }
}
pub(crate) fn objective_revision(t: &AgentTaskExecution) -> u64 {
    t.goal.as_ref().map_or(1, |g| g.objective_revision)
}
