use serde_json::{json, Value};

use crate::error::DaemonError;
use crate::local::LocalDaemonResponse;
use crate::runtime::command::{KernelCommand, KernelCommandPriority, KernelCommandSource};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommandTrace {
    command_id: String,
    command_type: String,
    trace_id: String,
    source: KernelCommandSource,
    priority: KernelCommandPriority,
    submitted_at_ms: u64,
}

impl CommandTrace {
    pub(crate) fn from_command(command: &KernelCommand) -> Self {
        Self {
            command_id: command.command_id.clone(),
            command_type: command.command_type.clone(),
            trace_id: command.correlation_id.clone(),
            source: command.source.clone(),
            priority: command.priority.clone(),
            submitted_at_ms: command.submitted_at_ms,
        }
    }

    pub(crate) fn command_id(&self) -> &str {
        &self.command_id
    }

    pub(crate) fn command_type(&self) -> &str {
        &self.command_type
    }

    fn base_fields(&self, now_ms: u64) -> Value {
        json!({
            "trace_id": self.trace_id,
            "command_id": self.command_id,
            "command_type": self.command_type,
            "source": command_source_label(&self.source),
            "priority": command_priority_label(&self.priority),
            "submitted_at_ms": self.submitted_at_ms,
            "command_age_ms": elapsed_ms(self.submitted_at_ms, now_ms),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LaneCommandTrace {
    command: CommandTrace,
    enqueued_at_ms: u64,
}

impl LaneCommandTrace {
    pub(crate) fn new(command: CommandTrace, enqueued_at_ms: u64) -> Self {
        Self {
            command,
            enqueued_at_ms,
        }
    }

    pub(crate) fn command_id(&self) -> &str {
        self.command.command_id()
    }

    pub(crate) fn command_type(&self) -> &str {
        self.command.command_type()
    }

    fn queue_fields(
        &self,
        lane_kind: &'static str,
        lane_id: &str,
        queue_limit: usize,
        queue_depth_before: usize,
        queue_depth_after: usize,
        now_ms: u64,
    ) -> Value {
        merge_fields(
            self.command.base_fields(now_ms),
            json!({
                "lane_kind": lane_kind,
                "lane_id": lane_id,
                "queue_limit": queue_limit,
                "queue_depth_before": queue_depth_before,
                "queue_depth_after": queue_depth_after,
                "enqueued_at_ms": self.enqueued_at_ms,
                "enqueue_delay_ms": elapsed_ms(self.command.submitted_at_ms, self.enqueued_at_ms),
            }),
        )
    }

    fn dispatch_fields(&self, lane_kind: &'static str, lane_id: &str, now_ms: u64) -> Value {
        merge_fields(
            self.command.base_fields(now_ms),
            json!({
                "lane_kind": lane_kind,
                "lane_id": lane_id,
                "enqueued_at_ms": self.enqueued_at_ms,
                "queue_wait_ms": elapsed_ms(self.enqueued_at_ms, now_ms),
                "submit_to_dispatch_ms": elapsed_ms(self.command.submitted_at_ms, now_ms),
            }),
        )
    }

    fn completion_fields(
        &self,
        lane_kind: &'static str,
        lane_id: &str,
        dispatch_started_at_ms: u64,
        result: &Result<LocalDaemonResponse, DaemonError>,
        now_ms: u64,
    ) -> Value {
        let mut fields = merge_fields(
            self.dispatch_fields(lane_kind, lane_id, now_ms),
            json!({
                "dispatch_started_at_ms": dispatch_started_at_ms,
                "lane_execution_ms": elapsed_ms(dispatch_started_at_ms, now_ms),
                "submit_to_complete_ms": elapsed_ms(self.command.submitted_at_ms, now_ms),
                "status": if result.is_ok() { "ok" } else { "error" },
            }),
        );
        if let Err(error) = result {
            fields = merge_fields(fields, json!({ "error": error.to_string() }));
        }
        fields
    }
}

pub(crate) fn now_ms() -> u64 {
    crate::session::unix_epoch_ms()
}

pub(crate) fn log_command_received(trace: &CommandTrace) {
    crate::logging::info_with_fields(
        "daemon.command_latency",
        "kernel command received",
        trace.base_fields(now_ms()),
    );
}

pub(crate) fn log_command_completed(
    trace: &CommandTrace,
    result: &Result<LocalDaemonResponse, DaemonError>,
) {
    let now_ms = now_ms();
    let mut fields = merge_fields(
        trace.base_fields(now_ms),
        json!({
            "completed_at_ms": now_ms,
            "submit_to_complete_ms": elapsed_ms(trace.submitted_at_ms, now_ms),
            "status": if result.is_ok() { "ok" } else { "error" },
        }),
    );
    if let Err(error) = result {
        fields = merge_fields(fields, json!({ "error": error.to_string() }));
    }
    crate::logging::info_with_fields("daemon.command_latency", "kernel command completed", fields);
}

pub(crate) fn log_lane_enqueued(
    trace: &LaneCommandTrace,
    lane_kind: &'static str,
    lane_id: &str,
    queue_limit: usize,
    queue_depth_before: usize,
    queue_depth_after: usize,
) {
    crate::logging::info_with_fields(
        "daemon.command_latency",
        "kernel command enqueued",
        trace.queue_fields(
            lane_kind,
            lane_id,
            queue_limit,
            queue_depth_before,
            queue_depth_after,
            now_ms(),
        ),
    );
}

pub(crate) fn log_lane_enqueue_failed(
    trace: &LaneCommandTrace,
    lane_kind: &'static str,
    lane_id: &str,
    queue_limit: usize,
    queue_depth_before: usize,
    error: &str,
) {
    let fields = merge_fields(
        trace.queue_fields(
            lane_kind,
            lane_id,
            queue_limit,
            queue_depth_before,
            queue_depth_before,
            now_ms(),
        ),
        json!({
            "status": "error",
            "error": error,
        }),
    );
    crate::logging::warn_with_fields(
        "daemon.command_latency",
        "kernel command enqueue failed",
        fields,
    );
}

pub(crate) fn log_lane_dispatched(
    trace: &LaneCommandTrace,
    lane_kind: &'static str,
    lane_id: &str,
    dispatch_started_at_ms: u64,
) {
    crate::logging::info_with_fields(
        "daemon.command_latency",
        "kernel command lane dispatched",
        trace.dispatch_fields(lane_kind, lane_id, dispatch_started_at_ms),
    );
}

pub(crate) fn log_lane_completed(
    trace: &LaneCommandTrace,
    lane_kind: &'static str,
    lane_id: &str,
    dispatch_started_at_ms: u64,
    result: &Result<LocalDaemonResponse, DaemonError>,
) {
    crate::logging::info_with_fields(
        "daemon.command_latency",
        "kernel command lane completed",
        trace.completion_fields(lane_kind, lane_id, dispatch_started_at_ms, result, now_ms()),
    );
}

fn elapsed_ms(start_ms: u64, end_ms: u64) -> u64 {
    end_ms.saturating_sub(start_ms)
}

fn command_source_label(source: &KernelCommandSource) -> &'static str {
    match source {
        KernelCommandSource::LocalCli => "local_cli",
        KernelCommandSource::LocalIpc => "local_ipc",
        KernelCommandSource::RelayClient => "relay_client",
        KernelCommandSource::RelayPeer => "relay_peer",
        KernelCommandSource::DaemonBackground => "daemon_background",
    }
}

fn command_priority_label(priority: &KernelCommandPriority) -> &'static str {
    match priority {
        KernelCommandPriority::Interactive => "interactive",
        KernelCommandPriority::Normal => "normal",
        KernelCommandPriority::Background => "background",
    }
}

fn merge_fields(mut base: Value, extra: Value) -> Value {
    if let (Some(base), Some(extra)) = (base.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            base.insert(key.clone(), value.clone());
        }
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::{GetDaemonHealthRequest, LocalDaemonRequest};
    use crate::runtime::command::KernelCommand;

    #[test]
    fn command_trace_uses_correlation_id_as_trace_id() {
        let request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
        let command = KernelCommand::from_local_request(
            "cmd-health",
            Some("trace-health".to_string()),
            None,
            &request,
        );

        let trace = CommandTrace::from_command(&command);
        let fields = trace.base_fields(command.submitted_at_ms + 7);

        assert_eq!(fields["trace_id"], "trace-health");
        assert_eq!(fields["command_id"], "cmd-health");
        assert_eq!(fields["command_type"], "daemon.health.get");
        assert_eq!(fields["priority"], "normal");
        assert_eq!(fields["command_age_ms"], 7);
    }

    #[test]
    fn lane_dispatch_fields_include_queue_wait_and_total_age() {
        let request = LocalDaemonRequest::GetDaemonHealth(GetDaemonHealthRequest);
        let mut command = KernelCommand::from_local_request("cmd-health", None, None, &request);
        command.submitted_at_ms = 100;
        let trace = LaneCommandTrace::new(CommandTrace::from_command(&command), 130);

        let fields = trace.dispatch_fields("session", "session-1", 145);

        assert_eq!(fields["trace_id"], "cmd-health");
        assert_eq!(fields["lane_kind"], "session");
        assert_eq!(fields["lane_id"], "session-1");
        assert_eq!(fields["queue_wait_ms"], 15);
        assert_eq!(fields["submit_to_dispatch_ms"], 45);
    }

    #[test]
    fn elapsed_ms_saturates_for_clock_skew() {
        assert_eq!(elapsed_ms(200, 100), 0);
    }
}
