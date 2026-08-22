use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::app::{ActivePromptState, ActiveTurnState};
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

    pub(crate) fn trace_id(&self) -> &str {
        &self.trace_id
    }

    fn fields_at(&self, now_ms: u64) -> Value {
        self.base_fields(now_ms)
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
    if is_quiet_success_command_type(trace.command_type()) || !sampled(trace.command_id()) {
        return;
    }
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
    if result.is_ok()
        && (is_quiet_success_command_type(trace.command_type()) || !sampled(trace.command_id()))
    {
        return;
    }
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

pub(crate) fn is_quiet_success_command_type(command_type: &str) -> bool {
    matches!(
        command_type,
        "relay.status" | "waiting_room.public_snapshot.get" | "slice.list" | "provider.catalog.get"
    )
}

pub(crate) fn log_lane_enqueued(
    trace: &LaneCommandTrace,
    lane_kind: &'static str,
    lane_id: &str,
    queue_limit: usize,
    queue_depth_before: usize,
    queue_depth_after: usize,
) {
    if !sampled(trace.command_id()) {
        return;
    }
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
    if !sampled(trace.command_id()) {
        return;
    }
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
    if result.is_ok() && !sampled(trace.command_id()) {
        return;
    }
    crate::logging::info_with_fields(
        "daemon.command_latency",
        "kernel command lane completed",
        trace.completion_fields(lane_kind, lane_id, dispatch_started_at_ms, result, now_ms()),
    );
}

pub(crate) fn log_provider_launch_accepted(
    trace: &CommandTrace,
    run: &crate::provider::RuntimeProviderRun,
    launch_started_at_ms: u64,
    runtime_init_delay_ms: u64,
) {
    if !sampled(trace.command_id()) {
        return;
    }
    let now_ms = now_ms();
    crate::logging::info_with_fields(
        "daemon.provider_latency",
        "provider launch accepted",
        merge_fields(
            merge_fields(trace.fields_at(now_ms), provider_run_fields(run)),
            json!({
                "launch_started_at_ms": launch_started_at_ms,
                "prepare_and_spawn_ms": elapsed_ms(launch_started_at_ms, now_ms),
                "runtime_init_delay_ms": runtime_init_delay_ms,
            }),
        ),
    );
}

pub(crate) fn log_provider_runtime_binding_started(
    trace: &CommandTrace,
    run: &crate::provider::RuntimeProviderRun,
    launch_started_at_ms: u64,
) -> u64 {
    let now_ms = now_ms();
    if !sampled(trace.command_id()) {
        return now_ms;
    }
    crate::logging::info_with_fields(
        "daemon.provider_latency",
        "provider runtime binding started",
        merge_fields(
            merge_fields(trace.fields_at(now_ms), provider_run_fields(run)),
            json!({
                "launch_started_at_ms": launch_started_at_ms,
                "binding_started_at_ms": now_ms,
                "launch_to_binding_start_ms": elapsed_ms(launch_started_at_ms, now_ms),
            }),
        ),
    );
    now_ms
}

pub(crate) fn log_provider_runtime_binding_succeeded(
    trace: &CommandTrace,
    run: &crate::provider::RuntimeProviderRun,
    launch_started_at_ms: u64,
    binding_started_at_ms: u64,
) {
    log_provider_runtime_binding_completed(
        trace,
        run,
        launch_started_at_ms,
        binding_started_at_ms,
        "ok",
        None,
    );
}

pub(crate) fn log_provider_runtime_binding_failed(
    trace: &CommandTrace,
    run: &crate::provider::RuntimeProviderRun,
    launch_started_at_ms: u64,
    binding_started_at_ms: u64,
    error: &DaemonError,
) {
    log_provider_runtime_binding_completed(
        trace,
        run,
        launch_started_at_ms,
        binding_started_at_ms,
        "error",
        Some(error.to_string()),
    );
}

pub(crate) fn log_provider_first_response_content(
    run: &crate::provider::RuntimeProviderRun,
    active_turn: Option<&ActiveTurnState>,
) {
    if !sampled(run.id()) {
        return;
    }
    crate::logging::info_with_fields(
        "daemon.provider_latency",
        "provider first response content observed",
        provider_first_output_fields(run, active_turn, now_ms()),
    );
}

pub(crate) fn log_provider_turn_completed(
    run: &crate::provider::RuntimeProviderRun,
    active_turn: Option<&ActiveTurnState>,
    prompt_activity: Option<&ActivePromptState>,
) {
    if !sampled(run.id()) {
        return;
    }
    crate::logging::info_with_fields(
        "daemon.provider_latency",
        "provider turn activity completed",
        provider_turn_completion_fields(run, active_turn, prompt_activity, now_ms()),
    );
}

fn sampled(key: &str) -> bool {
    if cfg!(test) {
        return true;
    }
    static CONFIG: OnceLock<Option<u64>> = OnceLock::new();
    let Some(sample_rate) = *CONFIG.get_or_init(|| {
        let enabled = std::env::var("CHARIOX_PERF_DIAGNOSTICS")
            .ok()
            .is_some_and(|value| matches!(value.trim(), "1" | "true" | "yes" | "on"));
        enabled.then(|| {
            std::env::var("CHARIOX_PERF_DIAGNOSTICS_SAMPLE_RATE")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(100)
                .max(1)
        })
    }) else {
        return false;
    };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish().is_multiple_of(sample_rate)
}

fn elapsed_ms(start_ms: u64, end_ms: u64) -> u64 {
    end_ms.saturating_sub(start_ms)
}

fn duration_ms(duration: std::time::Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
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

fn provider_run_fields(run: &crate::provider::RuntimeProviderRun) -> Value {
    json!({
        "provider_run_id": run.id(),
        "session_id": run.session_id(),
        "agent_id": run.agent_instance_id(),
        "adapter_key": run.adapter_key(),
        "provider": run.provider(),
        "model": run.model(),
    })
}

fn provider_first_output_fields(
    run: &crate::provider::RuntimeProviderRun,
    active_turn: Option<&ActiveTurnState>,
    now_ms: u64,
) -> Value {
    let trace_id = active_turn
        .map(|turn| turn.trace_id.as_str())
        .unwrap_or_else(|| run.id());
    let mut fields = merge_fields(
        json!({
            "trace_id": trace_id,
        }),
        provider_run_fields(run),
    );
    fields = merge_fields(
        fields,
        json!({
            "prompt_id": active_turn.map(|turn| turn.prompt_id.as_str()),
            "turn_started_at_ms": active_turn.map(|turn| turn.started_at_ms),
            "first_output_at_ms": now_ms,
            "provider_run_started_at_ms": run.started_at_ms(),
            "provider_run_to_first_output_ms": elapsed_ms(run.started_at_ms(), now_ms),
            "output_kind": "response_content",
        }),
    );
    if let Some(turn) = active_turn {
        fields = merge_fields(
            fields,
            json!({
                "prompt_to_first_output_ms": elapsed_ms(turn.started_at_ms, now_ms),
            }),
        );
    }
    fields
}

fn provider_turn_completion_fields(
    run: &crate::provider::RuntimeProviderRun,
    active_turn: Option<&ActiveTurnState>,
    prompt_activity: Option<&ActivePromptState>,
    now_ms: u64,
) -> Value {
    let trace_id = active_turn
        .map(|turn| turn.trace_id.as_str())
        .unwrap_or_else(|| run.id());
    let mut fields = merge_fields(
        json!({
            "trace_id": trace_id,
        }),
        provider_run_fields(run),
    );
    fields = merge_fields(
        fields,
        json!({
            "prompt_id": active_turn.map(|turn| turn.prompt_id.as_str()),
            "turn_started_at_ms": active_turn.map(|turn| turn.started_at_ms),
            "completed_at_ms": now_ms,
            "provider_run_started_at_ms": run.started_at_ms(),
            "provider_run_to_completion_ms": elapsed_ms(run.started_at_ms(), now_ms),
            "saw_response_content": prompt_activity.map(|state| state.saw_response_content),
            "completion_recorded": prompt_activity.map(|state| state.completion_recorded),
            "settlement_requested": prompt_activity.map(|state| state.settlement_requested),
            "last_output_to_completion_ms": prompt_activity
                .and_then(|state| state.last_output_at)
                .map(|last_output_at| duration_ms(last_output_at.elapsed())),
        }),
    );
    if let Some(turn) = active_turn {
        fields = merge_fields(
            fields,
            json!({
                "prompt_to_completion_ms": elapsed_ms(turn.started_at_ms, now_ms),
            }),
        );
    }
    fields
}

fn log_provider_runtime_binding_completed(
    trace: &CommandTrace,
    run: &crate::provider::RuntimeProviderRun,
    launch_started_at_ms: u64,
    binding_started_at_ms: u64,
    status: &'static str,
    error: Option<String>,
) {
    if status == "ok" && !sampled(trace.command_id()) {
        return;
    }
    let now_ms = now_ms();
    let mut fields = merge_fields(
        merge_fields(trace.fields_at(now_ms), provider_run_fields(run)),
        json!({
            "launch_started_at_ms": launch_started_at_ms,
            "binding_started_at_ms": binding_started_at_ms,
            "binding_duration_ms": elapsed_ms(binding_started_at_ms, now_ms),
            "launch_to_binding_complete_ms": elapsed_ms(launch_started_at_ms, now_ms),
            "status": status,
        }),
    );
    if let Some(error) = error {
        fields = merge_fields(fields, json!({ "error": error }));
    }
    let message = if status == "ok" {
        "provider runtime binding completed"
    } else {
        "provider runtime binding failed"
    };
    crate::logging::info_with_fields("daemon.provider_latency", message, fields);
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
    use std::collections::BTreeMap;

    use crate::local::{GetDaemonHealthRequest, LocalDaemonRequest};
    use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};
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

    #[test]
    fn first_output_fields_join_provider_run_to_active_turn_trace() {
        let request =
            LaunchProviderRequest::new("session-1", "dev-stub", "dev-stub", "default", "model-a")
                .with_agent_id("agent-1");
        let launch_result = ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "dev-stub".to_string(),
            pty_target: Some("pty-1".to_string()),
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        };
        let run = crate::provider::RuntimeProviderRun::new("run-1", &request, launch_result);
        let mut turn = ActiveTurnState::new(
            "session-1".to_string(),
            "agent-1".to_string(),
            "prompt-1".to_string(),
            "run-1".to_string(),
        )
        .with_trace_id("trace-1");
        turn.started_at_ms = 1_000;

        let fields = provider_first_output_fields(&run, Some(&turn), 1_050);

        assert_eq!(fields["trace_id"], "trace-1");
        assert_eq!(fields["provider_run_id"], "run-1");
        assert_eq!(fields["prompt_id"], "prompt-1");
        assert_eq!(fields["prompt_to_first_output_ms"], 50);
        assert_eq!(
            fields["provider_run_to_first_output_ms"],
            1_050_u64.saturating_sub(run.started_at_ms())
        );
    }

    #[test]
    fn completion_fields_include_prompt_tail_state() {
        let request =
            LaunchProviderRequest::new("session-1", "dev-stub", "dev-stub", "default", "model-a")
                .with_agent_id("agent-1");
        let launch_result = ProviderLaunchResult {
            endpoint_mode: AgentEndpointMode::Managed,
            process_label: "dev-stub".to_string(),
            pty_target: Some("pty-1".to_string()),
            pty_program: None,
            pty_args: Vec::new(),
            pty_env: BTreeMap::new(),
            pty_env_remove: Vec::new(),
            working_directory: None,
            structured_endpoint: None,
        };
        let run = crate::provider::RuntimeProviderRun::new("run-1", &request, launch_result);
        let mut turn = ActiveTurnState::new(
            "session-1".to_string(),
            "agent-1".to_string(),
            "prompt-1".to_string(),
            "run-1".to_string(),
        )
        .with_trace_id("trace-1");
        turn.started_at_ms = 2_000;
        let activity = ActivePromptState {
            last_output_at: None,
            saw_response_content: true,
            completion_recorded: true,
            settlement_requested: true,
            active_tool_ids: std::collections::BTreeSet::new(),
        };

        let fields = provider_turn_completion_fields(&run, Some(&turn), Some(&activity), 2_075);

        assert_eq!(fields["trace_id"], "trace-1");
        assert_eq!(fields["prompt_id"], "prompt-1");
        assert_eq!(fields["prompt_to_completion_ms"], 75);
        assert_eq!(fields["saw_response_content"], true);
        assert_eq!(fields["completion_recorded"], true);
        assert_eq!(fields["settlement_requested"], true);
    }
}
