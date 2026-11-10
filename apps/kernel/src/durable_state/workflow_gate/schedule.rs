//! Stable schedule configuration identity excludes counters and derived timers.
use super::*;
use crate::session::{WorkflowScheduleDefinition, WorkflowScheduleTrigger};
pub(crate) fn revision(schedule: &WorkflowScheduleDefinition) -> Result<String> {
    Ok(gate::sha256(
        &gate::canonical_json(
            &serde_json::json!([
                schedule.id(),
                schedule.workflow_id(),
                schedule.endpoint_id(),
                schedule.queue_id(),
                schedule.trigger(),
                schedule.invocation_prompt(),
                schedule.policy(),
                schedule.max_wakeups()
            ]),
            gate::CONFIG_BYTES,
        )
        .map_err(|e| error(e.to_string()))?,
    ))
}
pub(crate) fn input(
    schedule: &WorkflowScheduleDefinition,
    scheduled_for_ms: u64,
) -> Result<(gate::JsonObject, gate::DataSchema)> {
    let (kind, timezone) = match schedule.trigger() {
        WorkflowScheduleTrigger::Interval { .. } => ("interval", "UTC"),
        WorkflowScheduleTrigger::Cron { timezone, .. } => ("cron", timezone.as_str()),
    };
    let data=serde_json::json!({"schedule_id":schedule.id(),"schedule_revision":revision(schedule)?,"scheduled_for_ms":scheduled_for_ms,"trigger_kind":kind,"timezone":timezone}).as_object().unwrap().clone();
    let schema = serde_json::json!({"type":"object","additionalProperties":false,"required":["schedule_id","schedule_revision","scheduled_for_ms","trigger_kind","timezone"],"properties":{"schedule_id":{"type":"string"},"schedule_revision":{"type":"string"},"scheduled_for_ms":{"type":"integer"},"trigger_kind":{"enum":["interval","cron"]},"timezone":{"type":"string"}}});
    Ok((
        data,
        gate::DataSchema {
            source_kind: "schedule".into(),
            event_type: "workflow.schedule".into(),
            version: 1,
            sha256: chariox_event_protocol::payload_schema_digest(&schema).map_err(error)?,
            schema: schema.as_object().unwrap().clone(),
        },
    ))
}
