//! MP-08 / MP-09 / MP-10 / MP-11 A12: the goal's completion-check intent.
//! Only a goal is evaluated; a plain turn is never held to a check.
use super::*;

pub(crate) fn evaluation(t: &AgentTaskExecution) -> Option<&goals::GoalEvaluation> {
    t.goal.as_ref().and_then(|g| g.evaluation.as_ref())
}
pub(super) fn evaluation_mut(t: &mut AgentTaskExecution) -> Option<&mut goals::GoalEvaluation> {
    t.goal.as_mut().and_then(|g| g.evaluation.as_mut())
}
pub(crate) fn objective_revision(t: &AgentTaskExecution) -> u64 {
    t.goal.as_ref().map_or(1, |g| g.objective_revision)
}
