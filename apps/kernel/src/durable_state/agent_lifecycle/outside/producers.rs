//! MP-08 / MP-09 / MP-10 / MP-11: project existing producers into outside parts.
use super::*;

/// Commit the notification with the first successful physical cleanup receipt.
pub(in crate::durable_state::agent_lifecycle) fn publish_deadline(
    tx: &Transaction<'_>,
    deadline: &crate::local::AgentDeadline,
) -> Result<(), DaemonError> {
    let Some((kernel, machine)) = identity(tx)? else {
        return Ok(());
    };
    let event = OutsideEvent {
        cursor: 0,
        kernel,
        machine,
        session_id: deadline.session_id.clone(),
        agent_id: Some(deadline.agent_id.clone()),
        value: OutsideEventValue::Agent {
            run_state: None,
            task_state: None,
            reason: Some("deadline_reached".into()),
            prompt_id: None,
            turn_id: None,
            outcome: None,
        },
        summary: "deadline reached".into(),
        at_ms: outside_now_ms()?,
        refs: Default::default(),
    };
    publish(
        tx,
        &format!(
            "deadline:{}",
            encode(&(&deadline.session_id, &deadline.agent_id))?
        ),
        &event,
    )
}

/// Called by the same writer transaction that changes task state.
pub(in crate::durable_state::agent_lifecycle) fn publish_task(
    tx: &Transaction<'_>,
    task: &AgentTaskExecution,
    previous: Option<&AgentTaskExecution>,
) -> Result<(), DaemonError> {
    if previous.is_some_and(|old| old.state == task.state && old.reason == task.reason) {
        return Ok(());
    }
    let Some((kernel, machine)) = identity(tx)? else {
        return Ok(());
    };
    let event = OutsideEvent {
        cursor: 0,
        kernel,
        machine,
        session_id: task.room_id.clone(),
        agent_id: Some(task.agent_id.clone()),
        value: OutsideEventValue::Agent {
            run_state: None,
            task_state: Some(task.state.clone()),
            reason: Some(
                task.reason
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(512)
                    .collect(),
            ),
            prompt_id: None,
            turn_id: None,
            outcome: None,
        },
        summary: task
            .reason
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(512)
            .collect(),
        at_ms: outside_now_ms()?,
        refs: crate::local::EventRefs {
            task_id: Some(task.task_id.clone()),
            run_id: task.provider_run_id.clone(),
            ..Default::default()
        },
    };
    publish(
        tx,
        &format!("task-{}-{}", task.task_id, task.revision),
        &event,
    )
}
pub(super) fn identity(db: &Connection) -> Result<Option<(String, String)>, DaemonError> {
    db.query_row(
        "SELECT kernel,machine FROM agent_outside_identity WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map_err(sql)
}
/// Identity for a producer outside a durable transaction. Without an open
/// subscription nothing would be kept, so the writer is not taken for it.
fn publish_identity(db: &Connection) -> Result<Option<(String, String)>, DaemonError> {
    if !parts(db)?.iter().any(|part| part.ended.is_none()) {
        return Ok(None);
    }
    identity(db)
}

impl DurableKernelStateStore {
    pub(crate) fn outside_publish(
        &self,
        session: &str,
        agent: Option<&str>,
        value: OutsideEventValue,
        summary: &str,
        refs: crate::local::EventRefs,
    ) -> Result<(), DaemonError> {
        let Some((kernel, machine)) =
            publish_identity(&*self.lock_connection("outside.identity")?)?
        else {
            return Ok(());
        };
        let event = OutsideEvent {
            cursor: 0,
            kernel,
            machine,
            session_id: session.into(),
            agent_id: agent.map(Into::into),
            value,
            summary: summary
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(512)
                .collect(),
            at_ms: outside_now_ms()?,
            refs,
        };
        self.outside_mutate(Mutation::Publish {
            occurrence: format!("producer-{:032x}", rand::random::<u128>()),
            event: Box::new(event),
        })
    }
}

impl DurableKernelStateStore {
    pub(crate) fn outside_publish_job(
        &self,
        session: &str,
        agent: Option<&str>,
        terminal: &str,
        command: Option<u64>,
        offset: u64,
        exit_code: Option<u32>,
    ) -> Result<(), DaemonError> {
        let Some((kernel, machine)) =
            publish_identity(&*self.lock_connection("outside.identity")?)?
        else {
            return Ok(());
        };
        let event = OutsideEvent {
            cursor: 0,
            kernel,
            machine,
            session_id: session.into(),
            agent_id: agent.map(Into::into),
            value: OutsideEventValue::Terminal { exit_code },
            summary: format!("exited ({exit_code:?})"),
            at_ms: outside_now_ms()?,
            refs: crate::local::EventRefs {
                terminal_id: Some(terminal.into()),
                ..Default::default()
            },
        };
        self.outside_mutate(Mutation::Publish {
            occurrence: format!("terminal-{terminal}-{command:?}-{offset}"),
            event: Box::new(event),
        })
    }
}
