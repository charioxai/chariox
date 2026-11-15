//! L10-RECURRING-SCHEDULE: `trigger add schedule` and `trigger list`, the calling agent's own
//! recurring schedule triggers.
use super::result::meta_command_error;
use super::*;

const USAGE: &str = "usage: trigger add schedule (--every <N>s|m|h | --cron '<sec min hour day month weekday>' [--timezone <IANA zone>]) [--prompt <text>]";

/// The schedule source and prompt of `trigger add schedule …`. It repeats until the agent removes
/// it and skips a firing while the previous one is still handled; the kernel validates the trigger.
pub(super) fn own_schedule(
    args: &[String],
) -> Result<(crate::local::AgentTriggerSourceRequest, Option<String>), DaemonError> {
    let (mut every, mut cron, mut timezone, mut prompt) = (None, None, None, None);
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let slot = match flag.as_str() {
            "--every" => &mut every,
            "--cron" => &mut cron,
            "--timezone" => &mut timezone,
            "--prompt" => &mut prompt,
            other => {
                return Err(meta_command_error(format!(
                    "unknown option `{other}`; {USAGE}"
                )))
            }
        };
        let Some(value) = rest.next() else {
            return Err(meta_command_error(format!(
                "`{flag}` needs a value; {USAGE}"
            )));
        };
        if slot.replace(value.clone()).is_some() {
            return Err(meta_command_error(format!(
                "`{flag}` is given twice; {USAGE}"
            )));
        }
    }
    let trigger = match (every, cron, timezone) {
        (Some(every), None, None) => {
            crate::session::WorkflowScheduleTrigger::interval(interval_seconds(&every)?)
        }
        (None, Some(cron), timezone) => crate::session::WorkflowScheduleTrigger::cron(
            cron,
            timezone.unwrap_or_else(|| "UTC".to_string()),
        ),
        _ => {
            return Err(meta_command_error(format!(
                "give either --every or --cron; {USAGE}"
            )))
        }
    };
    Ok((
        crate::local::AgentTriggerSourceRequest::Schedule {
            trigger,
            repeat: crate::session::AgentTriggerRepeat::Every,
            overlap_policy: Some(crate::session::WorkflowScheduleOverlapPolicy::Skip),
            max_runs: Some(None),
        },
        prompt,
    ))
}

fn interval_seconds(every: &str) -> Result<u64, DaemonError> {
    let unit = match every.chars().last() {
        Some('s') => 1,
        Some('m') => 60,
        Some('h') => 3_600,
        _ => 0,
    };
    let count = every[..every.len().saturating_sub(1)]
        .parse::<u64>()
        .ok()
        .filter(|count| *count > 0 && unit > 0);
    count
        .map(|count| count.saturating_mul(unit))
        .ok_or_else(|| {
            meta_command_error(format!(
            "--every `{every}` must be a positive number of seconds, minutes or hours, such as 30m"
        ))
        })
}

/// One row of `trigger list`: what fires the calling agent's trigger and when it fires next.
pub(super) fn own_trigger_row(trigger: &crate::session::AgentTrigger) -> serde_json::Value {
    use crate::session::{AgentTriggerSource, WorkflowScheduleTrigger};
    let schedule = match trigger.source() {
        AgentTriggerSource::Schedule {
            trigger: WorkflowScheduleTrigger::Interval { every_seconds },
            ..
        } => Some(format!("every {every_seconds}s")),
        AgentTriggerSource::Schedule {
            trigger:
                WorkflowScheduleTrigger::Cron {
                    expression,
                    timezone,
                },
            ..
        } => Some(format!("{expression} · {timezone}")),
        _ => None,
    };
    let kind = match trigger.source() {
        AgentTriggerSource::Schedule { .. } => "schedule",
        AgentTriggerSource::Http { .. } => "http",
        AgentTriggerSource::Notification { .. } => "notification",
    };
    serde_json::json!({
        "id": trigger.id(),
        "kind": kind,
        "schedule": schedule,
        "enabled": trigger.enabled(),
        "done": trigger.done(),
        "fired": trigger.fired(),
        "next_run_at_ms": schedule.is_some().then(|| trigger.next_run_at_ms()),
        "prompt": trigger.prompt(),
    })
}
