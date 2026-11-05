//! MP-08 / MP-09 / MP-10 / MP-11 A12: shared goal controls policy.
use super::*;
pub(in crate::durable_state::agent_lifecycle) fn control(
    tx: &Transaction<'_>,
    task: String,
    owner: String,
    revision: u64,
    action: String,
    value: Option<String>,
    now: u64,
) -> Result<Outcome, DaemonError> {
    let mut t = load(tx, &task)?;
    if t.owner_user_id != owner
        || t.revision != revision
        || t.goal.is_none()
        || matches!(t.state, ExecutionState::Done | ExecutionState::Cancelled)
    {
        return Err(error("no longer current; reload the goal and retry"));
    }
    clock(&mut t, now);
    match action.as_str() {
        "pause" => {
            t.goal.as_mut().unwrap().paused = true;
            blocked(
                &mut t,
                "Goal paused by owner; obligations remain supervised",
            );
        }
        "resume" => {
            if t.state != ExecutionState::Blocked {
                return Err(error("only a blocked or paused goal can resume"));
            }
            let Outcome::Task(resumed) = super::super::supervision::apply(
                tx,
                Operation::OwnerResponse {
                    task,
                    revision: t.blocked_revision,
                    resume: true,
                    now,
                },
            )?
            else {
                unreachable!()
            };
            return Ok(Outcome::Task(resumed));
        }
        "cancel" => super::super::supervision::cancel_intent(tx, &mut t)?,
        "interval" => {
            let n = value
                .as_deref()
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|n| (1..=60).contains(n))
                .ok_or_else(|| error("goal interval must be 1–60 minutes"))?;
            t.goal.as_mut().unwrap().check_in_minutes = n;
            if t.state == ExecutionState::Waiting {
                super::schedule_wait_check_in(&mut t, now);
            }
        }
        "edit" => {
            let objective = value
                .filter(|v| !v.trim().is_empty() && v.len() <= RESULT_LIMIT)
                .ok_or_else(|| error("goal edit requires a bounded objective"))?;
            let g = t.goal.as_mut().unwrap();
            g.objective = crate::secret_redaction::redact_secrets(&objective).into_owned();
            g.objective_revision += 1;
            g.paused = true;
            blocked(
                &mut t,
                "Owner edited objective; explicitly resume this revision",
            );
        }
        _ => return Err(error("unknown goal control")),
    }
    t.revision += 1;
    save(tx, &t)?;
    Ok(Outcome::Task(t))
}
